use anyhow::Result;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::migration::providers::aws::database::{extract_parameter_blocks, is_valid_pg_property};
use crate::migration::types::{MigrationResult, MigrationStatus};
use crate::migration::var_detector::{analyze_variable, apply_conversion_to_hcl, build_var_annotation, build_var_usage_map, extract_variable_info, VarKind};
use crate::terraform::types::{PassthroughBlock, PassthroughKind};
use crate::zones::zone_to_objstorage_region;

/// Prefix that identifies an unresolved TODO placeholder in generated HCL.
/// Used to skip values that the mapper could not resolve at mapping time.
const TODO_PLACEHOLDER_PREFIX: &str = "<TODO";

/// Sentinel stored in `resolved_hcl_map` for resources that were intentionally
/// skipped during generation (e.g. firewall rules with no resolvable server_id).
/// The diff view uses this to show a "skipped" note instead of partial HCL.
pub const SKIPPED_SENTINEL: &str = "\x00SKIPPED";


/// Resolved HCL map: maps `(resource_type, resource_name)` to the fully-resolved HCL string
/// (zone injected, cross-references resolved, AWS refs sanitised).
pub type ResolvedHclMap = HashMap<(String, String), String>;

pub fn generate_files(
    results: &[MigrationResult],
    passthroughs: &[PassthroughBlock],
    output_dir: &Path,
    source_dir: Option<&Path>,
    zone: &str,
    log: &mut Vec<String>,
) -> Result<(usize, ResolvedHclMap)> {
    std::fs::create_dir_all(output_dir)?;

    let mut resolved_hcl_map: ResolvedHclMap = HashMap::new();

    let objstorage_region = zone_to_objstorage_region(zone);

    // ── Build cross-resolution lookup tables ──────────────────────────────────
    let lb_names: Vec<String> = results.iter()
        .filter(|r| r.upcloud_type == "upcloud_loadbalancer")
        .map(|r| r.resource_name.clone())
        .collect();

    let network_names: Vec<String> = results.iter()
        .filter(|r| r.upcloud_type.contains("upcloud_network"))
        .map(|r| r.resource_name.clone())
        .collect();

    let server_names: Vec<String> = results.iter()
        .filter(|r| r.upcloud_type == "upcloud_server")
        .map(|r| r.resource_name.clone())
        .collect();

    let k8s_names: Vec<String> = results.iter()
        .filter(|r| r.upcloud_type == "upcloud_kubernetes_cluster")
        .map(|r| r.resource_name.clone())
        .collect();

    // Build key_pair_name → public_key map for auto-resolving login blocks.
    let mut ssh_key_map: HashMap<String, LoginKeysValue> = HashMap::new();
    for r in results {
        if r.resource_type == "aws_key_pair" {
            if let Some(snippet) = &r.snippet {
                if let Some(keys) = extract_login_keys(snippet) {
                    ssh_key_map.insert(r.resource_name.clone(), keys);
                }
            }
        }
    }

    let backend_names: Vec<String> = results.iter()
        .filter(|r| r.upcloud_type == "upcloud_loadbalancer_backend")
        .map(|r| r.resource_name.clone())
        .collect();

    let cert_bundle_names: Vec<String> = results.iter()
        .filter(|r| r.upcloud_type == "upcloud_loadbalancer_manual_certificate_bundle")
        .map(|r| r.resource_name.clone())
        .collect();

    // Build server_info_map: server resource_name → Option<count string>
    // Used for name-based firewall server_id resolution and IP ref indexing.
    let server_info_map: HashMap<String, Option<String>> = results.iter()
        .filter(|r| r.upcloud_type == "upcloud_server")
        .map(|r| {
            let count = r.upcloud_hcl.as_deref().and_then(extract_count_from_hcl);
            (r.resource_name.clone(), count)
        })
        .collect();

    // Build sg_to_server_map: SG resource_name → Vec<server resource_name>.
    // A single SG can be attached to multiple servers (e.g. a "monitoring" SG applied to
    // both web and api instances). Each entry collects all servers that reference this SG,
    // so the generator can add the SG's rules to every server's firewall resource.
    let mut sg_to_server_map: HashMap<String, Vec<String>> = HashMap::new();
    for r in results.iter().filter(|r| r.resource_type == "aws_instance") {
        if let Some(hcl) = r.source_hcl.as_deref() {
            for sg_name in extract_sg_refs_from_instance_hcl(hcl) {
                sg_to_server_map.entry(sg_name).or_default().push(r.resource_name.clone());
            }
        }
    }

    // Build network_count_map: network resource_name → whether it has count.
    // Used to decide whether network references need [count.index] or [0].
    let network_count_map: HashMap<String, bool> = results.iter()
        .filter(|r| r.upcloud_type.contains("upcloud_network"))
        .map(|r| {
            let has_count = r.upcloud_hcl.as_deref().and_then(extract_count_from_hcl).is_some();
            (r.resource_name.clone(), has_count)
        })
        .collect();

    // Build param_group_map: parameter group resource_name → Vec<(param_name, value)>
    // Used to inject parameters inline into the properties {} block of DB resources.
    let param_group_map: HashMap<String, Vec<(String, String)>> = results.iter()
        .filter(|r| r.resource_type == "aws_db_parameter_group"
               || r.resource_type == "aws_elasticache_parameter_group")
        .filter_map(|r| r.source_hcl.as_ref().map(|hcl| {
            (r.resource_name.clone(), extract_parameter_blocks(hcl))
        }))
        .collect();

    // Build subnet_group_subnets_map: subnet group resource_name → Vec<subnet resource_name>.
    // Parsed from aws_db_subnet_group / aws_elasticache_subnet_group source HCL.
    // Lets the generator resolve "<TODO: upcloud_network UUID subnet_group=NAME>" to the
    // correct upcloud_network instead of always falling back to the first network.
    let subnet_group_subnets_map: HashMap<String, Vec<String>> = results.iter()
        .filter(|r| r.resource_type == "aws_db_subnet_group"
               || r.resource_type == "aws_elasticache_subnet_group")
        .filter_map(|r| r.source_hcl.as_deref().map(|hcl| {
            (r.resource_name.clone(), extract_subnet_names_from_subnet_group(hcl))
        }))
        .filter(|(_, subnets)| !subnets.is_empty())
        .collect();

    // Build lb_backend_net_map: lb resource_name → network name that its backends are on.
    // Chain: aws_lb_listener (lb→tg) → aws_lb_target_group_attachment (tg→server) →
    //        aws_instance (server→subnet) → upcloud_network (subnet→name).
    // This gives a deterministic answer instead of the name-based heuristic.
    let server_subnet_map: HashMap<String, String> = results.iter()
        .filter(|r| r.resource_type == "aws_instance")
        .filter_map(|r| r.source_hcl.as_deref().and_then(|hcl| {
            extract_subnet_id_from_instance_hcl(hcl)
                .map(|subnet| (r.resource_name.clone(), subnet))
        }))
        .collect();

    let mut tg_servers_map: HashMap<String, Vec<String>> = HashMap::new();
    for r in results.iter().filter(|r| r.resource_type == "aws_lb_target_group_attachment") {
        if let Some(hcl) = r.source_hcl.as_deref() {
            if let Some((tg, srv)) = extract_tg_server_from_attachment_source_hcl(hcl) {
                tg_servers_map.entry(tg).or_default().push(srv);
            }
        }
    }

    let mut lb_tgs_map: HashMap<String, Vec<String>> = HashMap::new();
    // lb_name comes from the `parent_resource` field set by the loadbalancer mapper on listener results
    for r in results.iter().filter(|r| r.resource_type == "aws_lb_listener") {
        if let Some(hcl) = r.source_hcl.as_deref() {
            if let Some(tg) = extract_tg_from_listener_source_hcl(hcl) {
                // The lb_name: prefer parent_resource, fall back to parsing load_balancer_arn
                let lb_name = r.parent_resource.clone().unwrap_or_default();
                let lb_name = if lb_name.is_empty() {
                    // parse from `load_balancer_arn = aws_lb.NAME.arn`
                    hcl.lines()
                        .find(|l| l.trim().starts_with("load_balancer_arn"))
                        .and_then(|l| l.find("aws_lb.").map(|p| &l[p + "aws_lb.".len()..]))
                        .and_then(|s| s.split('.').next())
                        .map(|s| s.trim_matches('"').to_string())
                        .unwrap_or_default()
                } else {
                    lb_name
                };
                if !lb_name.is_empty() {
                    lb_tgs_map.entry(lb_name).or_default().push(tg);
                }
            }
        }
    }

    // Now chain lb → tgs → servers → subnets → network names
    let mut lb_backend_net_map: HashMap<String, String> = HashMap::new();
    for (lb_name, tgs) in &lb_tgs_map {
        'outer: for tg in tgs {
            if let Some(servers) = tg_servers_map.get(tg) {
                for srv in servers {
                    if let Some(subnet) = server_subnet_map.get(srv) {
                        // subnet name must correspond to a known upcloud_network resource
                        if network_names.contains(subnet) {
                            lb_backend_net_map.insert(lb_name.clone(), subnet.clone());
                            break 'outer;
                        }
                    }
                }
            }
        }
    }

    // Build storage_count_map and storage_inject_map:
    // For aws_volume_attachment resources, inject storage_devices blocks directly
    // into the matching server's HCL (via the __STORAGE_END_<name>__ sentinel).
    let storage_count_map: HashMap<String, bool> = results.iter()
        .filter(|r| r.upcloud_type == "upcloud_storage")
        .map(|r| {
            let has_count = r.upcloud_hcl.as_deref().and_then(extract_count_from_hcl).is_some();
            (r.resource_name.clone(), has_count)
        })
        .collect();

    // storage_inject_map: server_resource_name → Vec<storage_devices block>
    let mut storage_inject_map: HashMap<String, Vec<String>> = HashMap::new();
    for r in results.iter().filter(|r| r.resource_type == "aws_volume_attachment") {
        let server_name = match r.parent_resource.as_deref() {
            Some(n) if !n.is_empty() => n.to_string(),
            _ => continue,
        };
        // Parse the storage resource name from the snippet line: "storage = upcloud_storage.NAME.id"
        let storage_name = r.snippet.as_deref().and_then(|s| {
            s.lines()
                .find(|l| l.trim().starts_with("storage = upcloud_storage."))
                .and_then(|l| l.trim().strip_prefix("storage = upcloud_storage."))
                .and_then(|s| s.split('.').next())
                .map(str::to_string)
        });
        let storage_name = match storage_name {
            Some(n) => n,
            None => continue,
        };
        let server_has_count = server_info_map.get(&server_name).map(|c| c.is_some()).unwrap_or(false);
        let storage_has_count = storage_count_map.get(&storage_name).copied().unwrap_or(false);
        let storage_ref = if storage_has_count && server_has_count {
            format!("upcloud_storage.{}[count.index].id", storage_name)
        } else if storage_has_count {
            format!("upcloud_storage.{}[0].id", storage_name)
        } else {
            format!("upcloud_storage.{}.id", storage_name)
        };
        let block = format!(
            "  storage_devices {{\n    storage = {}\n    type    = \"disk\"\n  }}\n",
            storage_ref
        );
        storage_inject_map.entry(server_name).or_default().push(block);
    }

    // Apply zone substitution and cross-resolve TODO placeholders
    let resolve = |hcl: &str, resource_name: &str| -> String {
        let mut s = hcl
            .replace("__ZONE__", zone)
            .replace("__OBJSTORAGE_REGION__", objstorage_region);

        // LB references
        if let Some(lb) = lb_names.first() {
            s = s.replace("upcloud_loadbalancer.<TODO>.id", &format!("upcloud_loadbalancer.{}.id", lb));
        }

        // Backend name for frontends
        if let Some(backend) = backend_names.first() {
            s = s.replace(
                "upcloud_loadbalancer_backend.<TODO>.name",
                &format!("upcloud_loadbalancer_backend.{}.name", backend),
            );
        }

        // Certificate bundle for HTTPS frontends
        if let Some(cert) = cert_bundle_names.first() {
            s = s.replace(
                "upcloud_loadbalancer_manual_certificate_bundle.<TODO>.id",
                &format!("upcloud_loadbalancer_manual_certificate_bundle.{}.id", cert),
            );
        }

        // Network reference for LB networks blocks and server network_interface.
        // When the network is a counted resource, the reference must include an index:
        //   - counted server context  → [count.index]
        //   - non-counted server      → [0]

        // First, resolve subnet-specific placeholders generated by map_instance from
        // the source instance's subnet_id attribute: "<TODO: upcloud_network.NAME reference>".
        // Each server can reference a different subnet/network this way.
        for net_name in &network_names {
            let specific = format!("\"<TODO: upcloud_network.{} reference>\"", net_name);
            if s.contains(&specific) {
                let net_has_count = network_count_map.get(net_name.as_str()).copied().unwrap_or(false);
                let net_ref = if net_has_count {
                    if extract_count_from_hcl(&s).is_some() {
                        format!("upcloud_network.{}[count.index].id", net_name)
                    } else {
                        format!("upcloud_network.{}[0].id", net_name)
                    }
                } else {
                    format!("upcloud_network.{}.id", net_name)
                };
                s = s.replace(&specific, &net_ref);
            }
        }

        // Subnet-group-specific resolution for databases.
        // Replaces "<TODO: upcloud_network UUID subnet_group=NAME>" with the correct
        // upcloud_network derived from the aws_db_subnet_group / aws_elasticache_subnet_group.
        for (sg_name, subnets) in &subnet_group_subnets_map {
            let placeholder = format!("\"<TODO: upcloud_network UUID subnet_group={}>\"", sg_name);
            if s.contains(&placeholder) {
                let net = subnets.iter()
                    .find(|sn| network_names.contains(sn))
                    .or_else(|| network_names.first());
                if let Some(net_name) = net {
                    // If the resolved network has count, must index it (e.g. database[0])
                    let net_has_count = network_count_map.get(net_name.as_str()).copied().unwrap_or(false);
                    let net_ref = if net_has_count {
                        format!("upcloud_network.{}[0].id", net_name)
                    } else {
                        format!("upcloud_network.{}.id", net_name)
                    };
                    s = s.replace(&placeholder, &net_ref);
                }
            }
        }

        // Generic fallback: resolve "<TODO: upcloud_network reference>" to the best
        // available network.
        //
        // For load balancers, use the deterministic chain:
        //   aws_lb → aws_lb_listener (tg) → aws_lb_target_group_attachment (server) →
        //   aws_instance.subnet_id → upcloud_network
        // This is exact — no heuristic needed.
        //
        // For everything else, prefer (in order):
        //   1. a network whose name contains "private"
        //   2. any network whose name does NOT contain "public"
        //   3. whatever is first (last resort)
        let preferred_net = lb_backend_net_map
            .get(resource_name)
            .and_then(|net| network_names.iter().find(|n| *n == net))
            .or_else(|| network_names.iter().find(|n| n.to_lowercase().contains("private")))
            .or_else(|| network_names.iter().find(|n| !n.to_lowercase().contains("public")))
            .or_else(|| network_names.first());
        if let Some(net) = preferred_net {
            let net_has_count = network_count_map.get(net.as_str()).copied().unwrap_or(false);
            let net_ref = if net_has_count {
                if extract_count_from_hcl(&s).is_some() {
                    format!("upcloud_network.{}[count.index].id", net)
                } else {
                    format!("upcloud_network.{}[0].id", net)
                }
            } else {
                format!("upcloud_network.{}.id", net)
            };
            s = s.replace("\"<TODO: upcloud_network reference>\"", &net_ref);
            // Legacy placeholders kept for backwards compat
            s = s.replace("\"<TODO: reference upcloud_network UUID>\"", &net_ref);
            s = s.replace("\"<TODO: upcloud_network UUID>\"", &net_ref);
        }

        // Server IP for LB backend members.
        // When the server is a counted resource, the reference must include [0].
        if let Some(srv) = server_names.first() {
            let srv_has_count = server_info_map.get(srv.as_str()).map(|c| c.is_some()).unwrap_or(false);
            let srv_ref = if srv_has_count {
                format!("upcloud_server.{}[0].network_interface[0].ip_address", srv)
            } else {
                format!("upcloud_server.{}.network_interface[0].ip_address", srv)
            };
            s = s.replace("\"<TODO: server IP>\"", &srv_ref);
        }

        // K8s cluster ref for node groups
        if let Some(k8s) = k8s_names.first() {
            s = s.replace(
                "upcloud_kubernetes_cluster.<TODO>.id",
                &format!("upcloud_kubernetes_cluster.{}.id", k8s),
            );
        }

        // Resolve login block SSH keys: replace TODO placeholders with actual public keys.
        let mut resolved = s.clone();
        for (kp_name, keys_value) in &ssh_key_map {
            let placeholder = format!("<TODO: SSH public key for aws_key_pair.{}>", kp_name);
            match keys_value {
                // Literal key: replace just the TODO content; surrounding quotes stay.
                LoginKeysValue::Literal(public_key) => {
                    resolved = resolved.replace(&placeholder, public_key);
                }
                // Expression key (ternary etc.): replace the quoted placeholder INCLUDING
                // its surrounding quotes so the expression is a bare HCL value, not a string.
                LoginKeysValue::Expression(expr) => {
                    let quoted_placeholder = format!("\"{}\"", placeholder);
                    resolved = resolved.replace(&quoted_placeholder, expr);
                }
            }
        }
        // Generic SSH key TODO fallback — use the first available literal key.
        if resolved.contains("<TODO: paste SSH public key>") {
            if let Some(LoginKeysValue::Literal(key)) = ssh_key_map.values().next() {
                resolved = resolved.replace("<TODO: paste SSH public key>", key);
            }
        }
        s = resolved;

        // Resolve DB parameter group property markers: # __DB_PROPS:PREFIX:GROUP_NAME__
        // Each marker line is replaced with the actual property lines from the param group.
        if s.contains("# __DB_PROPS:") {
            let mut out = String::with_capacity(s.len());
            for line in s.lines() {
                let trimmed = line.trim();
                if let Some(inner) = trimmed.strip_prefix("# __DB_PROPS:") {
                    if let Some(inner) = inner.strip_suffix("__") {
                        if let Some((prefix, group_name)) = inner.split_once(':') {
                            if let Some(params) = param_group_map.get(group_name) {
                                if !params.is_empty() {
                                    let _ = prefix; // prefix not prepended; UpCloud property names are unprefixed
                                    for (name, value) in params {
                                        if is_valid_pg_property(name) {
                                            out.push_str(&format!("    {} = \"{}\"\n", name, value));
                                        } else {
                                            out.push_str(&format!(
                                                "    # <TODO: {} = \"{}\" — not a valid upcloud_managed_database_postgresql property>\n",
                                                name, value
                                            ));
                                        }
                                    }
                                    continue; // marker replaced — skip appending the marker line
                                }
                            }
                            // Group not found or empty — leave a TODO comment
                            out.push_str(&format!(
                                "    # <TODO: migrate parameters from aws_db_parameter_group.{}>\n",
                                group_name
                            ));
                            continue;
                        }
                    }
                }
                out.push_str(line);
                out.push('\n');
            }
            // Preserve original trailing-newline behaviour
            if !s.ends_with('\n') && out.ends_with('\n') {
                out.pop();
            }
            s = out;
        }

        // Replace any remaining AWS-specific data source references
        s = sanitize_aws_refs(s);

        s
    };

    // Group fully-convertible results by source file basename
    let mut file_map: HashMap<String, Vec<&MigrationResult>> = HashMap::new();
    for result in results {
        if result.upcloud_hcl.is_some() {
            let basename = PathBuf::from(&result.source_file)
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("output.tf")
                .to_string();
            file_map.entry(basename).or_default().push(result);
        }
    }

    // Group passthrough blocks (variable / output / locals) by source file basename.
    // Files that contain only passthrough blocks (no resources) must also be created.
    let mut passthrough_map: HashMap<String, Vec<&PassthroughBlock>> = HashMap::new();
    for pt in passthroughs {
        let basename = pt
            .source_file
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("variables.tf")
            .to_string();
        // Make sure files with only passthroughs (no resources) still appear in the output.
        file_map.entry(basename.clone()).or_default();
        passthrough_map.entry(basename).or_default().push(pt);
    }

    // Detect whether ssh_public_key variable needs to be synthesized.
    // This happens when any server uses var.ssh_public_key (key_name was a variable reference)
    // but the source code doesn't already declare that variable.
    let needs_ssh_public_key = results.iter()
        .any(|r| r.upcloud_hcl.as_deref().unwrap_or("").contains("var.ssh_public_key"))
        && !passthroughs.iter()
            .any(|p| p.kind == PassthroughKind::Variable && p.name.as_deref() == Some("ssh_public_key"));
    let ssh_var_target_file: String = if needs_ssh_public_key {
        passthrough_map.iter()
            .filter(|(_, pts)| pts.iter().any(|p| p.kind == PassthroughKind::Variable))
            .map(|(name, _)| name.clone())
            .next()
            .unwrap_or_else(|| "variables.tf".to_string())
    } else {
        String::new()
    };
    if needs_ssh_public_key && !ssh_var_target_file.is_empty() {
        file_map.entry(ssh_var_target_file.clone()).or_default();
    }

    // Build a usage map so the variable detector can score each variable by where it is referenced.
    // Gather all source HCL from mapped resources (best-effort — not every resource has source_hcl).
    let source_hcl_refs: Vec<&str> = results
        .iter()
        .filter_map(|r| r.source_hcl.as_deref())
        .collect();
    let var_usage_map = build_var_usage_map(&source_hcl_refs);

    // Write provider config
    let provider_path = output_dir.join("providers.tf");
    let provider_hcl = r#"terraform {
  required_providers {
    upcloud = {
      source  = "UpCloudLtd/upcloud"
      version = "~> 5.0"
    }
  }
}

variable "upcloud_token" {
  description = "UpCloud API token"
  type        = string
  sensitive   = true
}

provider "upcloud" {
  token = var.upcloud_token
}
"#;
    std::fs::write(&provider_path, provider_hcl)?;
    log.push(format!("  [OK] providers.tf"));

    let mut total = 1;

    // Track servers that have already had their firewall rules written (across all files).
    // UpCloud allows only one upcloud_firewall_rules resource per server.
    let mut written_fw_servers: std::collections::HashSet<String> = std::collections::HashSet::new();

    for (filename, file_results) in &file_map {
        let out_path = output_dir.join(filename);
        let mut content = String::new();
        content.push_str(&format!(
            "# Migrated from AWS Terraform\n# Source: {}\n# Target zone: {}\n\n",
            filename, zone
        ));

        // Collect per-file pending firewall rules: server_id_expr → (resource_name, count_line, rule_blocks, notes)
        // Multiple SGs targeting the same server are merged into a single upcloud_firewall_rules resource.
        let mut fw_by_server: indexmap::IndexMap<String, (String, Option<String>, Vec<String>, Vec<String>)> =
            indexmap::IndexMap::new();

        for result in file_results {
            if let Some(hcl) = &result.upcloud_hcl {
                let resolved = resolve(hcl, &result.resource_name);

                // Name-based firewall server_id resolution.
                // An SG can be attached to multiple servers (e.g. a "monitoring" SG on both
                // web and api), so we iterate over every server that references this SG and
                // add the rules to each server's merged firewall resource.
                if result.upcloud_type == "upcloud_firewall_rules" {
                    // Collect all servers that reference this SG (via vpc_security_group_ids).
                    // Fall back to name-match: try the SG's own resource name as a server name.
                    let servers: Vec<String> = sg_to_server_map
                        .get(&result.resource_name)
                        .cloned()
                        .unwrap_or_else(|| vec![result.resource_name.clone()]);

                    let mut any_resolved = false;
                    let mut first_resolved_hcl: Option<String> = None;

                    for effective_server in &servers {
                        let resolved_for_server = resolve_firewall_server(
                            &resolved,
                            effective_server,
                            &server_info_map,
                        );

                        // If still unresolved for this server, skip it.
                        if resolved_for_server.contains("upcloud_server.<TODO>.id") {
                            continue;
                        }

                        any_resolved = true;
                        if first_resolved_hcl.is_none() {
                            first_resolved_hcl = Some(resolved_for_server.clone());
                        }

                        // Collect rules into the per-server merge map.
                        if let Some(server_id_expr) = extract_fw_server_id_expr(&resolved_for_server) {
                            let rule_blocks = extract_fw_rule_blocks(&resolved_for_server);
                            let count_line = extract_fw_count_line(&resolved_for_server);
                            let entry = fw_by_server
                                .entry(server_id_expr)
                                .or_insert_with(|| (result.resource_name.clone(), count_line, Vec::new(), Vec::new()));
                            entry.2.extend(rule_blocks);
                            // Tag each note with its source SG so the merged resource is traceable.
                            entry.3.extend(result.notes.iter().map(|n| format!("[{}] {}", result.resource_name, n)));
                        }
                    }

                    // Store resolved HCL for the diff view (first resolved server's HCL, or SKIPPED).
                    let diff_hcl = first_resolved_hcl.unwrap_or_else(|| resolved.clone());
                    resolved_hcl_map.insert(
                        (result.resource_type.clone(), result.resource_name.clone()),
                        if any_resolved { diff_hcl } else { SKIPPED_SENTINEL.to_string() },
                    );

                    if !any_resolved {
                        log.push(format!("  [SKIP] upcloud_firewall_rules.{}: if this SG guards a database, cache, or LB, UpCloud manages their access control separately", result.resource_name));
                    }
                    continue; // deferred — written after the resource loop below
                }

                // Store the resolved HCL for the diff view.
                resolved_hcl_map.insert(
                    (result.resource_type.clone(), result.resource_name.clone()),
                    resolved.clone(),
                );

                content.push_str(&format!(
                    "# {} {}\n",
                    result.resource_type, result.resource_name
                ));
                for note in &result.notes {
                    content.push_str(&format!("# NOTE: {}\n", note));
                }
                content.push_str(&resolved);
                content.push('\n');
            }
        }

        // Write merged firewall rules: one upcloud_firewall_rules per server.
        for (server_id_expr, (resource_name, count_line, rule_blocks, notes)) in &fw_by_server {
            let server_name = server_name_from_server_id_expr(server_id_expr);
            if written_fw_servers.contains(&server_name) {
                // Another file already wrote firewall rules for this server.
                // UpCloud allows only one upcloud_firewall_rules per server — warn the user.
                log.push(format!(
                    "  [WARN] upcloud_firewall_rules for '{}' already written by another file — merge manually",
                    server_name
                ));
                content.push_str(&format!(
                    "# WARNING: firewall rules for server '{}' were already written by another output file.\n\
                     # Merge the rules below into that resource manually (UpCloud requires exactly one\n\
                     # upcloud_firewall_rules per server).\n",
                    server_name
                ));
                // Still emit the rules as comments so the user can copy them.
                let commented: String = build_merged_fw_hcl(resource_name, server_id_expr, count_line.as_deref(), rule_blocks)
                    .lines()
                    .map(|l| format!("# {}\n", l))
                    .collect();
                content.push_str(&commented);
                content.push('\n');
                continue;
            }
            written_fw_servers.insert(server_name);

            // Count how many distinct SGs contributed (notes are tagged "[sg_name] ...").
            let sg_count = notes.iter()
                .filter_map(|n| n.strip_prefix('[').and_then(|s| s.split(']').next()))
                .collect::<std::collections::HashSet<_>>()
                .len();
            if sg_count > 1 {
                log.push(format!(
                    "  [MERGE] {} security groups → upcloud_firewall_rules.{}",
                    sg_count, resource_name
                ));
            }

            for note in notes {
                content.push_str(&format!("# NOTE: {}\n", note));
            }
            if rule_blocks.is_empty() {
                content.push_str(&format!(
                    "# NOTE: [{}] No SG rules resolved — add firewall_rule blocks manually.\n",
                    resource_name
                ));
            }
            let merged_hcl = build_merged_fw_hcl(resource_name, server_id_expr, count_line.as_deref(), rule_blocks);
            content.push_str(&merged_hcl);
            content.push('\n');
        }

        // Append passthrough blocks (variable / output / locals) for this file.
        if let Some(pts) = passthrough_map.get(filename) {
            for pt in pts {
                let aws_name = pt
                    .name
                    .as_deref()
                    .map(|n| n.starts_with("aws_") || n == "aws")
                    .unwrap_or(false);
                if aws_name {
                    // Do NOT inject <TODO:> into the variable name — that breaks HCL.
                    // Instead, add a plain comment so the user knows to review it.
                    content.push_str(&format!(
                        "# NOTE: Variable name '{}' references AWS — consider renaming \
                         (e.g. \"{}\").\n",
                        pt.name.as_deref().unwrap_or(""),
                        pt.name
                            .as_deref()
                            .unwrap_or("")
                            .trim_start_matches("aws_"),
                    ));
                }
                // Rewrite AWS resource references inside output/locals blocks.
                // Variables: run multi-signal detection and auto-convert instance types / regions.
                let hcl = if pt.kind == PassthroughKind::Output || pt.kind == PassthroughKind::Locals {
                    rewrite_output_refs(&pt.raw_hcl)
                } else if pt.kind == PassthroughKind::Variable {
                    let var_name = pt.name.as_deref().unwrap_or("");
                    let (default_val, description) = extract_variable_info(&pt.raw_hcl);
                    let usage_attrs = var_usage_map
                        .get(var_name)
                        .cloned()
                        .unwrap_or_default();
                    let rewritten = if let Some(mut conv) = analyze_variable(
                        var_name,
                        default_val.as_deref(),
                        description.as_deref(),
                        &usage_attrs,
                    ) {
                        // For region variables, always use the zone the user selected in the app
                        // rather than the geographically closest zone from the detector.
                        if conv.kind == VarKind::Region && conv.converted_value.is_some() {
                            conv.converted_value = Some(zone.to_string());
                        }
                        let annotation = build_var_annotation(var_name, &conv);
                        let converted_hcl = apply_conversion_to_hcl(&pt.raw_hcl, &conv);
                        format!("{}{}", annotation, converted_hcl)
                    } else {
                        pt.raw_hcl.clone()
                    };
                    rewritten
                } else {
                    pt.raw_hcl.clone()
                };
                content.push_str(&hcl);
                content.push('\n');
                content.push('\n');
            }
        }

        // Synthesize ssh_public_key variable if it is needed and this is the target file.
        if needs_ssh_public_key && filename == &ssh_var_target_file {
            content.push_str(concat!(
                "variable \"ssh_public_key\" {\n",
                "  description = \"SSH public key for server access (replaces AWS key_name)\"\n",
                "  type        = string\n",
                "}\n\n",
            ));
            log.push("  [SYNTH] variable \"ssh_public_key\" added to variables.tf".to_string());
        }

        // Inject storage_devices blocks into servers that have volume attachments.
        // The compute mapper leaves a sentinel comment inside each server block:
        //   # __STORAGE_END_<server_name>__
        // We replace it with the actual storage_devices block(s).
        for (server_name, blocks) in &storage_inject_map {
            let sentinel = format!("  # __STORAGE_END_{}__\n", server_name);
            if content.contains(&sentinel) {
                let injection: String = blocks.join("");
                content = content.replace(&sentinel, &injection);
                log.push(format!("  [INJECT] storage_devices → upcloud_server.{}", server_name));
            }
        }
        // Remove any uninjected sentinels (servers with no attachments)
        let content_cleaned: String = content.lines()
            .filter(|l| !l.trim_start().starts_with("# __STORAGE_END_"))
            .map(|l| { let mut s = l.to_string(); s.push('\n'); s })
            .collect();
        content = content_cleaned;

        match std::fs::write(&out_path, &content) {
            Ok(_) => {
                log.push(format!("  [OK] {}", filename));
                // Only validate HCL when there are no TODO placeholders — TODOs intentionally
                // produce invalid HCL (e.g. `= upcloud_server.web.<TODO: ...>`), so any parse
                // error there is expected and not worth surfacing to the user.
                if !content.contains(TODO_PLACEHOLDER_PREFIX) {
                    if let Err(e) = hcl::from_str::<hcl::Body>(&content) {
                        log.push(format!("  [HCL ERR] {} — {}", filename, e));
                    }
                }
                total += 1;
            }
            Err(e) => {
                log.push(format!("  [ERR] {}: {}", filename, e));
            }
        }
    }

    // Write MIGRATION_NOTES.md covering partial and unsupported resources
    let partial: Vec<&MigrationResult> = results
        .iter()
        .filter(|r| {
            r.status == MigrationStatus::Partial
                && (r.snippet.is_some() || !r.notes.is_empty())
                // Volume attachments with a resolved parent_resource were auto-injected into
                // the server's storage_devices block — no manual action needed.
                && !(r.resource_type == "aws_volume_attachment" && r.parent_resource.is_some())
        })
        .collect();

    let unsupported: Vec<&MigrationResult> = results
        .iter()
        .filter(|r| r.status == MigrationStatus::Unsupported || r.status == MigrationStatus::Unknown)
        .collect();

    if !partial.is_empty() || !unsupported.is_empty() {
        let notes_path = output_dir.join("MIGRATION_NOTES.md");
        let mut notes = String::from("# Migration Notes\n\n");
        notes.push_str(&format!("Target zone: **{}**  \n", zone));
        notes.push_str(&format!("Object storage region: **{}**\n\n", objstorage_region));

        if !partial.is_empty() {
            notes.push_str("## Partial Resources — Manual Action Required\n\n");
            notes.push_str("These resources have no standalone UpCloud equivalent. \
                The code snippets below must be merged into the appropriate resource blocks.\n\n");

            for r in &partial {
                notes.push_str(&format!(
                    "### `{}` **{}**  *({})*\n\n",
                    r.resource_type, r.resource_name, r.source_file
                ));
                for note in &r.notes {
                    notes.push_str(&format!("{}\n\n", note));
                }
                if let Some(snippet) = &r.snippet {
                    notes.push_str("```hcl\n");
                    notes.push_str(snippet);
                    notes.push_str("\n```\n\n");
                }
            }
        }

        if !unsupported.is_empty() {
            notes.push_str("## Unsupported Resources — No UpCloud Equivalent\n\n");
            notes.push_str("These resources require fully manual migration:\n\n");
            for r in &unsupported {
                notes.push_str(&format!(
                    "- **{}** `{}` *({})*\n",
                    r.resource_type, r.resource_name, r.source_file
                ));
                for note in &r.notes {
                    notes.push_str(&format!("  - {}\n", note));
                }
            }
            notes.push('\n');
        }

        std::fs::write(&notes_path, &notes)?;
        log.push(format!(
            "  [OK] MIGRATION_NOTES.md ({} partial, {} unsupported)",
            partial.len(),
            unsupported.len()
        ));
        total += 1;
    }

    // ── Copy non-.tf files (scripts, JSON, YAML, etc.) from source dir ────────
    if let Some(src) = source_dir {
        for entry in walkdir::WalkDir::new(src).follow_links(true) {
            let entry = match entry {
                Ok(e) => e,
                Err(_) => continue,
            };
            if !entry.file_type().is_file() {
                continue;
            }
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("tf") {
                continue; // already handled via HCL generation
            }
            let rel = match path.strip_prefix(src) {
                Ok(r) => r,
                Err(_) => continue,
            };
            let dest = output_dir.join(rel);
            if let Some(parent) = dest.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::copy(path, &dest)?;
            log.push(format!("  [COPY] {}", rel.display()));
            total += 1;
        }
    }

    Ok((total, resolved_hcl_map))
}

/// The two forms a `keys = [...]` value can take in a generated login block.
enum LoginKeysValue {
    /// A literal public-key string, e.g. `"ssh-rsa AAAA..."`.
    /// The surrounding double-quotes in the HCL are kept; only the TODO content
    /// inside them is replaced.
    Literal(String),
    /// An HCL expression, e.g. `var.x != "" ? var.x : "fallback"`.
    /// The entire quoted TODO placeholder (including its surrounding `"`) is
    /// replaced with the bare expression.
    Expression(String),
}

/// Extract the SSH public key value from a `login { keys = [...] }` snippet
/// by parsing the snippet as HCL and inspecting the AST.
///
/// Returns `None` when the snippet cannot be parsed, has no `login`/`keys`,
/// or the key is still an unresolved `<TODO` placeholder.
///
/// # Why two variants?
///
/// `hcl::Expression` implements `Display` (via `hcl::format::to_string`), so
/// non-literal expressions can be serialized back to HCL source text and
/// embedded verbatim in the generated output.  The two variants differ in how
/// the surrounding double-quotes in the HCL template are handled:
///
/// - `Literal` — the template already contains `keys = ["..."]`; only the
///   inner value is swapped in, leaving the quotes intact.
/// - `Expression` — the entire `"<TODO: ...>"` placeholder (quotes included)
///   is replaced with the bare expression so the result is valid HCL.
///
/// # Examples
///
/// ```text
/// // Input snippet — literal key
/// login {
///   keys = ["ssh-rsa AAAA..."]
/// }
/// // → LoginKeysValue::Literal("ssh-rsa AAAA...")
///
/// // Input snippet — ternary expression
/// login {
///   keys = [var.key != "" ? var.key : "fallback"]
/// }
/// // → LoginKeysValue::Expression(r#"var.key != "" ? var.key : "fallback""#)
/// ```
fn extract_login_keys(snippet: &str) -> Option<LoginKeysValue> {
    let body: hcl::Body = hcl::from_str(snippet).ok()?;

    let login_body = body
        .blocks()
        .find(|b| b.identifier() == "login")?
        .body()
        .clone();

    let keys_expr = login_body
        .attributes()
        .find(|a| a.key() == "keys")?
        .expr()
        .clone();

    // keys = [...] is always an Array expression with one element.
    let hcl::Expression::Array(elements) = keys_expr else {
        return None;
    };
    let first = elements.into_iter().next()?;

    // Reject unresolved TODO placeholders left by the mapper.
    if first.to_string().contains(TODO_PLACEHOLDER_PREFIX) {
        return None;
    }

    Some(match first {
        // Plain string literal: the HCL template already has surrounding quotes,
        // so we only need the inner value.
        hcl::Expression::String(s) => LoginKeysValue::Literal(s),
        // Any other expression (conditional, traversal, operation, …): serialize
        // back to HCL source text via Expression's Display impl and strip the
        // surrounding quotes from the template placeholder at resolution time.
        other => LoginKeysValue::Expression(other.to_string()),
    })
}

/// Replace AWS-specific data source references that leaked into output HCL.
/// Handles patterns like `data.aws_caller_identity.current.account_id`.
/// Also replaces cross-resource AWS refs like `aws_security_group.main.id`.
/// Skips any `aws_` occurrences that are already inside a `<TODO: ...>` marker.
fn sanitize_aws_refs(mut s: String) -> String {
    // data.aws_* data source references
    let mut search_from = 0;
    loop {
        let Some(rel) = s[search_from..].find("data.aws_") else { break };
        let start = search_from + rel;
        if inside_todo_marker(&s, start) {
            search_from = start + "data.aws_".len();
            continue;
        }
        let end = s[start..]
            .find(|c: char| !c.is_alphanumeric() && c != '.' && c != '_')
            .map(|off| start + off)
            .unwrap_or(s.len());
        let aws_ref = s[start..end].to_string();
        s = s.replacen(&aws_ref, "<TODO: remove AWS data source ref>", 1);
        search_from = 0; // restart since string changed
    }
    // aws_type.name.attr resource references inside interpolations ${...}
    // These have the form "aws_*.*.*" with at least one dot
    let mut search_from = 0;
    while let Some(rel) = s[search_from..].find("aws_") {
        let start = search_from + rel;
        if inside_todo_marker(&s, start) {
            search_from = start + "aws_".len();
            continue;
        }
        let end = s[start..]
            .find(|c: char| !c.is_alphanumeric() && c != '.' && c != '_')
            .map(|off| start + off)
            .unwrap_or(s.len());
        let candidate = &s[start..end];
        if candidate.matches('.').count() >= 1 {
            let owned = candidate.to_string();
            s = s.replacen(&owned, "<TODO: remove AWS resource ref>", 1);
            // reset search since string changed
            search_from = 0;
        } else {
            // no dots = likely a comment or type name; skip past it
            search_from = end;
        }
    }
    // Clean up any "${<TODO: ...>...}" that were created by replacing an AWS ref
    // that was inside a Terraform template expression (e.g. in a user_data heredoc).
    // "${<invalid>}" is not valid HCL; strip the "${...}" wrapper so the TODO
    // becomes plain text in the heredoc (which IS valid HCL).
    s = remove_todo_interpolations(s);

    s
}

/// Replace every `${<TODO: ...>...}` template interpolation with the plain text
/// `<TODO: remove AWS resource ref>`.
///
/// These arise when `sanitize_aws_refs` replaces an AWS traversal that was
/// embedded inside a `${...}` in a heredoc user_data block.  Leaving the
/// `${...}` wrapper makes the HCL invalid; stripping it produces a literal
/// string in the heredoc that `terraform validate` (and hcl-rs) can parse.
fn remove_todo_interpolations(mut s: String) -> String {
    loop {
        // Find "${" (with optional spaces) immediately followed by "<TODO:"
        let start = {
            let bytes = s.as_bytes();
            let mut found = None;
            let mut i = 0;
            while i + 1 < bytes.len() {
                if bytes[i] == b'$' && bytes[i + 1] == b'{' {
                    let mut j = i + 2;
                    while j < bytes.len() && bytes[j] == b' ' {
                        j += 1;
                    }
                    if s[j..].starts_with("<TODO:") {
                        found = Some(i);
                        break;
                    }
                }
                i += 1;
            }
            found
        };
        let Some(start) = start else { break };

        // Find the matching closing '}', tracking brace depth.
        let inner_start = start + 2;
        let mut depth = 1usize;
        let mut end = s.len(); // fallback: consume to end
        for (idx, c) in s[inner_start..].char_indices() {
            match c {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        end = inner_start + idx + c.len_utf8();
                        break;
                    }
                }
                _ => {}
            }
        }
        s.replace_range(start..end, "<TODO: remove AWS resource ref>");
    }
    s
}

/// Returns true if byte position `pos` in `s` is inside an open `<TODO: ...>` marker,
/// i.e. there is a `<TODO` before `pos` with no closing `>` between them.
fn inside_todo_marker(s: &str, pos: usize) -> bool {
    if let Some(last_open) = s[..pos].rfind(TODO_PLACEHOLDER_PREFIX) {
        !s[last_open..pos].contains('>')
    } else {
        false
    }
}

/// Map an AWS resource type name to its UpCloud equivalent.
/// Some AWS resource types get a suffix appended to their name in the UpCloud mapping.
/// E.g. aws_vpc "main" → upcloud_router "main_router".
fn upcloud_resource_name_for(aws_type: &str, resource_name: &str) -> String {
    match aws_type {
        "aws_vpc" => format!("{}_router", resource_name),
        _ => resource_name.to_string(),
    }
}

fn upcloud_type_for_aws(aws_type: &str) -> Option<&'static str> {
    match aws_type {
        "aws_instance" | "aws_launch_template" | "aws_launch_configuration"
            => Some("upcloud_server"),
        "aws_lb" | "aws_alb" | "aws_elb"
            => Some("upcloud_loadbalancer"),
        "aws_vpc"
            => Some("upcloud_router"),
        "aws_subnet"
            => Some("upcloud_network"),
        "aws_security_group"
            => Some("upcloud_firewall_rules"),
        "aws_db_instance" | "aws_rds_cluster" | "aws_rds_cluster_instance"
            => Some("upcloud_managed_database_postgresql"),
        "aws_elasticache_cluster" | "aws_elasticache_replication_group"
            => Some("upcloud_managed_database_valkey"),
        "aws_eks_cluster"
            => Some("upcloud_kubernetes_cluster"),
        "aws_eip"
            => Some("upcloud_floating_ip_address"),
        _ => None,
    }
}

/// Map an AWS attribute name to its UpCloud equivalent for a given UpCloud type.
/// Returns `None` for attributes with no direct equivalent (caller injects a TODO).
fn upcloud_attr_for(upcloud_type: &str, aws_attr: &str) -> Option<&'static str> {
    match (upcloud_type, aws_attr) {
        // upcloud_server
        ("upcloud_server", "id")         => Some("id"),
        ("upcloud_server", "public_ip")  => Some("network_interface[0].ip_address"),
        ("upcloud_server", "private_ip") => Some("network_interface[1].ip_address"),
        // upcloud_loadbalancer
        ("upcloud_loadbalancer", "id")       => Some("id"),
        ("upcloud_loadbalancer", "dns_name") => Some("dns_name"),
        // upcloud_router
        ("upcloud_router", "id") => Some("id"),
        // upcloud_network
        ("upcloud_network", "id")         => Some("id"),
        ("upcloud_network", "cidr_block") => Some("ip_network[0].address"),
        // upcloud_firewall_rules
        ("upcloud_firewall_rules", "id") => Some("id"),
        // upcloud_managed_database_* (postgresql / valkey / mysql / opensearch share these)
        (t, "id")       if t.starts_with("upcloud_managed_database") => Some("id"),
        (t, "endpoint") if t.starts_with("upcloud_managed_database") => Some("service_host"),
        (t, "address")  if t.starts_with("upcloud_managed_database") => Some("service_host"),
        (t, "port")     if t.starts_with("upcloud_managed_database") => Some("service_port"),
        (t, "username") if t.starts_with("upcloud_managed_database") => Some("service_username"),
        (t, "password") if t.starts_with("upcloud_managed_database") => Some("service_password"),
        (t, "primary_endpoint_address")
                        if t.starts_with("upcloud_managed_database") => Some("service_host"),
        // upcloud_kubernetes_cluster
        ("upcloud_kubernetes_cluster", "id") => Some("id"),
        // upcloud_floating_ip_address
        ("upcloud_floating_ip_address", "id")            => Some("id"),
        ("upcloud_floating_ip_address", "public_ip")     => Some("ip_address"),
        ("upcloud_floating_ip_address", "allocation_id") => Some("id"),
        _ => None,
    }
}

/// When the attribute has no direct mapping a `<TODO: was .attr>` suffix is
/// injected so it surfaces in the TODO review screen.  Unknown AWS resource
/// types get a full `<TODO: was aws_type.name.attr>` replacement.
fn rewrite_output_refs(s: &str) -> String {
    let mut result = s.to_string();
    let mut search_from = 0usize;

    loop {
        let Some(rel) = result[search_from..].find("aws_") else { break };
        let start = search_from + rel;

        if inside_todo_marker(&result, start) {
            search_from = start + 4;
            continue;
        }

        // Capture the full Terraform traversal: TYPE.NAME.ATTR[…].subattr…
        // Valid chars: alphanumeric, '_', '.', '[', ']', '*' (splat operator in [*])
        let end = result[start..]
            .find(|c: char| !c.is_alphanumeric() && c != '.' && c != '_' && c != '[' && c != ']' && c != '*')
            .map(|off| start + off)
            .unwrap_or(result.len());

        let candidate = &result[start..end];

        // Need at least two dots: aws_TYPE.NAME.ATTR
        if candidate.matches('.').count() < 2 {
            search_from = end;
            continue;
        }

        // Split into aws_type / resource_name / attr_path
        let first_dot  = candidate.find('.').unwrap();
        let second_dot = first_dot + 1
            + candidate[first_dot + 1..].find('.').unwrap();
        let aws_type      = &candidate[..first_dot];
        let resource_name = &candidate[first_dot + 1..second_dot];
        let attr_path     = &candidate[second_dot + 1..];

        // The lookup key is just the first identifier segment (before '[' or '.')
        let attr_key = attr_path
            .split(|c: char| c == '[' || c == '.')
            .next()
            .unwrap_or(attr_path);

        let new_ref = if let Some(upcloud_type) = upcloud_type_for_aws(aws_type) {
            let upcloud_name = upcloud_resource_name_for(aws_type, resource_name);
            if let Some(upcloud_attr) = upcloud_attr_for(upcloud_type, attr_key) {
                format!("{}.{}.{}", upcloud_type, upcloud_name, upcloud_attr)
            } else {
                format!(
                    "{}.{}.<TODO: was .{}, check UpCloud provider docs>",
                    upcloud_type, upcloud_name, attr_path,
                )
            }
        } else {
            format!("<TODO: was {}, no known UpCloud equivalent>", candidate)
        };

        let owned = candidate.to_string();
        result = result.replacen(&owned, &new_ref, 1);
        search_from = 0; // restart — string length may have changed
    }

    result
}

/// Extract the count value from generated upcloud_server HCL.
/// Looks for a line like `  count    = 2`.
fn extract_count_from_hcl(hcl: &str) -> Option<String> {
    hcl.lines()
        .find(|l| {
            let t = l.trim_start();
            t.starts_with("count") && t.contains('=')
        })
        .and_then(|l| l.split('=').nth(1))
        .map(|v| v.trim().to_string())
}

/// Scan an `aws_instance` source HCL block and return all security group resource names
/// referenced in `vpc_security_group_ids` or `security_groups` attributes.
/// E.g. `vpc_security_group_ids = [aws_security_group.docker_demo.id]` → `["docker_demo"]`
/// Extract subnet resource names from an `aws_db_subnet_group` or
/// `aws_elasticache_subnet_group` source HCL block.
/// Looks for `aws_subnet.NAME.id` (or `.name`) patterns in the `subnet_ids` list.
fn extract_subnet_names_from_subnet_group(hcl: &str) -> Vec<String> {
    let mut names = Vec::new();
    const PREFIX: &str = "aws_subnet.";
    for word in hcl.split(|c: char| c == ',' || c == '[' || c == ']' || c == '\n' || c == ' ') {
        let word = word.trim();
        if let Some(after) = word.strip_prefix(PREFIX) {
            // after = "NAME.id" or "NAME.name" — take the first segment
            let name_end = after.find('.').unwrap_or(after.len());
            let name = &after[..name_end];
            if !name.is_empty() && !names.contains(&name.to_string()) {
                names.push(name.to_string());
            }
        }
    }
    names
}

/// Extract the subnet name from an `aws_instance` source HCL block.
/// Returns e.g. `"public_a"` from `subnet_id = aws_subnet.public_a.id`.
fn extract_subnet_id_from_instance_hcl(hcl: &str) -> Option<String> {
    for line in hcl.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("subnet_id") && trimmed.contains('=') {
            if let Some(pos) = trimmed.find("aws_subnet.") {
                let after = &trimmed[pos + "aws_subnet.".len()..];
                // Strip trailing `.id`, `.arn`, etc. — take only the resource name segment
                let name = after.split('.').next().unwrap_or("").trim_matches('"');
                if !name.is_empty() {
                    return Some(name.to_string());
                }
            }
        }
    }
    None
}

/// Extract the target group name from an `aws_lb_listener` source HCL block.
/// Looks for `target_group_arn = aws_lb_target_group.NAME.arn` inside a forward action.
fn extract_tg_from_listener_source_hcl(hcl: &str) -> Option<String> {
    const PREFIX: &str = "aws_lb_target_group.";
    for line in hcl.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("target_group_arn") && trimmed.contains(PREFIX) {
            if let Some(pos) = trimmed.find(PREFIX) {
                let after = &trimmed[pos + PREFIX.len()..];
                let name = after.split('.').next().unwrap_or("").trim_matches('"');
                if !name.is_empty() {
                    return Some(name.to_string());
                }
            }
        }
    }
    None
}

/// Extract (tg_name, server_name) from an `aws_lb_target_group_attachment` source HCL.
/// Returns e.g. `("web", "web")` from:
///   `target_group_arn = aws_lb_target_group.web.arn`
///   `target_id        = aws_instance.web[0].id`
fn extract_tg_server_from_attachment_source_hcl(hcl: &str) -> Option<(String, String)> {
    let mut tg_name: Option<String> = None;
    let mut server_name: Option<String> = None;
    for line in hcl.lines() {
        let trimmed = line.trim();
        if tg_name.is_none() && trimmed.starts_with("target_group_arn") {
            if let Some(pos) = trimmed.find("aws_lb_target_group.") {
                let after = &trimmed[pos + "aws_lb_target_group.".len()..];
                let name = after.split('.').next().unwrap_or("").trim_matches('"');
                if !name.is_empty() {
                    tg_name = Some(name.to_string());
                }
            }
        }
        if server_name.is_none() && trimmed.starts_with("target_id") {
            if let Some(pos) = trimmed.find("aws_instance.") {
                let after = &trimmed[pos + "aws_instance.".len()..];
                // Strip any index suffix like `[0]` — take alphanumeric+underscore only
                let name: String = after.chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect();
                if !name.is_empty() {
                    server_name = Some(name);
                }
            }
        }
    }
    match (tg_name, server_name) {
        (Some(tg), Some(srv)) => Some((tg, srv)),
        _ => None,
    }
}

fn extract_sg_refs_from_instance_hcl(hcl: &str) -> Vec<String> {
    let mut refs = Vec::new();
    const PREFIX: &str = "aws_security_group.";
    for line in hcl.lines() {
        let mut search = line;
        while let Some(pos) = search.find(PREFIX) {
            let after = &search[pos + PREFIX.len()..];
            let end = after
                .find(|c: char| !c.is_alphanumeric() && c != '_')
                .unwrap_or(after.len());
            let name = &after[..end];
            if !name.is_empty() {
                refs.push(name.to_string());
            }
            search = &search[pos + 1..];
        }
    }
    refs
}

/// Extract the `server_id = <expr>` RHS from a resolved `upcloud_firewall_rules` HCL block.
fn extract_fw_server_id_expr(hcl: &str) -> Option<String> {
    hcl.lines()
        .find(|l| {
            let t = l.trim_start();
            t.starts_with("server_id") && t.contains('=')
        })
        .and_then(|l| l.split_once('='))
        .map(|(_, rhs)| rhs.trim().to_string())
}

/// Extract the `count = <expr>` line (with leading spaces) from HCL, if present.
/// Returns the full line including its leading indentation and trailing newline.
fn extract_fw_count_line(hcl: &str) -> Option<String> {
    hcl.lines()
        .find(|l| {
            let t = l.trim_start();
            t.starts_with("count") && t.contains('=') && !t.starts_with("count.index")
        })
        .map(|l| format!("{}\n", l))
}

/// Extract all `firewall_rule { ... }` blocks from a resolved `upcloud_firewall_rules` HCL.
/// Returns each block as a string including its leading indentation.
fn extract_fw_rule_blocks(hcl: &str) -> Vec<String> {
    let mut blocks: Vec<String> = Vec::new();
    let mut current: Option<String> = None;
    let mut depth = 0i32;
    for line in hcl.lines() {
        let trimmed = line.trim_start();
        if current.is_none() {
            if trimmed.starts_with("firewall_rule") && trimmed.contains('{') {
                current = Some(format!("{}\n", line));
                depth = 1;
            }
        } else {
            let block = current.as_mut().unwrap();
            block.push_str(line);
            block.push('\n');
            depth += line.chars().filter(|&c| c == '{').count() as i32;
            depth -= line.chars().filter(|&c| c == '}').count() as i32;
            if depth <= 0 {
                blocks.push(current.take().unwrap());
                depth = 0;
            }
        }
    }
    blocks
}

/// Build a merged `upcloud_firewall_rules` HCL block from collected rule blocks.
/// Deduplicates identical catch-all outbound rules before writing.
fn build_merged_fw_hcl(
    resource_name: &str,
    server_id_expr: &str,
    count_line: Option<&str>,
    rule_blocks: &[String],
) -> String {
    let mut s = format!("resource \"upcloud_firewall_rules\" \"{}\" {{\n", resource_name);
    if let Some(count) = count_line {
        s.push_str(count);
    }
    s.push_str(&format!("  server_id = {}\n", server_id_expr));
    // Deduplicate identical rule blocks (e.g. catch-all outbound added by every SG).
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    for block in rule_blocks {
        if seen.insert(block.clone()) {
            s.push('\n');
            s.push_str(block);
        }
    }
    s.push_str("}\n");
    s
}

/// Extract the server logical name from a server_id expression.
/// `upcloud_server.myserver.id` → `"myserver"`
/// `upcloud_server.myserver[count.index].id` → `"myserver"`
fn server_name_from_server_id_expr(expr: &str) -> String {
    expr.strip_prefix("upcloud_server.")
        .and_then(|s| s.split('[').next())
        .and_then(|s| s.split('.').next())
        .unwrap_or("server")
        .to_string()
}

/// Resolve `upcloud_server.<TODO>.id` in a `upcloud_firewall_rules` block using name-matching.
///
/// - If the SG name matches a server with no count: simple `upcloud_server.NAME.id`
/// - If the SG name matches a server with count N: adds `count = N` and uses
///   `upcloud_server.NAME[count.index].id`
/// - If no match: returns the HCL unchanged (TODO stays for manual editing)
fn resolve_firewall_server(
    hcl: &str,
    sg_name: &str,
    server_info_map: &HashMap<String, Option<String>>,
) -> String {
    match server_info_map.get(sg_name) {
        None => hcl.to_string(),
        Some(None) => hcl.replace(
            "upcloud_server.<TODO>.id",
            &format!("upcloud_server.{}.id", sg_name),
        ),
        Some(Some(n)) => {
            // Replace the server_id reference with an indexed one
            let mut s = hcl.replace(
                "upcloud_server.<TODO>.id",
                &format!("upcloud_server.{}[count.index].id", sg_name),
            );
            // Insert `count = N` on the line before `server_id`
            s = s.replacen(
                "  server_id =",
                &format!("  count     = {}\n  server_id =", n),
                1,
            );
            s
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::migration::types::{MigrationResult, MigrationStatus};

    fn make_result(
        resource_type: &str,
        resource_name: &str,
        upcloud_type: &str,
        upcloud_hcl: Option<&str>,
        snippet: Option<&str>,
    ) -> MigrationResult {
        MigrationResult {
            resource_type: resource_type.to_string(),
            resource_name: resource_name.to_string(),
            source_file: "test.tf".to_string(),
            status: MigrationStatus::Native,
            score: 0,
            upcloud_type: upcloud_type.to_string(),
            upcloud_hcl: upcloud_hcl.map(str::to_string),
            snippet: snippet.map(str::to_string),
            parent_resource: None,
            notes: vec![],
            source_hcl: None,
        }
    }

    fn run_generate(results: &[MigrationResult], test_name: &str) -> String {
        let dir = std::env::temp_dir().join(format!("upcloud_gen_test_{}", test_name));
        std::fs::create_dir_all(&dir).unwrap();
        let mut log = vec![];
        generate_files(results, &[], &dir, None, "fi-hel2", &mut log).unwrap();
        let content = std::fs::read_to_string(dir.join("test.tf"))
            .expect("generate_files must have created test.tf");
        let _ = std::fs::remove_dir_all(&dir);
        content
    }

    #[test]
    fn login_key_ternary_expression_is_not_mangled() {
        // Snippet uses unquoted HCL expression (the fixed map_key_pair format).
        // generate_files must extract the expression and replace the quoted TODO
        // placeholder including its surrounding quotes, producing keys = [expr].
        let key_pair = make_result(
            "aws_key_pair",
            "main",
            "login block (server resource)",
            None,
            Some(
                r#"login {
  user = "root"
  keys = [var.ssh_public_key != "" ? var.ssh_public_key : "ssh-rsa PLACEHOLDER"]  # was aws_key_pair.main
}"#,
            ),
        );
        let server = make_result(
            "aws_instance",
            "web",
            "upcloud_server",
            Some(
                r#"resource "upcloud_server" "web" {
  hostname = "web"
  zone     = "__ZONE__"
  plan     = "1xCPU-1GB"

  template {
    storage = "Ubuntu Server 24.04 LTS (Noble Numbat)"
    size    = 50
  }

  network_interface {
    type = "public"
  }

  login {
    user = "root"
    keys = ["<TODO: SSH public key for aws_key_pair.main>"]
  }
}
"#,
            ),
            None,
        );
        let output = run_generate(&[key_pair, server], "ternary_login");
        assert!(
            output.contains("keys = [var.ssh_public_key"),
            "ternary expression should appear unquoted in keys\n{output}"
        );
        assert!(
            !output.contains(r#"keys = ["var.ssh_public_key"#),
            "ternary expression must not be wrapped in extra quotes\n{output}"
        );
    }

    #[test]
    fn counted_network_ref_in_counted_server_uses_count_index() {
        let network = make_result(
            "aws_subnet",
            "public",
            "upcloud_network",
            Some(
                r#"resource "upcloud_network" "public" {
  count = 2
  name  = "public-${count.index + 1}"
  zone  = "__ZONE__"

  ip_network {
    address = "10.0.${count.index + 1}.0/24"
    dhcp    = true
    family  = "IPv4"
  }
}
"#,
            ),
            None,
        );
        let server = make_result(
            "aws_instance",
            "web",
            "upcloud_server",
            Some(
                r#"resource "upcloud_server" "web" {
  count    = 2
  hostname = "web-${count.index + 1}"
  zone     = "__ZONE__"
  plan     = "1xCPU-1GB"

  template {
    storage = "Ubuntu Server 24.04 LTS (Noble Numbat)"
    size    = 50
  }

  network_interface {
    type = "public"
  }

  network_interface {
    type    = "private"
    network = "<TODO: upcloud_network reference>"
  }
}
"#,
            ),
            None,
        );
        let output = run_generate(&[network, server], "counted_net_counted_srv");
        assert!(
            output.contains("upcloud_network.public[count.index].id"),
            "counted server + counted network should use [count.index]\n{output}"
        );
    }

    #[test]
    fn counted_network_ref_in_uncounted_server_uses_zero_index() {
        let network = make_result(
            "aws_subnet",
            "public",
            "upcloud_network",
            Some(
                r#"resource "upcloud_network" "public" {
  count = 2
  name  = "public-${count.index + 1}"
  zone  = "__ZONE__"

  ip_network {
    address = "10.0.${count.index + 1}.0/24"
    dhcp    = true
    family  = "IPv4"
  }
}
"#,
            ),
            None,
        );
        let server = make_result(
            "aws_instance",
            "database",
            "upcloud_server",
            Some(
                r#"resource "upcloud_server" "database" {
  hostname = "database"
  zone     = "__ZONE__"
  plan     = "1xCPU-2GB"

  template {
    storage = "Ubuntu Server 24.04 LTS (Noble Numbat)"
    size    = 50
  }

  network_interface {
    type = "public"
  }

  network_interface {
    type    = "private"
    network = "<TODO: upcloud_network reference>"
  }
}
"#,
            ),
            None,
        );
        let output = run_generate(&[network, server], "counted_net_uncounted_srv");
        assert!(
            output.contains("upcloud_network.public[0].id"),
            "non-counted server + counted network should use [0]\n{output}"
        );
    }

    #[test]
    fn lb_private_network_block_prefers_private_named_network_over_public() {
        // Regression: when networks are ordered [public_a, private_a], the generic
        // "<TODO: upcloud_network reference>" fallback was picking public_a because
        // it was first. The LB's `type = "private"` block must use the private network.
        let public_net = make_result(
            "aws_subnet",
            "public_a",
            "upcloud_network",
            Some(
                r#"resource "upcloud_network" "public_a" {
  zone = "__ZONE__"
  name = "public-a"

  ip_network {
    address = "10.0.1.0/24"
    dhcp    = true
    family  = "IPv4"
  }
}
"#,
            ),
            None,
        );
        let private_net = make_result(
            "aws_subnet",
            "private_a",
            "upcloud_network",
            Some(
                r#"resource "upcloud_network" "private_a" {
  zone = "__ZONE__"
  name = "private-a"

  ip_network {
    address = "10.0.2.0/24"
    dhcp    = true
    family  = "IPv4"
  }
}
"#,
            ),
            None,
        );
        let lb = make_result(
            "aws_lb",
            "main",
            "upcloud_loadbalancer",
            Some(
                r#"resource "upcloud_loadbalancer" "main" {
  name              = "main"
  plan              = "development"
  zone              = "__ZONE__"
  configured_status = "started"

  networks {
    name    = "private"
    type    = "private"
    family  = "IPv4"
    network = "<TODO: upcloud_network reference>"
  }

  networks {
    name   = "public"
    type   = "public"
    family = "IPv4"
  }
}
"#,
            ),
            None,
        );

        // public_a comes first in the slice — it would have been picked by the old logic
        let output = run_generate(&[public_net, private_net, lb], "lb_private_net_preference");

        assert!(
            output.contains("upcloud_network.private_a.id"),
            "LB private network block must resolve to private_a, not public_a\n{output}"
        );
        assert!(
            !output.contains("network = upcloud_network.public_a.id"),
            "LB private network block must NOT reference public_a\n{output}"
        );
    }

    #[test]
    fn lb_private_network_resolved_deterministically_via_backend_chain() {
        // Regression: with `public_a` ordered first AND named "public_a", the heuristic
        // could not resolve correctly because the actual backends live in public_a.
        // The deterministic chain (listener → attachment → instance → subnet) must win.
        //
        // Chain: aws_lb.main → listener (TG=web) → attachment (web[0] → public_a) →
        //        aws_instance.web (subnet_id = aws_subnet.public_a) → upcloud_network.public_a
        let public_net = make_result(
            "aws_subnet", "public_a", "upcloud_network",
            Some(r#"resource "upcloud_network" "public_a" {
  zone = "__ZONE__"
  name = "public-a"
  ip_network { address = "10.0.1.0/24" dhcp = true family = "IPv4" }
}
"#),
            None,
        );
        let private_net = make_result(
            "aws_subnet", "private_a", "upcloud_network",
            Some(r#"resource "upcloud_network" "private_a" {
  zone = "__ZONE__"
  name = "private-a"
  ip_network { address = "10.0.2.0/24" dhcp = true family = "IPv4" }
}
"#),
            None,
        );

        // aws_instance.web lives in public_a
        let mut instance = make_result(
            "aws_instance", "web", "upcloud_server",
            Some(r#"resource "upcloud_server" "web" {
  hostname = "web"
  zone     = "__ZONE__"
  plan     = "1xCPU-1GB"
  template { storage = "Ubuntu Server 24.04 LTS (Noble Numbat)" size = 50 }
}
"#),
            None,
        );
        instance.source_hcl = Some(
            r#"resource "aws_instance" "web" {
  ami           = "ami-12345"
  instance_type = "t3.micro"
  subnet_id     = aws_subnet.public_a.id
}
"#.to_string(),
        );

        // aws_lb_target_group_attachment: TG=web, server=web[0]
        let mut attachment = make_result(
            "aws_lb_target_group_attachment", "web_1", "upcloud_loadbalancer_static_backend_member",
            Some(r#"resource "upcloud_loadbalancer_static_backend_member" "web_1" {
  backend      = upcloud_loadbalancer_backend.<TODO>.name
  name         = "web-1"
  ip           = "<TODO: server IP>"
  port         = 80
  weight       = 100
  max_sessions = 1000
}
"#),
            None,
        );
        attachment.source_hcl = Some(
            r#"resource "aws_lb_target_group_attachment" "web_1" {
  target_group_arn = aws_lb_target_group.web.arn
  target_id        = aws_instance.web[0].id
  port             = 80
}
"#.to_string(),
        );

        // aws_lb_listener: lb=main, TG=web
        let mut listener = make_result(
            "aws_lb_listener", "https", "upcloud_loadbalancer_frontend",
            Some(r#"resource "upcloud_loadbalancer_frontend" "https" {
  name             = "https"
  mode             = "tcp"
  port             = 443
  loadbalancer     = upcloud_loadbalancer.<TODO>.id
  default_backend_name = upcloud_loadbalancer_backend.<TODO>.name
}
"#),
            None,
        );
        listener.source_hcl = Some(
            r#"resource "aws_lb_listener" "https" {
  load_balancer_arn = aws_lb.main.arn
  port              = "443"
  protocol          = "HTTPS"
  default_action {
    type             = "forward"
    target_group_arn = aws_lb_target_group.web.arn
  }
}
"#.to_string(),
        );

        // aws_lb.main: has the generic network placeholder
        let lb = make_result(
            "aws_lb", "main", "upcloud_loadbalancer",
            Some(r#"resource "upcloud_loadbalancer" "main" {
  name              = "main"
  plan              = "development"
  zone              = "__ZONE__"
  configured_status = "started"
  networks {
    name    = "private"
    type    = "private"
    family  = "IPv4"
    network = "<TODO: upcloud_network reference>"
  }
  networks {
    name   = "public"
    type   = "public"
    family = "IPv4"
  }
}
"#),
            None,
        );

        // public_a is ordered first (and is named "public_a") — heuristic would pick private_a,
        // but the deterministic chain shows the backends are actually in public_a.
        let output = run_generate(
            &[public_net, private_net, instance, attachment, listener, lb],
            "lb_deterministic_chain",
        );

        assert!(
            output.contains("upcloud_network.public_a.id"),
            "LB private network block must resolve to public_a via backend chain\n{output}"
        );
        assert!(
            !output.contains("upcloud_network.private_a.id"),
            "LB must NOT reference private_a when backends are in public_a\n{output}"
        );
    }

    #[test]
    fn backend_member_ip_ref_for_counted_server_uses_zero_index() {
        let server = make_result(
            "aws_instance",
            "web",
            "upcloud_server",
            Some(
                r#"resource "upcloud_server" "web" {
  count    = 2
  hostname = "web-${count.index + 1}"
  zone     = "__ZONE__"
  plan     = "1xCPU-1GB"

  template {
    storage = "Ubuntu Server 24.04 LTS (Noble Numbat)"
    size    = 50
  }

  network_interface {
    type = "public"
  }
}
"#,
            ),
            None,
        );
        let backend = make_result(
            "aws_lb_target_group",
            "web",
            "upcloud_loadbalancer_backend",
            Some(
                r#"resource "upcloud_loadbalancer_backend" "web" {
  loadbalancer = upcloud_loadbalancer.<TODO>.id
  name         = "web"

  properties {
    health_check_type = "http"
  }
}

resource "upcloud_loadbalancer_static_backend_member" "web_member" {
  backend      = upcloud_loadbalancer_backend.web.id
  name         = "web-member"
  weight       = 100
  max_sessions = 1000
  enabled      = true
  ip           = "<TODO: server IP>"
  port         = 80
}
"#,
            ),
            None,
        );
        let output = run_generate(&[server, backend], "backend_counted_srv");
        assert!(
            output.contains("upcloud_server.web[0].network_interface[0].ip_address"),
            "backend member ip should use [0] for counted server\n{output}"
        );
        assert!(
            !output.contains("upcloud_server.web.network_interface"),
            "should not reference non-indexed counted server\n{output}"
        );
    }

    #[test]
    fn extract_count_finds_count_line() {
        let hcl = "resource \"upcloud_server\" \"web\" {\n  count    = 2\n  hostname = \"web\"\n}\n";
        assert_eq!(extract_count_from_hcl(hcl), Some("2".to_string()));
    }

    #[test]
    fn extract_count_returns_none_when_absent() {
        let hcl = "resource \"upcloud_server\" \"web\" {\n  hostname = \"web\"\n}\n";
        assert_eq!(extract_count_from_hcl(hcl), None);
    }

    #[test]
    fn firewall_rule_with_unresolved_todo_is_skipped_even_with_other_servers() {
        // aws_security_group "lb" → upcloud_firewall_rules "lb", but there is no
        // upcloud_server named "lb". Even though other servers exist (web, app),
        // the rule must be dropped entirely — not emitted with a broken <TODO>.
        let lb_fw = make_result(
            "aws_security_group",
            "lb",
            "upcloud_firewall_rules",
            Some(
                "resource \"upcloud_firewall_rules\" \"lb\" {\n  \
                 server_id = upcloud_server.<TODO>.id\n}\n",
            ),
            None,
        );
        let web = make_result(
            "aws_instance",
            "web",
            "upcloud_server",
            Some(
                "resource \"upcloud_server\" \"web\" {\n  \
                 hostname = \"web\"\n  zone = \"__ZONE__\"\n  plan = \"1xCPU-1GB\"\n}\n",
            ),
            None,
        );
        let app = make_result(
            "aws_instance",
            "app",
            "upcloud_server",
            Some(
                "resource \"upcloud_server\" \"app\" {\n  \
                 hostname = \"app\"\n  zone = \"__ZONE__\"\n  plan = \"1xCPU-1GB\"\n}\n",
            ),
            None,
        );
        let output = run_generate(&[lb_fw, web, app], "fw_lb_skip");
        assert!(
            !output.contains("upcloud_firewall_rules"),
            "lb firewall rule must be skipped when no server named 'lb' exists\n{output}"
        );
        assert!(
            !output.contains("upcloud_server.<TODO>"),
            "no unresolved TODO must appear in output\n{output}"
        );
        // The server resources must still be present
        assert!(output.contains("upcloud_server\" \"web\""), "{output}");
        assert!(output.contains("upcloud_server\" \"app\""), "{output}");
    }

    #[test]
    fn resolve_firewall_no_match_leaves_todo() {
        let hcl = "resource \"upcloud_firewall_rules\" \"lb\" {\n  server_id = upcloud_server.<TODO>.id\n}\n";
        let map = HashMap::new();
        let out = resolve_firewall_server(hcl, "lb", &map);
        assert!(out.contains("upcloud_server.<TODO>.id"), "{out}");
    }

    #[test]
    fn sg_to_server_map_resolves_mismatched_names() {
        // docker_demo SG is attached to docker_server instance via vpc_security_group_ids.
        let instance_hcl = concat!(
            "resource \"aws_instance\" \"docker_server\" {\n",
            "  vpc_security_group_ids = [aws_security_group.docker_demo.id]\n",
            "}\n"
        );
        let refs = extract_sg_refs_from_instance_hcl(instance_hcl);
        assert!(refs.contains(&"docker_demo".to_string()), "should extract SG name: {refs:?}");

        // When the sg_to_server_map is used, the firewall rule should resolve to docker_server.
        let fw_hcl = "resource \"upcloud_firewall_rules\" \"docker_demo\" {\n  server_id = upcloud_server.<TODO>.id\n}\n";
        let mut server_map = HashMap::new();
        server_map.insert("docker_server".to_string(), None);
        // effective_server comes from sg_to_server_map lookup
        let effective_server = "docker_server";
        let out = resolve_firewall_server(fw_hcl, effective_server, &server_map);
        assert!(out.contains("upcloud_server.docker_server.id"), "should resolve to docker_server: {out}");
        assert!(!out.contains("<TODO>"), "no TODO should remain: {out}");
    }

    #[test]
    fn resolve_firewall_single_server_no_count() {
        let hcl = "resource \"upcloud_firewall_rules\" \"redis\" {\n  server_id = upcloud_server.<TODO>.id\n}\n";
        let mut map = HashMap::new();
        map.insert("redis".to_string(), None);
        let out = resolve_firewall_server(hcl, "redis", &map);
        assert!(out.contains("upcloud_server.redis.id"), "{out}");
        assert!(!out.contains("<TODO>"), "{out}");
        assert!(!out.contains("count"), "{out}");
    }

    #[test]
    fn resolve_firewall_counted_server_injects_count_and_index() {
        let hcl = "resource \"upcloud_firewall_rules\" \"web\" {\n  server_id = upcloud_server.<TODO>.id\n}\n";
        let mut map = HashMap::new();
        map.insert("web".to_string(), Some("2".to_string()));
        let out = resolve_firewall_server(hcl, "web", &map);
        assert!(out.contains("count     = 2"), "{out}");
        assert!(out.contains("upcloud_server.web[count.index].id"), "{out}");
        assert!(!out.contains("<TODO>"), "{out}");
    }

    // ── remove_todo_interpolations ────────────────────────────────────────────

    #[test]
    fn todo_interpolation_stripped_from_heredoc() {
        let input = "proxy_pass http://${<TODO: remove AWS resource ref>[0].private_ip}:3000;";
        let out = remove_todo_interpolations(input.to_string());
        assert_eq!(out, "proxy_pass http://<TODO: remove AWS resource ref>:3000;");
    }

    #[test]
    fn todo_interpolation_with_space_after_brace() {
        let input = "url: '${ <TODO: remove AWS resource ref>}';";
        let out = remove_todo_interpolations(input.to_string());
        assert_eq!(out, "url: '<TODO: remove AWS resource ref>';");
    }

    #[test]
    fn valid_interpolations_are_not_touched() {
        let input = "hostname = \"web-${count.index + 1}\"";
        let out = remove_todo_interpolations(input.to_string());
        assert_eq!(out, input);
    }

    #[test]
    fn multiple_todo_interpolations_all_replaced() {
        let input =
            "a=${<TODO: remove AWS resource ref>.x} b=${<TODO: remove AWS resource ref>.y}";
        let out = remove_todo_interpolations(input.to_string());
        assert_eq!(
            out,
            "a=<TODO: remove AWS resource ref> b=<TODO: remove AWS resource ref>"
        );
    }

    // ── End-to-end: user_data with AWS cross-refs ─────────────────────────────
    //
    // Ensures that when user_data heredocs contain ${aws_*.*.attr} references,
    // the generated output strips the invalid `${...}` wrapper and the result
    // is parseable by hcl-rs (i.e. `terraform validate`-able syntax).

    #[test]
    fn user_data_with_aws_refs_produces_valid_hcl() {
        use crate::migration::mapper::map_resource;
        use crate::terraform::parser::parse_tf_file;

        // A server whose user_data references a cross-resource AWS IP (common
        // pattern for nginx reverse-proxy configurations).
        let tf_source = r#"
resource "aws_instance" "web" {
  instance_type = "t3.micro"
  user_data = <<-EOF
#!/bin/bash
proxy_pass http://${aws_instance.app[0].private_ip}:3000;
redis_url="${aws_instance.redis.private_ip}:6379"
db_host="${aws_instance.database.private_ip}"
EOF
}

resource "aws_instance" "app" {
  instance_type = "t3.small"
}
"#;

        let dir = std::env::temp_dir().join("upcloud_e2e_userdata");
        std::fs::create_dir_all(&dir).unwrap();
        let tf_path = dir.join("main.tf");
        std::fs::write(&tf_path, tf_source).unwrap();

        let parsed = parse_tf_file(&tf_path).expect("source terraform should parse");
        let results: Vec<MigrationResult> =
            parsed.resources.iter().map(|r| map_resource(r)).collect();

        let out_dir = dir.join("out");
        let mut log = vec![];
        generate_files(&results, &[], &out_dir, None, "fi-hel2", &mut log).unwrap();
        let output =
            std::fs::read_to_string(out_dir.join("main.tf"))
            .expect("generate_files must produce main.tf");
        let _ = std::fs::remove_dir_all(&dir);

        // No "${<TODO:...>" must remain — those are invalid HCL.
        assert!(
            !output.contains("${<TODO:"),
            "no invalid ${{<TODO:}} interpolations must remain in output\n{output}"
        );

        // The TODO text should still appear so the user knows what to fix.
        assert!(
            output.contains("<TODO: remove AWS resource ref>"),
            "TODO placeholder must still be visible in output\n{output}"
        );

        // The generated file must be parseable by hcl-rs (i.e. valid HCL syntax).
        hcl::from_str::<hcl::Body>(&output)
            .expect("generated output must be valid HCL");
    }

    // ── End-to-end: test-fixtures/webapp-e2e.tf ───────────────────────────────
    //
    // Full pipeline test: parse HCL → map resources → generate UpCloud Terraform.
    // The fixture covers all mapped resource types and includes the ternary
    // public_key expression that previously triggered a missing-closing-quote bug.
    #[test]
    fn webapp_terraform_example_end_to_end() {
        use crate::migration::mapper::map_resource;
        use crate::terraform::parser::parse_tf_file;

        let tf_path = std::path::PathBuf::from("test-fixtures/webapp-e2e.tf");
        let parsed = parse_tf_file(&tf_path).expect("test-fixtures/webapp-e2e.tf should parse");
        let results: Vec<MigrationResult> = parsed.resources.iter().map(|r| map_resource(r)).collect();

        let out_dir = std::env::temp_dir().join("upcloud_e2e_webapp");
        let mut log = vec![];
        generate_files(&results, &[], &out_dir, None, "fi-hel2", &mut log).unwrap();
        let output = std::fs::read_to_string(out_dir.join("webapp-e2e.tf"))
            .expect("generate_files must produce webapp-e2e.tf");
        let _ = std::fs::remove_dir_all(&out_dir);

        // No unresolved server_id TODOs in the entire output.
        assert!(
            !output.contains("upcloud_server.<TODO>"),
            "no unresolved server_id TODO should remain in output\n{output}"
        );

        // LB security group must be dropped — 'lb' is not a server name.
        assert!(
            !output.contains("upcloud_firewall_rules\" \"lb\""),
            "firewall_rules for 'lb' SG must be skipped (LB uses built-in security)\n{output}"
        );

        // Server-attached firewall rules must be present with resolved server_id.
        assert!(
            output.contains("upcloud_firewall_rules\" \"web\""),
            "firewall_rules for 'web' must be generated\n{output}"
        );
        assert!(
            output.contains("upcloud_server.web[count.index].id"),
            "web firewall server_id must use count.index (web has count=2)\n{output}"
        );
        assert!(
            output.contains("upcloud_firewall_rules\" \"app\""),
            "firewall_rules for 'app' must be generated\n{output}"
        );
        assert!(
            output.contains("upcloud_server.app[count.index].id"),
            "app firewall server_id must use count.index (app has count=2)\n{output}"
        );
        assert!(
            output.contains("upcloud_firewall_rules\" \"database\""),
            "firewall_rules for 'database' must be generated\n{output}"
        );
        assert!(
            output.contains("upcloud_server.database.id"),
            "database firewall server_id must be resolved (no count)\n{output}"
        );
        assert!(
            output.contains("upcloud_firewall_rules\" \"redis\""),
            "firewall_rules for 'redis' must be generated\n{output}"
        );
        assert!(
            output.contains("upcloud_server.redis.id"),
            "redis firewall server_id must be resolved (no count)\n{output}"
        );

        // Login block: ternary else-branch closing `"` must not be stripped.
        assert!(
            output.contains(r#"placeholder-key"]"#),
            "ternary else-branch closing quote must be intact in login block\n{output}"
        );

        // Server resources.
        assert!(output.contains("resource \"upcloud_server\" \"web\""), "{output}");
        assert!(output.contains("resource \"upcloud_server\" \"app\""), "{output}");
        assert!(output.contains("resource \"upcloud_server\" \"database\""), "{output}");
        assert!(output.contains("resource \"upcloud_server\" \"redis\""), "{output}");

        // Networking: VPC → router, subnet → network.
        assert!(output.contains("resource \"upcloud_router\" \"main_router\""), "{output}");
        assert!(output.contains("resource \"upcloud_network\" \"public\""), "{output}");

        // Load balancer and its components.
        assert!(output.contains("resource \"upcloud_loadbalancer\" \"main\""), "{output}");
        assert!(output.contains("type   = \"public\""), "LB must have public networks block\n{output}");
        assert!(output.contains("resource \"upcloud_loadbalancer_backend\" \"web\""), "{output}");
        assert!(output.contains("resource \"upcloud_loadbalancer_frontend\" \"http\""), "{output}");
    }

    // ── Multi-SG merge ────────────────────────────────────────────────────────

    #[test]
    fn two_sgs_targeting_same_server_produce_one_firewall_resource() {
        // Two security groups (web_sg, db_sg) both attached to server "app".
        // Both have vpc_security_group_ids resolved via source HCL on the instance.
        // The generator must emit exactly ONE upcloud_firewall_rules resource containing
        // all rules, not two conflicting resources.
        let fw_web = make_result(
            "aws_security_group",
            "web_sg",
            "upcloud_firewall_rules",
            Some(
                "resource \"upcloud_firewall_rules\" \"web_sg\" {\n  \
                 server_id = upcloud_server.<TODO>.id\n\n  \
                 firewall_rule {\n    direction = \"in\"\n    action    = \"accept\"\n    \
                 family    = \"IPv4\"\n    protocol  = \"tcp\"\n    \
                 destination_port_start = \"80\"\n    destination_port_end   = \"80\"\n  }\n\n  \
                 firewall_rule {\n    direction = \"out\"\n    action    = \"accept\"\n    \
                 family    = \"IPv4\"\n    comment   = \"Allow all outbound\"\n  }\n}\n",
            ),
            None,
        );
        let fw_db = make_result(
            "aws_security_group",
            "db_sg",
            "upcloud_firewall_rules",
            Some(
                "resource \"upcloud_firewall_rules\" \"db_sg\" {\n  \
                 server_id = upcloud_server.<TODO>.id\n\n  \
                 firewall_rule {\n    direction = \"in\"\n    action    = \"accept\"\n    \
                 family    = \"IPv4\"\n    protocol  = \"tcp\"\n    \
                 destination_port_start = \"5432\"\n    destination_port_end   = \"5432\"\n  }\n\n  \
                 firewall_rule {\n    direction = \"out\"\n    action    = \"accept\"\n    \
                 family    = \"IPv4\"\n    comment   = \"Allow all outbound\"\n  }\n}\n",
            ),
            None,
        );
        // Server "app" with vpc_security_group_ids referencing both SGs.
        let mut server = make_result(
            "aws_instance",
            "app",
            "upcloud_server",
            Some(
                "resource \"upcloud_server\" \"app\" {\n  hostname = \"app\"\n  \
                 zone = \"fi-hel2\"\n  plan = \"2xCPU-4GB\"\n}\n",
            ),
            None,
        );
        // Attach both SGs to the server via source_hcl (simulates vpc_security_group_ids).
        server.source_hcl = Some(
            "resource \"aws_instance\" \"app\" {\n  \
             vpc_security_group_ids = [aws_security_group.web_sg.id, aws_security_group.db_sg.id]\n}\n"
                .to_string(),
        );

        let output = run_generate(&[fw_web, fw_db, server], "multi_sg");

        // Must have exactly one upcloud_firewall_rules resource.
        let fw_count = output.matches("resource \"upcloud_firewall_rules\"").count();
        assert_eq!(fw_count, 1, "must have exactly 1 upcloud_firewall_rules resource\n{output}");

        // Must contain rules from BOTH security groups.
        assert!(output.contains("destination_port_start = \"80\""), "must contain web_sg port-80 rule\n{output}");
        assert!(output.contains("destination_port_start = \"5432\""), "must contain db_sg port-5432 rule\n{output}");

        // Catch-all outbound rule must appear only once (deduplicated).
        let outbound_count = output.matches("Allow all outbound").count();
        assert_eq!(outbound_count, 1, "catch-all outbound rule must be deduplicated\n{output}");

        // server_id must be resolved (no TODO).
        assert!(!output.contains("<TODO>"), "no TODO must remain in merged firewall resource\n{output}");
        assert!(output.contains("upcloud_server.app.id"), "server_id must be resolved to 'app'\n{output}");
    }

    #[test]
    fn extract_fw_rule_blocks_returns_all_rules() {
        let hcl = r#"resource "upcloud_firewall_rules" "web" {
  server_id = upcloud_server.web.id

  firewall_rule {
    direction = "in"
    action    = "accept"
    family    = "IPv4"
  }

  firewall_rule {
    direction = "out"
    action    = "accept"
    family    = "IPv4"
  }
}
"#;
        let blocks = extract_fw_rule_blocks(hcl);
        assert_eq!(blocks.len(), 2, "should extract 2 rule blocks: {:?}", blocks);
        assert!(blocks[0].contains("direction = \"in\""), "{:?}", blocks[0]);
        assert!(blocks[1].contains("direction = \"out\""), "{:?}", blocks[1]);
    }

    #[test]
    fn build_merged_fw_hcl_deduplicates_identical_blocks() {
        let outbound = "  firewall_rule {\n    direction = \"out\"\n    action = \"accept\"\n  }\n";
        let inbound  = "  firewall_rule {\n    direction = \"in\"\n    action = \"accept\"\n  }\n";
        let blocks = vec![
            outbound.to_string(),
            inbound.to_string(),
            outbound.to_string(), // duplicate
        ];
        let merged = build_merged_fw_hcl("web_sg", "upcloud_server.web.id", None, &blocks);
        let outbound_count = merged.matches("direction = \"out\"").count();
        assert_eq!(outbound_count, 1, "duplicate outbound rule must be deduplicated\n{merged}");
        assert!(merged.contains("direction = \"in\""), "{merged}");
    }

    #[test]
    fn server_name_from_server_id_expr_plain() {
        assert_eq!(server_name_from_server_id_expr("upcloud_server.myserver.id"), "myserver");
    }

    #[test]
    fn server_name_from_server_id_expr_counted() {
        assert_eq!(
            server_name_from_server_id_expr("upcloud_server.myserver[count.index].id"),
            "myserver"
        );
    }

    // ── Zone substitution ─────────────────────────────────────────────────────

    #[test]
    fn zone_placeholder_replaced_in_output() {
        let server = make_result(
            "aws_instance",
            "web",
            "upcloud_server",
            Some(
                r#"resource "upcloud_server" "web" {
  hostname = "web"
  zone     = "__ZONE__"
  plan     = "1xCPU-1GB"

  template {
    storage = "Ubuntu Server 24.04 LTS (Noble Numbat)"
    size    = 25
  }

  network_interface {
    type = "public"
  }
}
"#,
            ),
            None,
        );

        let dir = std::env::temp_dir().join("upcloud_gen_test_zone_pl_waw1");
        std::fs::create_dir_all(&dir).unwrap();
        let mut log = vec![];
        generate_files(&[server], &[], &dir, None, "pl-waw1", &mut log).unwrap();
        let content = std::fs::read_to_string(dir.join("test.tf"))
            .expect("generate_files must produce test.tf");
        let _ = std::fs::remove_dir_all(&dir);

        assert!(
            content.contains("zone     = \"pl-waw1\""),
            "zone placeholder must be replaced with the requested zone\n{content}"
        );
        assert!(
            !content.contains("__ZONE__"),
            "no __ZONE__ placeholders must remain in output\n{content}"
        );
        // Output must still be valid HCL regardless of zone.
        hcl::from_str::<hcl::Body>(&content).expect("output must be valid HCL");
    }

    // ── Passthrough blocks (variable / output / locals) ───────────────────────

    #[test]
    fn variables_are_passed_through_unchanged() {
        use crate::terraform::parser::parse_tf_file;
        use crate::migration::mapper::map_resource;
        use crate::terraform::types::PassthroughBlock;

        let tf_source = r#"
variable "db_username" {
  description = "Database username"
  type        = string
}

variable "db_password" {
  description = "Database password"
  type        = string
  sensitive   = true
}

resource "aws_instance" "web" {
  instance_type = "t3.micro"
}
"#;
        let dir = std::env::temp_dir().join("upcloud_e2e_vars");
        std::fs::create_dir_all(&dir).unwrap();
        let tf_path = dir.join("main.tf");
        std::fs::write(&tf_path, tf_source).unwrap();

        let parsed = parse_tf_file(&tf_path).expect("should parse");
        assert_eq!(parsed.passthroughs.len(), 2, "should find 2 variable blocks");

        let results: Vec<MigrationResult> =
            parsed.resources.iter().map(|r| map_resource(r)).collect();
        let pts: Vec<PassthroughBlock> = parsed.passthroughs.clone();

        let out_dir = dir.join("out");
        let mut log = vec![];
        generate_files(&results, &pts, &out_dir, None, "fi-hel2", &mut log).unwrap();
        let output = std::fs::read_to_string(out_dir.join("main.tf"))
            .expect("generate_files must produce main.tf");
        let _ = std::fs::remove_dir_all(&dir);

        assert!(output.contains("variable \"db_username\""), "db_username variable must appear\n{output}");
        assert!(output.contains("variable \"db_password\""), "db_password variable must appear\n{output}");
        assert!(output.contains("sensitive   = true"), "variable body must be preserved\n{output}");
        // Non-AWS names must not get a NOTE comment.
        assert!(!output.contains("NOTE: Variable name 'db_username'"), "{output}");

        // Must still be valid HCL.
        hcl::from_str::<hcl::Body>(&output).expect("output with variables must be valid HCL");
    }

    #[test]
    fn aws_prefixed_variable_gets_review_comment_not_todo() {
        use crate::terraform::types::{PassthroughBlock, PassthroughKind};
        use std::path::PathBuf;

        let pt = PassthroughBlock {
            name: Some("aws_region".to_string()),
            raw_hcl: "variable \"aws_region\" {\n  default = \"us-east-1\"\n}".to_string(),
            source_file: PathBuf::from("test.tf"),
            kind: PassthroughKind::Variable,
        };

        let dir = std::env::temp_dir().join("upcloud_e2e_awsvar");
        std::fs::create_dir_all(&dir).unwrap();
        let out_dir = dir.join("out");
        let mut log = vec![];
        generate_files(&[], &[pt], &out_dir, None, "fi-hel2", &mut log).unwrap();
        let output = std::fs::read_to_string(out_dir.join("test.tf"))
            .expect("generate_files must produce test.tf");
        let _ = std::fs::remove_dir_all(&dir);

        // Must have the review comment (but NOT a <TODO:> in the name — that breaks HCL).
        assert!(output.contains("# NOTE: Variable name 'aws_region'"), "{output}");
        assert!(!output.contains("<TODO:"), "must not inject TODO into variable name\n{output}");
        // The variable block itself must still be present with the original name.
        assert!(output.contains("variable \"aws_region\""), "{output}");

        // Must be parseable.
        hcl::from_str::<hcl::Body>(&output).expect("output must be valid HCL");
    }

    #[test]
    fn locals_block_passed_through() {
        use crate::terraform::parser::parse_tf_file;
        use crate::migration::mapper::map_resource;
        use crate::terraform::types::PassthroughBlock;

        let tf_source = r#"
locals {
  common_tags = {
    Project = "webapp"
    Env     = "prod"
  }
}

resource "aws_instance" "web" {
  instance_type = "t3.micro"
}
"#;
        let dir = std::env::temp_dir().join("upcloud_e2e_locals");
        std::fs::create_dir_all(&dir).unwrap();
        let tf_path = dir.join("main.tf");
        std::fs::write(&tf_path, tf_source).unwrap();

        let parsed = parse_tf_file(&tf_path).expect("should parse");
        let pts: Vec<PassthroughBlock> = parsed.passthroughs.clone();
        let results: Vec<MigrationResult> =
            parsed.resources.iter().map(|r| map_resource(r)).collect();

        let out_dir = dir.join("out");
        let mut log = vec![];
        generate_files(&results, &pts, &out_dir, None, "fi-hel2", &mut log).unwrap();
        let output = std::fs::read_to_string(out_dir.join("main.tf"))
            .expect("generate_files must produce main.tf");
        let _ = std::fs::remove_dir_all(&dir);

        assert!(output.contains("locals {"), "locals block must appear\n{output}");
        assert!(output.contains("common_tags"), "{output}");
        hcl::from_str::<hcl::Body>(&output).expect("output must be valid HCL");
    }

    // ── rewrite_output_refs ───────────────────────────────────────────────────

    #[test]
    fn output_known_type_and_attr_rewritten() {
        let hcl = "output \"ip\" {\n  value = aws_instance.web.public_ip\n}";
        let rewritten = rewrite_output_refs(hcl);
        assert!(
            rewritten.contains("upcloud_server.web.network_interface[0].ip_address"),
            "public_ip should map to network_interface[0].ip_address\n{rewritten}"
        );
        assert!(!rewritten.contains("aws_instance"), "no AWS ref should remain\n{rewritten}");
    }

    #[test]
    fn output_known_type_unknown_attr_gets_todo() {
        let hcl = "output \"arn\" {\n  value = aws_instance.web.arn\n}";
        let rewritten = rewrite_output_refs(hcl);
        assert!(
            rewritten.contains("upcloud_server.web.<TODO: was .arn"),
            "unknown attr should get TODO with original attr name\n{rewritten}"
        );
    }

    #[test]
    fn output_unknown_type_gets_full_todo() {
        let hcl = "output \"x\" {\n  value = aws_cloudfront_distribution.cdn.domain_name\n}";
        let rewritten = rewrite_output_refs(hcl);
        assert!(
            rewritten.contains("<TODO: was aws_cloudfront_distribution.cdn.domain_name"),
            "unknown AWS type should get full TODO replacement\n{rewritten}"
        );
    }

    #[test]
    fn output_db_endpoint_rewritten() {
        let hcl = "output \"db_host\" {\n  value = aws_db_instance.main.endpoint\n}";
        let rewritten = rewrite_output_refs(hcl);
        assert!(
            rewritten.contains("upcloud_managed_database_postgresql.main.service_host"),
            "db endpoint should map to service_host\n{rewritten}"
        );
    }

    #[test]
    fn output_lb_dns_name_rewritten() {
        let hcl = "output \"lb\" {\n  value = aws_lb.main.dns_name\n}";
        let rewritten = rewrite_output_refs(hcl);
        assert!(
            rewritten.contains("upcloud_loadbalancer.main.dns_name"),
            "lb dns_name should map directly\n{rewritten}"
        );
    }

    #[test]
    fn output_id_passthrough_rewritten() {
        // .id is the same in both providers — just the type prefix changes
        let hcl = "output \"net_id\" {\n  value = aws_subnet.public.id\n}";
        let rewritten = rewrite_output_refs(hcl);
        assert!(
            rewritten.contains("upcloud_network.public.id"),
            ".id should rewrite type but keep .id\n{rewritten}"
        );
    }

    #[test]
    fn output_ref_inside_existing_todo_not_double_rewritten() {
        // refs already inside <TODO:> markers must not be rewritten again
        let hcl = "output \"x\" {\n  value = \"<TODO: was aws_instance.web.arn>\"\n}";
        let rewritten = rewrite_output_refs(hcl);
        // Should be unchanged — the aws_instance ref is inside a TODO marker
        assert_eq!(rewritten, hcl, "refs inside TODO markers must not be rewritten");
    }

    #[test]
    fn variable_block_rewritten_with_annotation() {
        use crate::terraform::types::{PassthroughBlock, PassthroughKind};
        use std::path::PathBuf;

        // instance_type name alone scores 1 (name keyword) — below threshold.
        // But adding default "t3.micro" gives +5 → total 6 (MEDIUM confidence).
        let var_hcl = "variable \"instance_type\" {\n  default = \"t3.micro\"\n}";
        let pt = PassthroughBlock {
            name: Some("instance_type".to_string()),
            raw_hcl: var_hcl.to_string(),
            source_file: PathBuf::from("vars.tf"),
            kind: PassthroughKind::Variable,
        };
        let dir = std::env::temp_dir().join("upcloud_rewrite_var_test2");
        std::fs::create_dir_all(&dir).unwrap();
        let out_dir = dir.join("out");
        let mut log = vec![];
        generate_files(&[], &[pt], &out_dir, None, "fi-hel2", &mut log).unwrap();
        let output = std::fs::read_to_string(out_dir.join("vars.tf")).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
        // t3.micro should be rewritten to the UpCloud plan equivalent
        assert!(
            output.contains("default = \"1xCPU-1GB\""),
            "instance type default must be rewritten to UpCloud plan\n{output}"
        );
        // An AUTO-CONVERTED annotation comment must be prepended
        assert!(
            output.contains("AUTO-CONVERTED"),
            "annotation comment must be present\n{output}"
        );
        // Other parts of the variable block must be preserved
        assert!(
            output.contains("variable \"instance_type\""),
            "variable name must be preserved\n{output}"
        );
    }

    #[test]
    fn output_block_refs_rewritten_in_generated_file() {
        use crate::terraform::parser::parse_tf_file;
        use crate::migration::mapper::map_resource;

        let tf_source = r#"
resource "aws_instance" "web" {
  instance_type = "t3.micro"
}

output "server_ip" {
  value = aws_instance.web.public_ip
}

output "server_id" {
  value = aws_instance.web.id
}
"#;
        let dir = std::env::temp_dir().join("upcloud_output_rewrite_e2e");
        std::fs::create_dir_all(&dir).unwrap();
        let tf_path = dir.join("main.tf");
        std::fs::write(&tf_path, tf_source).unwrap();

        let parsed = parse_tf_file(&tf_path).expect("should parse");
        let results: Vec<crate::migration::types::MigrationResult> =
            parsed.resources.iter().map(|r| map_resource(r)).collect();

        let out_dir = dir.join("out");
        let mut log = vec![];
        generate_files(&results, &parsed.passthroughs, &out_dir, None, "fi-hel1", &mut log).unwrap();
        let output = std::fs::read_to_string(out_dir.join("main.tf"))
            .expect("generate_files must produce main.tf");
        let _ = std::fs::remove_dir_all(&dir);

        assert!(
            output.contains("upcloud_server.web.network_interface[0].ip_address"),
            "server_ip output should be rewritten\n{output}"
        );
        assert!(
            output.contains("upcloud_server.web.id"),
            "server_id output should be rewritten\n{output}"
        );
        assert!(!output.contains("aws_instance.web"), "no AWS resource traversals in output\n{output}");
    }

    // ── Subnet group → database network resolution ────────────────────────────

    #[test]
    fn db_subnet_group_resolves_to_correct_network() {
        // Setup: two networks (public_a comes first alphabetically/order), data comes second.
        // Both postgres and valkey reference subnet group "main" which lists aws_subnet.data.
        // The generator must resolve their network UUIDs to upcloud_network.data, NOT public_a.

        let mut pg = make_result(
            "aws_db_instance",
            "postgres",
            "upcloud_managed_database_postgresql",
            Some(concat!(
                "resource \"upcloud_managed_database_postgresql\" \"postgres\" {\n",
                "  name = \"postgres-db\"\n",
                "  plan = \"1x2xCPU-4GB-50GB\"\n",
                "  zone = \"__ZONE__\"\n",
                "  network {\n",
                "    family = \"IPv4\"\n",
                "    type   = \"private\"\n",
                "    uuid   = \"<TODO: upcloud_network UUID subnet_group=main>\"\n",
                "  }\n",
                "}\n",
            )),
            None,
        );
        pg.source_hcl = Some(
            "resource \"aws_db_instance\" \"postgres\" {\n  db_subnet_group_name = aws_db_subnet_group.main.name\n}\n"
                .to_string(),
        );

        let mut valkey = make_result(
            "aws_elasticache_cluster",
            "redis",
            "upcloud_managed_database_valkey",
            Some(concat!(
                "resource \"upcloud_managed_database_valkey\" \"redis\" {\n",
                "  name = \"redis-cache\"\n",
                "  plan = \"1x1xCPU-2GB\"\n",
                "  zone = \"__ZONE__\"\n",
                "  network {\n",
                "    family = \"IPv4\"\n",
                "    type   = \"private\"\n",
                "    uuid   = \"<TODO: upcloud_network UUID subnet_group=main>\"\n",
                "  }\n",
                "}\n",
            )),
            None,
        );
        valkey.source_hcl = Some(
            "resource \"aws_elasticache_cluster\" \"redis\" {\n  subnet_group_name = aws_elasticache_subnet_group.main.name\n}\n"
                .to_string(),
        );

        // aws_db_subnet_group "main" with source HCL listing aws_subnet.data
        let mut sg = make_result(
            "aws_db_subnet_group",
            "main",
            "(subnet group → network block)",
            None,
            None,
        );
        sg.source_hcl = Some(
            "resource \"aws_db_subnet_group\" \"main\" {\n  subnet_ids = [aws_subnet.data.id]\n}\n"
                .to_string(),
        );

        // aws_elasticache_subnet_group "main" referencing same subnet
        let mut cache_sg = make_result(
            "aws_elasticache_subnet_group",
            "main",
            "(subnet group → network block)",
            None,
            None,
        );
        cache_sg.source_hcl = Some(
            "resource \"aws_elasticache_subnet_group\" \"main\" {\n  subnet_ids = [aws_subnet.data.id]\n}\n"
                .to_string(),
        );

        // Networks: public_a first (would be picked by generic fallback), data second.
        let public_a = make_result(
            "aws_subnet",
            "public_a",
            "upcloud_network",
            Some("resource \"upcloud_network\" \"public_a\" {\n  zone = \"__ZONE__\"\n}\n"),
            None,
        );
        let data = make_result(
            "aws_subnet",
            "data",
            "upcloud_network",
            Some("resource \"upcloud_network\" \"data\" {\n  zone = \"__ZONE__\"\n}\n"),
            None,
        );

        let output = run_generate(&[pg, valkey, sg, cache_sg, public_a, data], "db_subnet_group");

        assert!(
            output.contains("uuid   = upcloud_network.data.id"),
            "postgres network uuid must resolve to upcloud_network.data (not public_a)\n{output}"
        );
        assert!(
            !output.contains("upcloud_network.public_a.id"),
            "public_a must not appear in database network uuid when data subnet is known\n{output}"
        );
        assert!(
            !output.contains("subnet_group="),
            "all subnet_group placeholders must be resolved\n{output}"
        );
    }

    // ── Router name fix (vpc → vpc_router) ────────────────────────────────────

    #[test]
    fn rewrite_output_refs_appends_router_suffix_for_vpc() {
        let input = r#"output "vpc_id" {
  value = aws_vpc.main.id
}"#;
        let result = rewrite_output_refs(input);
        assert!(
            result.contains("upcloud_router.main_router.id"),
            "aws_vpc.main.id must map to upcloud_router.main_router.id\n{result}"
        );
        assert!(!result.contains("upcloud_router.main.id"), "must not use bare 'main'\n{result}");
    }

    // ── DB network [0] index for counted network ───────────────────────────────

    #[test]
    fn subnet_group_resolution_uses_index_for_counted_network() {
        // When the target network has count=2, the DB network uuid must use [0].
        let pg = make_result(
            "aws_db_instance",
            "main",
            "upcloud_managed_database_postgresql",
            Some("resource \"upcloud_managed_database_postgresql\" \"main\" {\n  \
                  network {\n    uuid = \"<TODO: upcloud_network UUID subnet_group=main>\"\n  }\n}\n"),
            None,
        );
        // Subnet group "main" maps to subnet "database"
        let sg_result = make_result(
            "aws_db_subnet_group",
            "main",
            "upcloud_db_subnet_group_placeholder",
            None,
            None,
        );
        // Network "database" has count (2 instances) — note count in HCL
        let db_net = make_result(
            "aws_subnet",
            "database",
            "upcloud_network",
            Some("resource \"upcloud_network\" \"database\" {\n  count = 2\n  zone = \"__ZONE__\"\n}\n"),
            None,
        );

        let mut results = vec![pg, db_net];
        // Add a fake subnet_group result so source_hcl maps it
        let mut sg_with_hcl = sg_result;
        sg_with_hcl.source_hcl = Some(
            r#"resource "aws_db_subnet_group" "main" {
  subnet_ids = [aws_subnet.database[0].id, aws_subnet.database[1].id]
}"#.to_string()
        );
        results.push(sg_with_hcl);

        let out_dir = std::env::temp_dir().join("upcloud_gen_db_idx_test");
        let mut log = vec![];
        generate_files(&results, &[], &out_dir, None, "fi-hel1", &mut log).unwrap();
        let output = std::fs::read_to_string(out_dir.join("test.tf")).unwrap_or_default();
        let _ = std::fs::remove_dir_all(&out_dir);

        assert!(
            output.contains("upcloud_network.database[0].id"),
            "counted database network must be indexed with [0]\n{output}\nlog: {:?}", log
        );
    }

    // ── Storage injection ──────────────────────────────────────────────────────

    #[test]
    fn volume_attachment_injects_storage_devices_into_server() {
        use crate::migration::providers::aws::{compute::map_instance, storage::map_ebs_volume};
        use crate::terraform::types::TerraformResource;
        use std::path::PathBuf;

        let make = |rt: &str, name: &str, attrs: &[(&str, &str)]| TerraformResource {
            resource_type: rt.to_string(),
            name: name.to_string(),
            attributes: attrs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect(),
            source_file: PathBuf::from("main.tf"),
            raw_hcl: String::new(),
        };

        // web server with count
        let web_res = make("aws_instance", "web", &[
            ("instance_type", "t3.micro"),
            ("count", "2"),
            ("subnet_id", "aws_subnet.private.id"),
        ]);
        let mut web_result = map_instance(&web_res);
        web_result.source_hcl = Some(web_result.upcloud_hcl.clone().unwrap());

        // EBS volume with count
        let vol_res = make("aws_ebs_volume", "data", &[("type", "gp3"), ("size", "50"), ("count", "2")]);
        let vol_result = map_ebs_volume(&vol_res);

        // Attachment
        let att_res = make("aws_volume_attachment", "data", &[
            ("volume_id", "aws_ebs_volume.data[count.index].id"),
            ("instance_id", "aws_instance.web[count.index].id"),
        ]);
        use crate::migration::providers::aws::storage::map_volume_attachment;
        let att_result = map_volume_attachment(&att_res);

        let out_dir = std::env::temp_dir().join("upcloud_storage_inject_test");
        let mut log = vec![];
        generate_files(&[web_result, vol_result, att_result], &[], &out_dir, None, "fi-hel1", &mut log).unwrap();
        let output = std::fs::read_to_string(out_dir.join("main.tf")).unwrap_or_default();
        let _ = std::fs::remove_dir_all(&out_dir);

        assert!(
            output.contains("storage_devices {"),
            "storage_devices block must be injected into server\n{output}\nlog: {:?}", log
        );
        assert!(
            output.contains("upcloud_storage.data[count.index].id"),
            "storage ref must use count.index\n{output}"
        );
        assert!(
            !output.contains("__STORAGE_END_"),
            "sentinel must be removed after injection\n{output}"
        );
    }
}

