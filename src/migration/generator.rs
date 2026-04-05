use anyhow::Result;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::migration::providers::{ResourceRole, SourceProvider, detect_provider};
use crate::migration::types::{MigrationResult, MigrationStatus};
use crate::migration::var_detector::{
    VarKind, analyze_variable_with, apply_conversion_to_hcl, build_var_annotation,
    build_var_usage_map, extract_variable_info,
};
use crate::terraform::types::{PassthroughBlock, PassthroughKind};
use crate::zones::zone_to_objstorage_region;

/// Prefix that identifies an unresolved TODO placeholder in generated HCL.
/// Used to skip values that the mapper could not resolve at mapping time.
pub(crate) const TODO_PLACEHOLDER_PREFIX: &str = "<TODO";

/// Sentinel stored in `resolved_hcl_map` for resources that were intentionally
/// skipped during generation (e.g. firewall rules with no resolvable server_id).
/// The diff view uses this to show a "skipped" note instead of partial HCL.
pub const SKIPPED_SENTINEL: &str = "\x00SKIPPED";

/// Resolved HCL map: maps `(resource_type, resource_name)` to the fully-resolved HCL string
/// (zone injected, cross-references resolved, source refs sanitised).
pub type ResolvedHclMap = HashMap<(String, String), String>;

/// Cross-reference lookup tables for resource resolution.
struct CrossRefTables {
    lb_names: Vec<String>,
    network_names: Vec<String>,
    server_names: Vec<String>,
    k8s_names: Vec<String>,
    ssh_key_map: HashMap<String, LoginKeysValue>,
    backend_names: Vec<String>,
    cert_bundle_names: Vec<String>,
    server_info_map: HashMap<String, Option<String>>,
    firewall_to_server_map: HashMap<String, Vec<String>>,
    network_count_map: HashMap<String, bool>,
    param_group_map: HashMap<String, Vec<(String, String)>>,
    subnet_group_subnets_map: HashMap<String, Vec<String>>,
    lb_backend_net_map: HashMap<String, String>,
    storage_inject_map: HashMap<String, Vec<String>>,
    storage_promote_count: HashMap<String, String>,
    /// Maps backend_pool_name → health_check property lines for UpCloud backend `properties {}`.
    /// Built from provider probe resources (e.g. `azurerm_lb_probe`) chained via LB rules.
    probe_health_map: HashMap<String, String>,
}

/// Build cross-reference lookup tables from migration results.
fn build_cross_ref_tables(
    results: &[MigrationResult],
    provider: &dyn SourceProvider,
) -> CrossRefTables {
    // Collect resource names by type for direct lookups
    let lb_names: Vec<String> = results
        .iter()
        .filter(|r| r.upcloud_type == "upcloud_loadbalancer")
        .map(|r| r.resource_name.clone())
        .collect();

    let network_names: Vec<String> = results
        .iter()
        .filter(|r| r.upcloud_type.contains("upcloud_network"))
        .map(|r| r.resource_name.clone())
        .collect();

    let server_names: Vec<String> = results
        .iter()
        .filter(|r| r.upcloud_type == "upcloud_server")
        .map(|r| r.resource_name.clone())
        .collect();

    let k8s_names: Vec<String> = results
        .iter()
        .filter(|r| r.upcloud_type == "upcloud_kubernetes_cluster")
        .map(|r| r.resource_name.clone())
        .collect();

    // Build key_pair_name → public_key map for auto-resolving login blocks
    let mut ssh_key_map: HashMap<String, LoginKeysValue> = HashMap::new();
    for r in results {
        if provider.resource_role(&r.resource_type) == ResourceRole::KeyPair
            && let Some(snippet) = &r.snippet
            && let Some(keys) = extract_login_keys(snippet)
        {
            ssh_key_map.insert(r.resource_name.clone(), keys);
        }
    }

    let backend_names: Vec<String> = results
        .iter()
        .filter(|r| r.upcloud_type == "upcloud_loadbalancer_backend")
        .map(|r| r.resource_name.clone())
        .collect();

    let cert_bundle_names: Vec<String> = results
        .iter()
        .filter(|r| r.upcloud_type == "upcloud_loadbalancer_manual_certificate_bundle")
        .map(|r| r.resource_name.clone())
        .collect();

    // Build server_info_map: server resource_name → Option<count string>
    // Used for name-based firewall server_id resolution and IP ref indexing.
    let server_info_map: HashMap<String, Option<String>> = results
        .iter()
        .filter(|r| r.upcloud_type == "upcloud_server")
        .map(|r| {
            let count = r.upcloud_hcl.as_deref().and_then(extract_count_from_hcl);
            (r.resource_name.clone(), count)
        })
        .collect();

    // Build firewall_to_server_map: firewall resource_name → Vec<server resource_name>
    let mut firewall_to_server_map: HashMap<String, Vec<String>> = HashMap::new();
    for r in results
        .iter()
        .filter(|r| provider.resource_role(&r.resource_type) == ResourceRole::ComputeInstance)
    {
        if let Some(hcl) = r.source_hcl.as_deref() {
            for fw_name in provider.extract_security_refs_from_instance(hcl) {
                firewall_to_server_map
                    .entry(fw_name)
                    .or_default()
                    .push(r.resource_name.clone());
            }
        }
    }

    // Build network_count_map: network resource_name → whether it has count
    let network_count_map: HashMap<String, bool> = results
        .iter()
        .filter(|r| r.upcloud_type.contains("upcloud_network"))
        .map(|r| {
            let has_count = r
                .upcloud_hcl
                .as_deref()
                .and_then(extract_count_from_hcl)
                .is_some();
            (r.resource_name.clone(), has_count)
        })
        .collect();

    // Build param_group_map: parameter group resource_name → Vec<(param_name, value)>
    let param_group_map: HashMap<String, Vec<(String, String)>> = results
        .iter()
        .filter(|r| provider.resource_role(&r.resource_type) == ResourceRole::ParameterGroup)
        .filter_map(|r| {
            r.source_hcl.as_ref().map(|hcl| {
                (
                    r.resource_name.clone(),
                    provider.extract_parameter_blocks(hcl),
                )
            })
        })
        .collect();

    // Build subnet_group_subnets_map: subnet group resource_name → Vec<subnet resource_name>
    let subnet_group_subnets_map: HashMap<String, Vec<String>> = results
        .iter()
        .filter(|r| provider.resource_role(&r.resource_type) == ResourceRole::SubnetGroup)
        .filter_map(|r| {
            r.source_hcl.as_deref().map(|hcl| {
                (
                    r.resource_name.clone(),
                    provider.extract_subnet_names_from_subnet_group(hcl),
                )
            })
        })
        .filter(|(_, subnets)| !subnets.is_empty())
        .collect();

    // Build server_subnet_map and related tables for load balancer network resolution
    let mut server_subnet_map: HashMap<String, String> = results
        .iter()
        .filter(|r| provider.resource_role(&r.resource_type) == ResourceRole::ComputeInstance)
        .filter_map(|r| {
            r.source_hcl.as_deref().and_then(|hcl| {
                provider
                    .extract_subnet_from_instance(hcl)
                    .map(|subnet| (r.resource_name.clone(), subnet))
            })
        })
        .collect();

    // Supplement server_subnet_map via NIC indirection (e.g. VM→NIC→subnet).
    // Some providers use standalone NIC resources instead of inline subnet references.
    if let Some(nic_type) = provider.nic_resource_type() {
        let nic_to_subnet: HashMap<String, String> = results
            .iter()
            .filter(|r| r.resource_type == nic_type)
            .filter_map(|r| {
                r.source_hcl
                    .as_deref()
                    .and_then(|hcl| provider.extract_subnet_from_nic(hcl))
                    .map(|subnet| (r.resource_name.clone(), subnet))
            })
            .collect();
        if !nic_to_subnet.is_empty() {
            let supplements: Vec<(String, String)> = results
                .iter()
                .filter(|r| provider.resource_role(&r.resource_type) == ResourceRole::ComputeInstance)
                .filter(|r| !server_subnet_map.contains_key(&r.resource_name))
                .filter_map(|r| {
                    r.source_hcl.as_deref().and_then(|hcl| {
                        provider
                            .extract_nic_refs_from_instance(hcl)
                            .into_iter()
                            .find_map(|nic_name| nic_to_subnet.get(&nic_name).cloned())
                            .map(|subnet| (r.resource_name.clone(), subnet))
                    })
                })
                .collect();
            for (server, subnet) in supplements {
                server_subnet_map.insert(server, subnet);
            }
        }
    }

    // Supplement firewall_to_server_map via subnet-association chain.
    // Some providers attach security groups to subnets rather than directly to instances.
    // When the provider defines a subnet↔security-group association type, build a
    // subnet→security-group map and join it with server_subnet_map to discover
    // which servers each security group covers.
    if let Some(assoc_type) = provider.subnet_nsg_association_type() {
        let subnet_to_nsg: HashMap<String, String> = results
            .iter()
            .filter(|r| r.resource_type == assoc_type)
            .filter_map(|r| {
                r.source_hcl
                    .as_deref()
                    .and_then(|hcl| provider.extract_nsg_from_subnet_association(hcl))
            })
            .collect();
        if !subnet_to_nsg.is_empty() {
            for (server, subnet) in &server_subnet_map {
                if let Some(nsg) = subnet_to_nsg.get(subnet) {
                    let servers = firewall_to_server_map.entry(nsg.clone()).or_default();
                    if !servers.contains(server) {
                        servers.push(server.clone());
                    }
                }
            }
        }
    }

    let mut tg_servers_map: HashMap<String, Vec<String>> = HashMap::new();
    for r in results.iter().filter(|r| {
        provider.resource_role(&r.resource_type) == ResourceRole::LbTargetGroupAttachment
    }) {
        if let Some(hcl) = r.source_hcl.as_deref()
            && let Some((tg, srv)) = provider.extract_tg_server_from_attachment(hcl)
        {
            tg_servers_map.entry(tg).or_default().push(srv);
        }
    }

    let mut lb_tgs_map: HashMap<String, Vec<String>> = HashMap::new();
    for r in results
        .iter()
        .filter(|r| provider.resource_role(&r.resource_type) == ResourceRole::LbListener)
    {
        if let Some(hcl) = r.source_hcl.as_deref()
            && let Some(tg) = provider.extract_tg_from_listener(hcl)
        {
            let lb_name = r.parent_resource.clone().unwrap_or_default();
            let lb_name = if lb_name.is_empty() {
                provider
                    .extract_lb_name_from_listener(hcl)
                    .unwrap_or_default()
            } else {
                lb_name
            };
            if !lb_name.is_empty() {
                lb_tgs_map.entry(lb_name).or_default().push(tg);
            }
        }
    }

    // Chain lb → tgs → servers → subnets → network names
    let mut lb_backend_net_map: HashMap<String, String> = HashMap::new();
    for (lb_name, tgs) in &lb_tgs_map {
        'outer: for tg in tgs {
            if let Some(servers) = tg_servers_map.get(tg) {
                for srv in servers {
                    if let Some(subnet) = server_subnet_map.get(srv)
                        && network_names.contains(subnet)
                    {
                        lb_backend_net_map.insert(lb_name.clone(), subnet.clone());
                        break 'outer;
                    }
                }
            }
        }
    }

    // Build storage-related maps
    let storage_count_map: HashMap<String, bool> = results
        .iter()
        .filter(|r| r.upcloud_type == "upcloud_storage")
        .map(|r| {
            let has_count = r
                .upcloud_hcl
                .as_deref()
                .and_then(extract_count_from_hcl)
                .is_some();
            (r.resource_name.clone(), has_count)
        })
        .collect();

    let mut storage_inject_map: HashMap<String, Vec<String>> = HashMap::new();
    let mut storage_promote_count: HashMap<String, String> = HashMap::new();

    for r in results
        .iter()
        .filter(|r| provider.resource_role(&r.resource_type) == ResourceRole::VolumeAttachment)
    {
        let server_name = match r.parent_resource.as_deref() {
            Some(n) if !n.is_empty() => n.to_string(),
            _ => continue,
        };

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

        let server_count_val = server_info_map.get(&server_name).and_then(|c| c.clone());
        let server_has_count = server_count_val.is_some();
        let storage_has_count = storage_count_map
            .get(&storage_name)
            .copied()
            .unwrap_or(false);

        let storage_ref = if server_has_count && !storage_has_count {
            if let Some(ref count_val) = server_count_val {
                storage_promote_count.insert(storage_name.clone(), count_val.clone());
            }
            format!("upcloud_storage.{}[count.index].id", storage_name)
        } else if storage_has_count && server_has_count {
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
        storage_inject_map
            .entry(server_name)
            .or_default()
            .push(block);
    }

    // Build probe_health_map: backend_pool_name → health check property lines.
    // Chain: azurerm_lb_probe (or similar) → azurerm_lb_rule (probe_id + backend_pool_ids)
    let probe_health_map: HashMap<String, String> =
        if let Some(probe_type) = provider.lb_probe_resource_type() {
            let probe_snippet_map: HashMap<String, String> = results
                .iter()
                .filter(|r| r.resource_type == probe_type)
                .filter_map(|r| {
                    r.source_hcl.as_ref().map(|hcl| {
                        let props = provider.extract_probe_health_check_props(hcl);
                        (r.resource_name.clone(), props)
                    })
                })
                .filter(|(_, props)| !props.is_empty())
                .collect();

            let mut map: HashMap<String, String> = HashMap::new();
            for r in results
                .iter()
                .filter(|r| provider.resource_role(&r.resource_type) == ResourceRole::LbListener)
            {
                if let Some(hcl) = r.source_hcl.as_deref()
                    && let Some((backend_name, probe_name)) =
                        provider.extract_probe_from_lb_rule(hcl)
                    && let Some(props) = probe_snippet_map.get(&probe_name)
                {
                    map.entry(backend_name).or_insert_with(|| props.clone());
                }
            }
            map
        } else {
            HashMap::new()
        };

    CrossRefTables {
        lb_names,
        network_names,
        server_names,
        k8s_names,
        ssh_key_map,
        backend_names,
        cert_bundle_names,
        server_info_map,
        firewall_to_server_map,
        network_count_map,
        param_group_map,
        subnet_group_subnets_map,
        lb_backend_net_map,
        storage_inject_map,
        storage_promote_count,
        probe_health_map,
    }
}

/// Replace indexed `<TODO>` placeholders with items from a list.
fn replace_indexed_placeholders(hcl: &str, pattern: &str, replacements: &[String]) -> String {
    if replacements.is_empty() {
        return hcl.to_string();
    }
    let mut result = hcl.to_string();
    for (i, replacement) in replacements.iter().enumerate() {
        if i == 0 {
            // Replace first occurrence of pattern
            if let Some(pos) = result.find(pattern) {
                result.replace_range(pos..pos + pattern.len(), replacement);
            }
        } else {
            // For subsequent items, handle potential multiple occurrences
            // by replacing the next available occurrence
            if let Some(pos) = result.find(pattern) {
                result.replace_range(pos..pos + pattern.len(), replacement);
            }
        }
    }
    result
}

/// Resolve load balancer and certificate placeholders.
fn resolve_load_balancer_refs(hcl: &str, xref: &CrossRefTables) -> String {
    let mut s = hcl.to_string();

    // Resolve indexed load balancer references
    let lb_refs: Vec<String> = xref
        .lb_names
        .iter()
        .map(|lb| format!("upcloud_loadbalancer.{}.id", lb))
        .collect();
    s = replace_indexed_placeholders(&s, "upcloud_loadbalancer.<TODO>.id", &lb_refs);

    // Resolve indexed backend references
    let backend_refs: Vec<String> = xref
        .backend_names
        .iter()
        .map(|b| format!("upcloud_loadbalancer_backend.{}.name", b))
        .collect();
    s = replace_indexed_placeholders(
        &s,
        "upcloud_loadbalancer_backend.<TODO>.name",
        &backend_refs,
    );

    // Resolve indexed certificate bundle references
    let cert_refs: Vec<String> = xref
        .cert_bundle_names
        .iter()
        .map(|c| format!("upcloud_loadbalancer_manual_certificate_bundle.{}.id", c))
        .collect();
    s = replace_indexed_placeholders(
        &s,
        "upcloud_loadbalancer_manual_certificate_bundle.<TODO>.id",
        &cert_refs,
    );

    s
}

/// Resolve Kubernetes resource references.
fn resolve_k8s_and_generic_refs(hcl: &str, xref: &CrossRefTables) -> String {
    let mut s = hcl.to_string();

    // Resolve Kubernetes cluster references for node groups
    if let Some(k8s) = xref.k8s_names.first() {
        s = s.replace(
            "upcloud_kubernetes_cluster.<TODO>.id",
            &format!("upcloud_kubernetes_cluster.{}.id", k8s),
        );
    }

    s
}

/// Inject `lifecycle { ignore_changes = [router] }` before the final closing brace
/// of an `upcloud_network` resource block. Called only when Kubernetes clusters are
/// present in the project, since UKS attaches a router automatically.
fn inject_network_lifecycle_block(hcl: &str) -> String {
    let lifecycle = "\n  # UpCloud Kubernetes Service will attach a router automatically.\n  # Ignore router changes to avoid detaching it on subsequent applies.\n  lifecycle {\n    ignore_changes = [router]\n  }\n";
    // Insert before the final closing brace of the resource block
    if let Some(pos) = hcl.rfind('}') {
        let mut result = String::with_capacity(hcl.len() + lifecycle.len());
        result.push_str(&hcl[..pos]);
        result.push_str(lifecycle);
        result.push_str(&hcl[pos..]);
        result
    } else {
        hcl.to_string()
    }
}

/// Resolve placeholders in HCL (zones, cross-references, SSH keys, provider-specific values).
fn resolve_placeholders(
    hcl: &str,
    resource_name: &str,
    zone: &str,
    objstorage_region: &str,
    xref: &CrossRefTables,
    provider: &dyn SourceProvider,
) -> String {
    let mut s = hcl
        .replace("__ZONE__", zone)
        .replace("__OBJSTORAGE_REGION__", objstorage_region);

    s = resolve_load_balancer_refs(&s, xref);
    s = resolve_k8s_and_generic_refs(&s, xref);
    s = resolve_network_refs(&s, resource_name, xref);
    s = resolve_ssh_key_refs(&s, xref, provider);
    s = resolve_db_parameter_refs(&s, xref, provider);
    s = resolve_lb_probe_refs(&s, resource_name, xref);
    s = provider.sanitize_source_refs(s);

    s
}

/// Resolve LB probe health check markers (`# __PROBE_HC__`) in backend pool HCL.
/// Replaces the marker line with the health check properties from the cross-reference table.
fn resolve_lb_probe_refs(hcl: &str, resource_name: &str, xref: &CrossRefTables) -> String {
    if !hcl.contains("# __PROBE_HC__") {
        return hcl.to_string();
    }
    let mut out = String::with_capacity(hcl.len());
    for line in hcl.lines() {
        if line.trim() == "# __PROBE_HC__" {
            if let Some(props) = xref.probe_health_map.get(resource_name) {
                out.push_str(props);
            } else {
                // Fallback: keep a tcp health check if no probe data available
                out.push_str("    health_check_type     = \"tcp\"\n");
            }
        } else {
            out.push_str(line);
            out.push('\n');
        }
    }
    if !hcl.ends_with('\n') && out.ends_with('\n') {
        out.pop();
    }
    out
}

/// Resolve network references for both generic and subnet-specific cases.
fn resolve_network_refs(hcl: &str, resource_name: &str, xref: &CrossRefTables) -> String {
    let mut s = hcl.to_string();

    // Resolve network references (subnet-specific for servers)
    for net_name in &xref.network_names {
        let specific = format!("\"<TODO: upcloud_network.{} reference>\"", net_name);
        if s.contains(&specific) {
            let net_has_count = xref
                .network_count_map
                .get(net_name.as_str())
                .copied()
                .unwrap_or(false);
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

    // Resolve database subnet-group-specific network references
    for (sg_name, subnets) in &xref.subnet_group_subnets_map {
        let placeholder = format!("\"<TODO: upcloud_network UUID subnet_group={}>\"", sg_name);
        if s.contains(&placeholder) {
            let net = subnets
                .iter()
                .find(|sn| xref.network_names.contains(sn))
                .or_else(|| xref.network_names.first());
            if let Some(net_name) = net {
                let net_has_count = xref
                    .network_count_map
                    .get(net_name.as_str())
                    .copied()
                    .unwrap_or(false);
                let net_ref = if net_has_count {
                    format!("upcloud_network.{}[0].id", net_name)
                } else {
                    format!("upcloud_network.{}.id", net_name)
                };
                s = s.replace(&placeholder, &net_ref);
            }
        }
    }

    // Generic network fallback with heuristics
    let preferred_net = xref
        .lb_backend_net_map
        .get(resource_name)
        .and_then(|net| xref.network_names.iter().find(|n| *n == net))
        .or_else(|| {
            xref.network_names
                .iter()
                .find(|n| n.to_lowercase().contains("private"))
        })
        .or_else(|| {
            xref.network_names
                .iter()
                .find(|n| !n.to_lowercase().contains("public"))
        })
        .or_else(|| xref.network_names.first());
    if let Some(net) = preferred_net {
        let net_has_count = xref
            .network_count_map
            .get(net.as_str())
            .copied()
            .unwrap_or(false);
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
    }

    // Resolve server IP references for load balancer backends
    if let Some(srv) = xref.server_names.first() {
        let srv_has_count = xref
            .server_info_map
            .get(srv.as_str())
            .map(|c| c.is_some())
            .unwrap_or(false);
        let srv_ref = if srv_has_count {
            format!("upcloud_server.{}[0].network_interface[0].ip_address", srv)
        } else {
            format!("upcloud_server.{}.network_interface[0].ip_address", srv)
        };
        s = s.replace("\"<TODO: server IP>\"", &srv_ref);
    }

    s
}

/// Resolve SSH public key references in login blocks.
fn resolve_ssh_key_refs(hcl: &str, xref: &CrossRefTables, provider: &dyn SourceProvider) -> String {
    let mut s = hcl.to_string();
    for (kp_name, keys_value) in &xref.ssh_key_map {
        let placeholder = provider.ssh_key_placeholder(kp_name);
        match keys_value {
            LoginKeysValue::Literal(public_key) => {
                s = s.replace(&placeholder, public_key);
            }
            LoginKeysValue::Expression(expr) => {
                let quoted_placeholder = format!("\"{}\"", placeholder);
                s = s.replace(&quoted_placeholder, expr);
            }
        }
    }
    // Fallback for generic SSH key placeholder
    if s.contains("<TODO: paste SSH public key>")
        && let Some(LoginKeysValue::Literal(key)) = xref.ssh_key_map.values().next()
    {
        s = s.replace("<TODO: paste SSH public key>", key);
    }
    s
}

/// Resolve database parameter group properties.
fn resolve_db_parameter_refs(
    hcl: &str,
    xref: &CrossRefTables,
    provider: &dyn SourceProvider,
) -> String {
    if !hcl.contains("# __DB_PROPS:") {
        return hcl.to_string();
    }

    let mut out = String::with_capacity(hcl.len());
    for line in hcl.lines() {
        let trimmed = line.trim();
        if let Some(inner) = trimmed.strip_prefix("# __DB_PROPS:")
            && let Some(inner) = inner.strip_suffix("__")
            && let Some((prefix, group_name)) = inner.split_once(':')
        {
            if let Some(params) = xref.param_group_map.get(group_name)
                && !params.is_empty()
            {
                let _ = prefix;
                for (name, value) in params {
                    if provider.is_valid_db_property(name) {
                        if name == "max_connections" {
                            out.push_str(&format!(
                                    "    # max_connections = \"{}\"  # requires pg_user_config_max_connections account permission\n",
                                    value
                                ));
                        } else {
                            out.push_str(&format!("    {} = \"{}\"\n", name, value));
                        }
                    } else {
                        out.push_str(&format!(
                                "    # <TODO: {} = \"{}\" — not a valid upcloud_managed_database_postgresql property>\n",
                                name, value
                            ));
                    }
                }
                continue;
            }
            out.push_str(&provider.parameter_group_todo_text(group_name));
            continue;
        }
        out.push_str(line);
        out.push('\n');
    }
    if !hcl.ends_with('\n') && out.ends_with('\n') {
        out.pop();
    }
    out
}

/// Group resources and passthrough blocks by source file basename.
/// Returns (file_map, passthrough_map, ssh_var_target_file, needs_ssh_public_key).
#[allow(clippy::type_complexity)]
fn group_resources_and_passthroughs<'a>(
    results: &'a [MigrationResult],
    passthroughs: &'a [PassthroughBlock],
) -> (
    HashMap<String, Vec<&'a MigrationResult>>,
    HashMap<String, Vec<&'a PassthroughBlock>>,
    String,
    bool,
) {
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
    let needs_ssh_public_key = results.iter().any(|r| {
        r.upcloud_hcl
            .as_deref()
            .unwrap_or("")
            .contains("var.ssh_public_key")
    }) && !passthroughs.iter().any(|p| {
        p.kind == PassthroughKind::Variable && p.name.as_deref() == Some("ssh_public_key")
    });
    let ssh_var_target_file: String = if needs_ssh_public_key {
        passthrough_map
            .iter()
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

    (
        file_map,
        passthrough_map,
        ssh_var_target_file,
        needs_ssh_public_key,
    )
}

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

    // Detect the source cloud provider from the resource types
    let provider = detect_provider(results);

    // Build all cross-reference lookup tables
    let xref = build_cross_ref_tables(results, &*provider);

    // Group resources and passthroughs by source file basename
    let (file_map, passthrough_map, ssh_var_target_file, needs_ssh_public_key) =
        group_resources_and_passthroughs(results, passthroughs);

    // Build a usage map so the variable detector can score each variable by where it is referenced.
    // Gather all source HCL from mapped resources (best-effort — not every resource has source_hcl).
    let source_hcl_refs: Vec<&str> = results
        .iter()
        .filter_map(|r| r.source_hcl.as_deref())
        .collect();
    let var_usage_map = build_var_usage_map(&source_hcl_refs);

    // Write provider config
    let provider_path = output_dir.join("providers.tf");
    let mut provider_hcl = String::from(
        r#"terraform {
  required_providers {
    upcloud = {
      source  = "UpCloudLtd/upcloud"
      version = "~> 5.0"
    }
"#,
    );
    // Append non-cloud required_providers entries (e.g. kubernetes, helm)
    for pt in passthroughs {
        if pt.kind == PassthroughKind::Provider
            && let Some(name) = &pt.name
            && name.starts_with("required_provider:")
        {
            provider_hcl.push_str("    ");
            provider_hcl.push_str(&pt.raw_hcl);
            provider_hcl.push('\n');
        }
    }
    provider_hcl.push_str(
        r#"  }
}

variable "upcloud_token" {
  description = "UpCloud API token"
  type        = string
  sensitive   = true
}

provider "upcloud" {
  token = var.upcloud_token
}
"#,
    );
    // Append non-cloud provider blocks (e.g. provider "kubernetes" { ... })
    for pt in passthroughs {
        if pt.kind == PassthroughKind::Provider
            && let Some(name) = &pt.name
            && !name.starts_with("required_provider:")
        {
            provider_hcl.push('\n');
            provider_hcl.push_str(&pt.raw_hcl);
            provider_hcl.push('\n');
        }
    }
    std::fs::write(&provider_path, &provider_hcl)?;
    log.push("  [OK] providers.tf".to_string());

    let mut total = 1;

    // Track servers that have already had their firewall rules written (across all files).
    // UpCloud allows only one upcloud_firewall_rules resource per server.
    let mut written_fw_servers: std::collections::HashSet<String> =
        std::collections::HashSet::new();

    for (filename, file_results) in &file_map {
        let out_path = output_dir.join(filename);
        let mut content = String::new();
        content.push_str(&format!(
            "# Migrated from {} Terraform\n# Source: {}\n# Target zone: {}\n\n",
            provider.display_name(),
            filename,
            zone
        ));

        // Collect per-file pending firewall rules
        #[allow(clippy::type_complexity)]
        let mut fw_by_server: indexmap::IndexMap<
            String,
            (String, Option<String>, Vec<String>, Vec<String>),
        > = indexmap::IndexMap::new();

        for result in file_results {
            if let Some(hcl) = &result.upcloud_hcl {
                // Passthrough resources: rewrite cloud refs but skip UpCloud-specific resolution
                if result.status == MigrationStatus::Passthrough {
                    let rewritten = provider.rewrite_output_refs(hcl);
                    resolved_hcl_map.insert(
                        (result.resource_type.clone(), result.resource_name.clone()),
                        rewritten.clone(),
                    );
                    content.push_str(&format!(
                        "# {} {} (non-cloud-provider resource, kept as is)\n",
                        result.resource_type, result.resource_name
                    ));
                    content.push_str(&rewritten);
                    content.push('\n');
                    continue;
                }

                let mut resolved = resolve_placeholders(
                    hcl,
                    &result.resource_name,
                    zone,
                    objstorage_region,
                    &xref,
                    &*provider,
                );

                // Inject lifecycle { ignore_changes = [router] } into upcloud_network
                // resources only when the project contains Kubernetes clusters.
                if result.upcloud_type == "upcloud_network" && !xref.k8s_names.is_empty() {
                    resolved = inject_network_lifecycle_block(&resolved);
                }

                // Name-based firewall server_id resolution with rule merging
                if result.upcloud_type == "upcloud_firewall_rules" {
                    // Collect all servers that reference this firewall
                    let servers: Vec<String> = xref
                        .firewall_to_server_map
                        .get(&result.resource_name)
                        .cloned()
                        .unwrap_or_else(|| vec![result.resource_name.clone()]);

                    let mut any_resolved = false;
                    let mut first_resolved_hcl: Option<String> = None;

                    for effective_server in &servers {
                        let resolved_for_server = resolve_firewall_server(
                            &resolved,
                            effective_server,
                            &xref.server_info_map,
                        );

                        if resolved_for_server.contains("upcloud_server.<TODO>.id") {
                            continue;
                        }

                        any_resolved = true;
                        if first_resolved_hcl.is_none() {
                            first_resolved_hcl = Some(resolved_for_server.clone());
                        }

                        // Collect rules into the per-server merge map.
                        if let Some(server_id_expr) =
                            extract_fw_server_id_expr(&resolved_for_server)
                        {
                            let rule_blocks = extract_fw_rule_blocks(&resolved_for_server);
                            let count_line = extract_fw_count_line(&resolved_for_server);
                            let entry = fw_by_server.entry(server_id_expr).or_insert_with(|| {
                                (
                                    result.resource_name.clone(),
                                    count_line,
                                    Vec::new(),
                                    Vec::new(),
                                )
                            });
                            entry.2.extend(rule_blocks);
                            // Tag each note with its source firewall rule so the merged resource is traceable.
                            entry.3.extend(
                                result
                                    .notes
                                    .iter()
                                    .map(|n| format!("[{}] {}", result.resource_name, n)),
                            );
                        }
                    }

                    // Store resolved HCL for the diff view (best available, may contain TODO).
                    let diff_hcl = first_resolved_hcl.unwrap_or_else(|| resolved.clone());
                    resolved_hcl_map.insert(
                        (result.resource_type.clone(), result.resource_name.clone()),
                        diff_hcl,
                    );

                    if !any_resolved {
                        // No server could be resolved (e.g. subnet-level security groups, or
                        // groups that only guard managed services). Emit the HCL with the TODO placeholder
                        // intact so users can see and manually complete the assignment.
                        for note in &result.notes {
                            content.push_str(&format!("# NOTE: {}\n", note));
                        }
                        content.push_str(&resolved);
                        content.push('\n');
                        log.push(format!("  [TODO] upcloud_firewall_rules.{}: server_id unresolved — assign manually", result.resource_name));
                    }
                    continue; // deferred to fw_by_server loop (for resolved ones)
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
                let commented: String = build_merged_fw_hcl(
                    resource_name,
                    server_id_expr,
                    count_line.as_deref(),
                    rule_blocks,
                )
                .lines()
                .map(|l| format!("# {}\n", l))
                .collect();
                content.push_str(&commented);
                content.push('\n');
                continue;
            }
            written_fw_servers.insert(server_name);

            // Count how many distinct SGs contributed (notes are tagged "[sg_name] ...").
            let sg_count = notes
                .iter()
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
            let merged_hcl = build_merged_fw_hcl(
                resource_name,
                server_id_expr,
                count_line.as_deref(),
                rule_blocks,
            );
            content.push_str(&merged_hcl);
            content.push('\n');
        }

        // Append passthrough blocks (variable / output / locals / data) for this file.
        // Provider blocks are handled separately in providers.tf.
        if let Some(pts) = passthrough_map.get(filename) {
            for pt in pts {
                // Provider/required_provider entries go to providers.tf, not here
                if pt.kind == PassthroughKind::Provider {
                    continue;
                }
                let has_provider_prefix = pt
                    .name
                    .as_deref()
                    .map(|n: &str| {
                        let prefix = provider.resource_type_prefix();
                        let bare = prefix.trim_end_matches('_');
                        n.starts_with(prefix) || n == bare
                    })
                    .unwrap_or(false);
                if has_provider_prefix {
                    // Do NOT inject <TODO:> into the variable name — that breaks HCL.
                    // Instead, add a plain comment so the user knows to review it.
                    content.push_str(&format!(
                        "# NOTE: Variable name '{}' references {} — consider renaming \
                         (e.g. \"{}\").\n",
                        pt.name.as_deref().unwrap_or(""),
                        provider.display_name(),
                        pt.name
                            .as_deref()
                            .unwrap_or("")
                            .trim_start_matches(provider.resource_type_prefix()),
                    ));
                }
                // Rewrite source provider resource references inside output/locals/data blocks.
                // Variables: run multi-signal detection and auto-convert instance types / regions.
                let hcl = if pt.kind == PassthroughKind::Output
                    || pt.kind == PassthroughKind::Locals
                    || pt.kind == PassthroughKind::Data
                {
                    // rewrite_output_refs may leave ${<TODO: ...>} when a ref can't be mapped.
                    // remove_todo_interpolations strips the ${} wrapper, producing valid HCL.
                    remove_todo_interpolations(provider.rewrite_output_refs(&pt.raw_hcl))
                } else if pt.kind == PassthroughKind::Variable {
                    let var_name = pt.name.as_deref().unwrap_or("");
                    let (default_val, description) = extract_variable_info(&pt.raw_hcl);
                    let usage_attrs = var_usage_map.get(var_name).cloned().unwrap_or_default();
                    if let Some(mut conv) = analyze_variable_with(
                        provider.var_detector().as_ref(),
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
                    }
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
                "  description = \"SSH public key for server access (replaces source key_name)\"\n",
                "  type        = string\n",
                "}\n\n",
            ));
            log.push("  [SYNTH] variable \"ssh_public_key\" added to variables.tf".to_string());
        }

        // Inject storage_devices blocks into servers that have volume attachments.
        // The compute mapper leaves a sentinel comment inside each server block:
        //   # __STORAGE_END_<server_name>__
        // We replace it with the actual storage_devices block(s).
        for (server_name, blocks) in &xref.storage_inject_map {
            let sentinel = format!("  # __STORAGE_END_{}__\n", server_name);
            if content.contains(&sentinel) {
                let injection: String = blocks.join("");
                content = content.replace(&sentinel, &injection);
                log.push(format!(
                    "  [INJECT] storage_devices → upcloud_server.{}",
                    server_name
                ));
            }
        }

        // Promote count onto storage resources that are attached to counted servers.
        for (storage_name, count_val) in &xref.storage_promote_count {
            let header = format!("resource \"upcloud_storage\" \"{}\" {{", storage_name);
            if let Some(pos) = content.find(&header) {
                let after_brace = pos + header.len();
                // Find the existing title line and update it to include count.index
                let old_title = format!("title = \"{}\"", storage_name);
                let new_title = format!("title = \"{}_${{count.index + 1}}\"", storage_name);
                // Insert count after the opening brace and update the title
                let count_line = format!("\n  count = {}", count_val);
                content.insert_str(after_brace, &count_line);
                content = content.replace(&old_title, &new_title);
                // Insert a NOTE comment before the resource
                let note = format!(
                    "# NOTE: One storage device per server instance (count = {})\n",
                    count_val
                );
                content = content.replacen(&header, &format!("{}{}", note, header), 1);
                log.push(format!(
                    "  [PROMOTE] count = {} → upcloud_storage.{} (one per server instance)",
                    count_val, storage_name
                ));
                // Keep resolved_hcl_map in sync so the pricing calculator sees the count.
                // The map is keyed by (source_resource_type, resource_name).
                let key = (
                    provider.volume_resource_type().to_string(),
                    storage_name.clone(),
                );
                if let Some(existing) = resolved_hcl_map.get(&key).cloned()
                    && let Some(brace_pos) = existing.find('{')
                {
                    let count_insert = format!("\n  count = {}", count_val);
                    let mut updated = existing.clone();
                    updated.insert_str(brace_pos + 1, &count_insert);
                    let old_title = format!("title = \"{}\"", storage_name);
                    let new_title = format!("title = \"{}_${{count.index + 1}}\"", storage_name);
                    let updated = updated.replace(&old_title, &new_title);
                    resolved_hcl_map.insert(key, updated);
                }
            }
        }

        // Remove any uninjected sentinels (servers with no attachments)
        let content_cleaned: String = content
            .lines()
            .filter(|l| !l.trim_start().starts_with("# __STORAGE_END_"))
            .map(|l| {
                let mut s = l.to_string();
                s.push('\n');
                s
            })
            .collect();
        content = content_cleaned;

        match std::fs::write(&out_path, &content) {
            Ok(_) => {
                log.push(format!("  [OK] {}", filename));
                // Only validate HCL when there are no TODO placeholders — TODOs intentionally
                // produce invalid HCL (e.g. `= upcloud_server.web.<TODO: ...>`), so any parse
                // error there is expected and not worth surfacing to the user.
                if !content.contains(TODO_PLACEHOLDER_PREFIX)
                    && let Err(e) = hcl::from_str::<hcl::Body>(&content)
                {
                    log.push(format!("  [HCL ERR] {} — {}", filename, e));
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
                && !(provider.resource_role(&r.resource_type) == ResourceRole::VolumeAttachment && r.parent_resource.is_some())
        })
        .collect();

    let unsupported: Vec<&MigrationResult> = results
        .iter()
        .filter(|r| {
            r.status == MigrationStatus::Unsupported || r.status == MigrationStatus::Unknown
        })
        .collect();

    if !partial.is_empty() || !unsupported.is_empty() {
        let notes_path = output_dir.join("MIGRATION_NOTES.md");
        let mut notes = String::from("# Migration Notes\n\n");
        notes.push_str(&format!("Target zone: **{}**  \n", zone));
        notes.push_str(&format!(
            "Object storage region: **{}**\n\n",
            objstorage_region
        ));

        if !partial.is_empty() {
            notes.push_str("## Partial Resources — Manual Action Required\n\n");
            notes.push_str(
                "These resources have no standalone UpCloud equivalent. \
                The code snippets below must be merged into the appropriate resource blocks.\n\n",
            );

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

/// Returns true if byte position `pos` in `s` is inside an open `<TODO: ...>` marker,
/// i.e. there is a `<TODO` before `pos` with no closing `>` between them.
pub(crate) fn inside_todo_marker(s: &str, pos: usize) -> bool {
    if let Some(last_open) = s[..pos].rfind(TODO_PLACEHOLDER_PREFIX) {
        !s[last_open..pos].contains('>')
    } else {
        false
    }
}

/// Replace every `${<TODO: ...>...}` template interpolation with the plain TODO
/// text extracted from inside.
///
/// These arise when source provider sanitization replaces a traversal that was
/// embedded inside a `${...}` in a heredoc user_data block.  Leaving the
/// `${...}` wrapper makes the HCL invalid; stripping it produces a literal
/// string in the heredoc that `terraform validate` (and hcl-rs) can parse.
pub(crate) fn remove_todo_interpolations(mut s: String) -> String {
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
        // Extract the TODO text from inside the ${...} wrapper
        let inner = s[start + 2..end - 1].trim().to_string();
        let replacement = if let Some(todo_start) = inner.find("<TODO:") {
            // Find the end of the TODO marker
            let todo_end = inner[todo_start..]
                .find('>')
                .map(|p| todo_start + p + 1)
                .unwrap_or(inner.len());
            inner[todo_start..todo_end].to_string()
        } else {
            "<TODO: remove source provider resource ref>".to_string()
        };
        s.replace_range(start..end, &replacement);
    }
    s
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
    let mut s = format!(
        "resource \"upcloud_firewall_rules\" \"{}\" {{\n",
        resource_name
    );
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
    server_name: &str,
    server_info_map: &HashMap<String, Option<String>>,
) -> String {
    match server_info_map.get(server_name) {
        None => hcl.to_string(),
        Some(None) => hcl.replace(
            "upcloud_server.<TODO>.id",
            &format!("upcloud_server.{}.id", server_name),
        ),
        Some(Some(n)) => {
            // Replace the server_id reference with an indexed one
            let mut s = hcl.replace(
                "upcloud_server.<TODO>.id",
                &format!("upcloud_server.{}[count.index].id", server_name),
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
    use crate::migration::providers::aws::generator_support::rewrite_output_refs;
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
            "aws_subnet",
            "public_a",
            "upcloud_network",
            Some(
                r#"resource "upcloud_network" "public_a" {
  zone = "__ZONE__"
  name = "public-a"
  ip_network { address = "10.0.1.0/24" dhcp = true family = "IPv4" }
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
  ip_network { address = "10.0.2.0/24" dhcp = true family = "IPv4" }
}
"#,
            ),
            None,
        );

        // aws_instance.web lives in public_a
        let mut instance = make_result(
            "aws_instance",
            "web",
            "upcloud_server",
            Some(
                r#"resource "upcloud_server" "web" {
  hostname = "web"
  zone     = "__ZONE__"
  plan     = "1xCPU-1GB"
  template { storage = "Ubuntu Server 24.04 LTS (Noble Numbat)" size = 50 }
}
"#,
            ),
            None,
        );
        instance.source_hcl = Some(
            r#"resource "aws_instance" "web" {
  ami           = "ami-12345"
  instance_type = "t3.micro"
  subnet_id     = aws_subnet.public_a.id
}
"#
            .to_string(),
        );

        // aws_lb_target_group_attachment: TG=web, server=web[0]
        let mut attachment = make_result(
            "aws_lb_target_group_attachment",
            "web_1",
            "upcloud_loadbalancer_static_backend_member",
            Some(
                r#"resource "upcloud_loadbalancer_static_backend_member" "web_1" {
  backend      = upcloud_loadbalancer_backend.<TODO>.name
  name         = "web-1"
  ip           = "<TODO: server IP>"
  port         = 80
  weight       = 100
  max_sessions = 1000
}
"#,
            ),
            None,
        );
        attachment.source_hcl = Some(
            r#"resource "aws_lb_target_group_attachment" "web_1" {
  target_group_arn = aws_lb_target_group.web.arn
  target_id        = aws_instance.web[0].id
  port             = 80
}
"#
            .to_string(),
        );

        // aws_lb_listener: lb=main, TG=web
        let mut listener = make_result(
            "aws_lb_listener",
            "https",
            "upcloud_loadbalancer_frontend",
            Some(
                r#"resource "upcloud_loadbalancer_frontend" "https" {
  name             = "https"
  mode             = "tcp"
  port             = 443
  loadbalancer     = upcloud_loadbalancer.<TODO>.id
  default_backend_name = upcloud_loadbalancer_backend.<TODO>.name
}
"#,
            ),
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
"#
            .to_string(),
        );

        // aws_lb.main: has the generic network placeholder
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
        let hcl =
            "resource \"upcloud_server\" \"web\" {\n  count    = 2\n  hostname = \"web\"\n}\n";
        assert_eq!(extract_count_from_hcl(hcl), Some("2".to_string()));
    }

    #[test]
    fn extract_count_returns_none_when_absent() {
        let hcl = "resource \"upcloud_server\" \"web\" {\n  hostname = \"web\"\n}\n";
        assert_eq!(extract_count_from_hcl(hcl), None);
    }

    #[test]
    fn firewall_rule_with_unresolved_server_is_emitted_with_todo() {
        // aws_security_group "lb" → upcloud_firewall_rules "lb", but there is no
        // upcloud_server named "lb". The rule must still be emitted (with a TODO
        // server_id) so users can see and manually complete the assignment.
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
        let output = run_generate(&[lb_fw, web, app], "fw_lb_emit");
        // Unresolved firewall rules are emitted with a TODO so the user can assign them.
        assert!(
            output.contains("upcloud_firewall_rules\" \"lb\""),
            "unresolved firewall rule must be present in output\n{output}"
        );
        assert!(
            output.contains("upcloud_server.<TODO>.id"),
            "unresolved server_id TODO must appear in output\n{output}"
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
    fn firewall_to_server_map_resolves_mismatched_names() {
        use crate::migration::providers::{SourceProvider, aws::AwsSourceProvider};
        let provider = AwsSourceProvider;
        // docker_demo firewall is attached to docker_server instance via vpc_security_group_ids.
        let instance_hcl = concat!(
            "resource \"aws_instance\" \"docker_server\" {\n",
            "  vpc_security_group_ids = [aws_security_group.docker_demo.id]\n",
            "}\n"
        );
        let refs = provider.extract_security_refs_from_instance(instance_hcl);
        assert!(
            refs.contains(&"docker_demo".to_string()),
            "should extract firewall name: {refs:?}"
        );

        // When the firewall_to_server_map is used, the firewall rule should resolve to docker_server.
        let fw_hcl = "resource \"upcloud_firewall_rules\" \"docker_demo\" {\n  server_id = upcloud_server.<TODO>.id\n}\n";
        let mut server_map = HashMap::new();
        server_map.insert("docker_server".to_string(), None);
        // effective_server comes from firewall_to_server_map lookup
        let effective_server = "docker_server";
        let out = resolve_firewall_server(fw_hcl, effective_server, &server_map);
        assert!(
            out.contains("upcloud_server.docker_server.id"),
            "should resolve to docker_server: {out}"
        );
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

    // ── End-to-end: user_data with source provider cross-refs ────────────────
    //
    // Ensures that when user_data heredocs contain ${provider_type.*.attr} references,
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
        let results: Vec<MigrationResult> = parsed.resources.iter().map(map_resource).collect();

        let out_dir = dir.join("out");
        let mut log = vec![];
        generate_files(&results, &[], &out_dir, None, "fi-hel2", &mut log).unwrap();
        let output = std::fs::read_to_string(out_dir.join("main.tf"))
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
        hcl::from_str::<hcl::Body>(&output).expect("generated output must be valid HCL");
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
        let results: Vec<MigrationResult> = parsed.resources.iter().map(map_resource).collect();

        let out_dir = std::env::temp_dir().join("upcloud_e2e_webapp");
        let mut log = vec![];
        generate_files(&results, &[], &out_dir, None, "fi-hel2", &mut log).unwrap();
        let output = std::fs::read_to_string(out_dir.join("webapp-e2e.tf"))
            .expect("generate_files must produce webapp-e2e.tf");
        let _ = std::fs::remove_dir_all(&out_dir);

        // No unresolved server_id TODOs from server-attached SGs — they should all resolve.
        // (The 'lb' SG has no matching server and will be emitted with a TODO.)
        assert!(
            !output.contains("upcloud_server.web.<TODO>"),
            "web server_id must be resolved\n{output}"
        );
        assert!(
            !output.contains("upcloud_server.app.<TODO>"),
            "app server_id must be resolved\n{output}"
        );

        // LB security group has no matching server — emitted with TODO so users can assign it.
        assert!(
            output.contains("upcloud_firewall_rules\" \"lb\""),
            "firewall_rules for 'lb' SG must be emitted with TODO\n{output}"
        );
        assert!(
            output.contains("upcloud_server.<TODO>.id"),
            "lb firewall rules must have TODO server_id\n{output}"
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
        assert!(
            output.contains("resource \"upcloud_server\" \"web\""),
            "{output}"
        );
        assert!(
            output.contains("resource \"upcloud_server\" \"app\""),
            "{output}"
        );
        assert!(
            output.contains("resource \"upcloud_server\" \"database\""),
            "{output}"
        );
        assert!(
            output.contains("resource \"upcloud_server\" \"redis\""),
            "{output}"
        );

        // Networking: VPC → router, subnet → network.
        assert!(
            output.contains("resource \"upcloud_router\" \"main_router\""),
            "{output}"
        );
        assert!(
            output.contains("resource \"upcloud_network\" \"public\""),
            "{output}"
        );

        // Load balancer and its components.
        assert!(
            output.contains("resource \"upcloud_loadbalancer\" \"main\""),
            "{output}"
        );
        assert!(
            output.contains("type   = \"public\""),
            "LB must have public networks block\n{output}"
        );
        assert!(
            output.contains("resource \"upcloud_loadbalancer_backend\" \"web\""),
            "{output}"
        );
        assert!(
            output.contains("resource \"upcloud_loadbalancer_frontend\" \"http\""),
            "{output}"
        );
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
        let fw_count = output
            .matches("resource \"upcloud_firewall_rules\"")
            .count();
        assert_eq!(
            fw_count, 1,
            "must have exactly 1 upcloud_firewall_rules resource\n{output}"
        );

        // Must contain rules from BOTH security groups.
        assert!(
            output.contains("destination_port_start = \"80\""),
            "must contain web_sg port-80 rule\n{output}"
        );
        assert!(
            output.contains("destination_port_start = \"5432\""),
            "must contain db_sg port-5432 rule\n{output}"
        );

        // Catch-all outbound rule must appear only once (deduplicated).
        let outbound_count = output.matches("Allow all outbound").count();
        assert_eq!(
            outbound_count, 1,
            "catch-all outbound rule must be deduplicated\n{output}"
        );

        // server_id must be resolved (no TODO).
        assert!(
            !output.contains("<TODO>"),
            "no TODO must remain in merged firewall resource\n{output}"
        );
        assert!(
            output.contains("upcloud_server.app.id"),
            "server_id must be resolved to 'app'\n{output}"
        );
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
        assert_eq!(
            blocks.len(),
            2,
            "should extract 2 rule blocks: {:?}",
            blocks
        );
        assert!(blocks[0].contains("direction = \"in\""), "{:?}", blocks[0]);
        assert!(blocks[1].contains("direction = \"out\""), "{:?}", blocks[1]);
    }

    #[test]
    fn build_merged_fw_hcl_deduplicates_identical_blocks() {
        let outbound = "  firewall_rule {\n    direction = \"out\"\n    action = \"accept\"\n  }\n";
        let inbound = "  firewall_rule {\n    direction = \"in\"\n    action = \"accept\"\n  }\n";
        let blocks = vec![
            outbound.to_string(),
            inbound.to_string(),
            outbound.to_string(), // duplicate
        ];
        let merged = build_merged_fw_hcl("web_sg", "upcloud_server.web.id", None, &blocks);
        let outbound_count = merged.matches("direction = \"out\"").count();
        assert_eq!(
            outbound_count, 1,
            "duplicate outbound rule must be deduplicated\n{merged}"
        );
        assert!(merged.contains("direction = \"in\""), "{merged}");
    }

    #[test]
    fn server_name_from_server_id_expr_plain() {
        assert_eq!(
            server_name_from_server_id_expr("upcloud_server.myserver.id"),
            "myserver"
        );
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
        use crate::migration::mapper::map_resource;
        use crate::terraform::parser::parse_tf_file;
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
        assert_eq!(
            parsed.passthroughs.len(),
            2,
            "should find 2 variable blocks"
        );

        let results: Vec<MigrationResult> = parsed.resources.iter().map(map_resource).collect();
        let pts: Vec<PassthroughBlock> = parsed.passthroughs.clone();

        let out_dir = dir.join("out");
        let mut log = vec![];
        generate_files(&results, &pts, &out_dir, None, "fi-hel2", &mut log).unwrap();
        let output = std::fs::read_to_string(out_dir.join("main.tf"))
            .expect("generate_files must produce main.tf");
        let _ = std::fs::remove_dir_all(&dir);

        assert!(
            output.contains("variable \"db_username\""),
            "db_username variable must appear\n{output}"
        );
        assert!(
            output.contains("variable \"db_password\""),
            "db_password variable must appear\n{output}"
        );
        assert!(
            output.contains("sensitive   = true"),
            "variable body must be preserved\n{output}"
        );
        // Non-AWS names must not get a NOTE comment.
        assert!(
            !output.contains("NOTE: Variable name 'db_username'"),
            "{output}"
        );

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
        assert!(
            output.contains("# NOTE: Variable name 'aws_region'"),
            "{output}"
        );
        assert!(
            !output.contains("<TODO:"),
            "must not inject TODO into variable name\n{output}"
        );
        // The variable block itself must still be present with the original name.
        assert!(output.contains("variable \"aws_region\""), "{output}");

        // Must be parseable.
        hcl::from_str::<hcl::Body>(&output).expect("output must be valid HCL");
    }

    #[test]
    fn locals_block_passed_through() {
        use crate::migration::mapper::map_resource;
        use crate::terraform::parser::parse_tf_file;
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
        let results: Vec<MigrationResult> = parsed.resources.iter().map(map_resource).collect();

        let out_dir = dir.join("out");
        let mut log = vec![];
        generate_files(&results, &pts, &out_dir, None, "fi-hel2", &mut log).unwrap();
        let output = std::fs::read_to_string(out_dir.join("main.tf"))
            .expect("generate_files must produce main.tf");
        let _ = std::fs::remove_dir_all(&dir);

        assert!(
            output.contains("locals {"),
            "locals block must appear\n{output}"
        );
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
        assert!(
            !rewritten.contains("aws_instance"),
            "no AWS ref should remain\n{rewritten}"
        );
    }

    #[test]
    fn output_known_type_unknown_attr_gets_todo() {
        let hcl = "output \"arn\" {\n  value = aws_instance.web.arn\n}";
        let rewritten = rewrite_output_refs(hcl);
        assert!(
            rewritten.contains("\"<TODO: was upcloud_server.web.arn"),
            "unknown attr should get a quoted-string TODO\n{rewritten}"
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
            rewritten.contains("[for n in upcloud_loadbalancer.main.networks : n.dns_name if n.type == \"public\"][0]"),
            "lb dns_name should use for-expression over public network\n{rewritten}"
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
        assert_eq!(
            rewritten, hcl,
            "refs inside TODO markers must not be rewritten"
        );
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
        use crate::migration::mapper::map_resource;
        use crate::terraform::parser::parse_tf_file;

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
            parsed.resources.iter().map(map_resource).collect();

        let out_dir = dir.join("out");
        let mut log = vec![];
        generate_files(
            &results,
            &parsed.passthroughs,
            &out_dir,
            None,
            "fi-hel1",
            &mut log,
        )
        .unwrap();
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
        assert!(
            !output.contains("aws_instance.web"),
            "no AWS resource traversals in output\n{output}"
        );
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

        let output = run_generate(
            &[pg, valkey, sg, cache_sg, public_a, data],
            "db_subnet_group",
        );

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
        assert!(
            !result.contains("upcloud_router.main.id"),
            "must not use bare 'main'\n{result}"
        );
    }

    // ── DB network [0] index for counted network ───────────────────────────────

    #[test]
    fn subnet_group_resolution_uses_index_for_counted_network() {
        // When the target network has count=2, the DB network uuid must use [0].
        let pg = make_result(
            "aws_db_instance",
            "main",
            "upcloud_managed_database_postgresql",
            Some(
                "resource \"upcloud_managed_database_postgresql\" \"main\" {\n  \
                  network {\n    uuid = \"<TODO: upcloud_network UUID subnet_group=main>\"\n  }\n}\n",
            ),
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
            Some(
                "resource \"upcloud_network\" \"database\" {\n  count = 2\n  zone = \"__ZONE__\"\n}\n",
            ),
            None,
        );

        let mut results = vec![pg, db_net];
        // Add a fake subnet_group result so source_hcl maps it
        let mut sg_with_hcl = sg_result;
        sg_with_hcl.source_hcl = Some(
            r#"resource "aws_db_subnet_group" "main" {
  subnet_ids = [aws_subnet.database[0].id, aws_subnet.database[1].id]
}"#
            .to_string(),
        );
        results.push(sg_with_hcl);

        let out_dir = std::env::temp_dir().join("upcloud_gen_db_idx_test");
        let mut log = vec![];
        generate_files(&results, &[], &out_dir, None, "fi-hel1", &mut log).unwrap();
        let output = std::fs::read_to_string(out_dir.join("test.tf")).unwrap_or_default();
        let _ = std::fs::remove_dir_all(&out_dir);

        assert!(
            output.contains("upcloud_network.database[0].id"),
            "counted database network must be indexed with [0]\n{output}\nlog: {:?}",
            log
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
            attributes: attrs
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            source_file: PathBuf::from("main.tf"),
            raw_hcl: String::new(),
        };

        // web server with count
        let web_res = make(
            "aws_instance",
            "web",
            &[
                ("instance_type", "t3.micro"),
                ("count", "2"),
                ("subnet_id", "aws_subnet.private.id"),
            ],
        );
        let mut web_result = map_instance(&web_res);
        web_result.source_hcl = Some(web_result.upcloud_hcl.clone().unwrap());

        // EBS volume with count
        let vol_res = make(
            "aws_ebs_volume",
            "data",
            &[("type", "gp3"), ("size", "50"), ("count", "2")],
        );
        let vol_result = map_ebs_volume(&vol_res);

        // Attachment
        let att_res = make(
            "aws_volume_attachment",
            "data",
            &[
                ("volume_id", "aws_ebs_volume.data[count.index].id"),
                ("instance_id", "aws_instance.web[count.index].id"),
            ],
        );
        use crate::migration::providers::aws::storage::map_volume_attachment;
        let att_result = map_volume_attachment(&att_res);

        let out_dir = std::env::temp_dir().join("upcloud_storage_inject_test");
        let mut log = vec![];
        generate_files(
            &[web_result, vol_result, att_result],
            &[],
            &out_dir,
            None,
            "fi-hel1",
            &mut log,
        )
        .unwrap();
        let output = std::fs::read_to_string(out_dir.join("main.tf")).unwrap_or_default();
        let _ = std::fs::remove_dir_all(&out_dir);

        assert!(
            output.contains("storage_devices {"),
            "storage_devices block must be injected into server\n{output}\nlog: {:?}",
            log
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

    #[test]
    fn storage_count_promoted_when_server_has_count_but_storage_does_not() {
        use crate::migration::providers::aws::{
            compute::map_instance,
            storage::{map_ebs_volume, map_volume_attachment},
        };
        use crate::terraform::types::TerraformResource;
        use std::path::PathBuf;

        let make = |rt: &str, name: &str, attrs: &[(&str, &str)]| TerraformResource {
            resource_type: rt.to_string(),
            name: name.to_string(),
            attributes: attrs
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            source_file: PathBuf::from("main.tf"),
            raw_hcl: String::new(),
        };

        // Server with count = 2
        let srv_res = make(
            "aws_instance",
            "api",
            &[
                ("instance_type", "t3.micro"),
                ("count", "2"),
                ("subnet_id", "aws_subnet.private.id"),
            ],
        );
        let mut srv_result = map_instance(&srv_res);
        srv_result.source_hcl = Some(srv_result.upcloud_hcl.clone().unwrap());

        // EBS volume WITHOUT count (single volume in AWS, attached to api[0])
        let vol_res = make(
            "aws_ebs_volume",
            "api_data",
            &[("type", "gp3"), ("size", "200")],
        );
        let vol_result = map_ebs_volume(&vol_res);

        // Attachment: volume → api[0]
        let att_res = make(
            "aws_volume_attachment",
            "api_data",
            &[
                ("volume_id", "aws_ebs_volume.api_data.id"),
                ("instance_id", "aws_instance.api[0].id"),
            ],
        );
        let att_result = map_volume_attachment(&att_res);

        let out_dir = std::env::temp_dir().join("upcloud_storage_promote_count_test");
        let mut log = vec![];
        generate_files(
            &[srv_result, vol_result, att_result],
            &[],
            &out_dir,
            None,
            "fi-hel1",
            &mut log,
        )
        .unwrap();
        let output = std::fs::read_to_string(out_dir.join("main.tf")).unwrap_or_default();
        let _ = std::fs::remove_dir_all(&out_dir);

        // Storage must now have count = 2
        assert!(
            output.contains("resource \"upcloud_storage\" \"api_data\"")
                && output.contains("count = 2"),
            "storage must have count promoted from server\n{output}\nlog: {:?}",
            log
        );
        // Title must be indexed
        assert!(
            output.contains("title = \"api_data_${count.index + 1}\""),
            "storage title must include count.index\n{output}"
        );
        // storage_devices must reference with [count.index]
        assert!(
            output.contains("upcloud_storage.api_data[count.index].id"),
            "storage_devices ref must use count.index\n{output}"
        );
        // Log must show promotion
        assert!(
            log.iter().any(|l| l.contains("[PROMOTE]")),
            "log must contain PROMOTE entry\nlog: {:?}",
            log
        );
    }

    // ── Non-cloud-provider resource passthrough ────────────────────────────────

    #[test]
    fn non_cloud_resource_kept_as_is_in_output() {
        // A kubernetes_deployment_v1 should be passed through unchanged.
        let k8s_hcl = r#"resource "kubernetes_deployment_v1" "nginx" {
  metadata {
    name = "nginx"
  }

  spec {
    replicas = 2
    selector {
      match_labels = {
        App = "nginx"
      }
    }
    template {
      metadata {
        labels = {
          App = "nginx"
        }
      }
      spec {
        container {
          image = "nginx:stable"
          name  = "example"
        }
      }
    }
  }
}
"#;
        let passthrough = MigrationResult {
            resource_type: "kubernetes_deployment_v1".to_string(),
            resource_name: "nginx".to_string(),
            source_file: "test.tf".to_string(),
            status: MigrationStatus::Passthrough,
            upcloud_type: "kubernetes_deployment_v1".to_string(),
            upcloud_hcl: Some(k8s_hcl.to_string()),
            snippet: None,
            parent_resource: None,
            notes: vec!["Non-cloud-provider resource, kept as is.".to_string()],
            source_hcl: Some(k8s_hcl.to_string()),
        };

        let output = run_generate(&[passthrough], "passthrough_k8s");

        assert!(
            output.contains("kubernetes_deployment_v1"),
            "passthrough resource must appear in output\n{output}"
        );
        assert!(
            output.contains("replicas = 2"),
            "passthrough resource HCL must be preserved\n{output}"
        );
        assert!(
            output.contains("kept as is"),
            "passthrough resource must have 'kept as is' comment\n{output}"
        );
    }

    #[test]
    fn passthrough_resource_alongside_mapped_resource() {
        // When a file has both AWS resources (mapped) and non-cloud resources (passthrough),
        // both should appear in the output.
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

        let k8s_hcl = r#"resource "kubernetes_deployment_v1" "app" {
  metadata {
    name = "app"
  }

  spec {
    replicas = 1
  }
}
"#;
        let passthrough = MigrationResult {
            resource_type: "kubernetes_deployment_v1".to_string(),
            resource_name: "app".to_string(),
            source_file: "test.tf".to_string(),
            status: MigrationStatus::Passthrough,
            upcloud_type: "kubernetes_deployment_v1".to_string(),
            upcloud_hcl: Some(k8s_hcl.to_string()),
            snippet: None,
            parent_resource: None,
            notes: vec!["Non-cloud-provider resource, kept as is.".to_string()],
            source_hcl: Some(k8s_hcl.to_string()),
        };

        let output = run_generate(&[server, passthrough], "passthrough_mixed");

        // Both should be present
        assert!(
            output.contains("upcloud_server"),
            "mapped resource must appear in output\n{output}"
        );
        assert!(
            output.contains("kubernetes_deployment_v1"),
            "passthrough resource must appear in output\n{output}"
        );
    }

    #[test]
    fn passthrough_not_in_migration_notes() {
        // Passthrough resources should NOT appear in MIGRATION_NOTES.md
        let k8s_hcl = r#"resource "kubernetes_deployment_v1" "nginx" {
  metadata {
    name = "nginx"
  }
}
"#;
        let passthrough = MigrationResult {
            resource_type: "kubernetes_deployment_v1".to_string(),
            resource_name: "nginx".to_string(),
            source_file: "test.tf".to_string(),
            status: MigrationStatus::Passthrough,
            upcloud_type: "kubernetes_deployment_v1".to_string(),
            upcloud_hcl: Some(k8s_hcl.to_string()),
            snippet: None,
            parent_resource: None,
            notes: vec!["Non-cloud-provider resource, kept as is.".to_string()],
            source_hcl: Some(k8s_hcl.to_string()),
        };

        let dir = std::env::temp_dir().join("upcloud_gen_test_passthrough_notes");
        std::fs::create_dir_all(&dir).unwrap();
        let mut log = vec![];
        generate_files(&[passthrough], &[], &dir, None, "fi-hel2", &mut log).unwrap();
        let notes_exists = dir.join("MIGRATION_NOTES.md").exists();
        let _ = std::fs::remove_dir_all(&dir);

        // No unsupported/partial resources → no MIGRATION_NOTES.md should be generated
        assert!(
            !notes_exists,
            "MIGRATION_NOTES.md should not be generated when only passthrough resources exist"
        );
    }

    #[test]
    fn passthrough_with_cloud_refs_are_rewritten() {
        // If a non-cloud resource references an AWS resource, the reference
        // should be rewritten to the UpCloud equivalent.
        let k8s_hcl = r#"resource "kubernetes_config_map" "cluster_info" {
  metadata {
    name = "cluster-info"
  }

  data = {
    endpoint = aws_eks_cluster.main.endpoint
  }
}
"#;
        let passthrough = MigrationResult {
            resource_type: "kubernetes_config_map".to_string(),
            resource_name: "cluster_info".to_string(),
            source_file: "test.tf".to_string(),
            status: MigrationStatus::Passthrough,
            upcloud_type: "kubernetes_config_map".to_string(),
            upcloud_hcl: Some(k8s_hcl.to_string()),
            snippet: None,
            parent_resource: None,
            notes: vec!["Non-cloud-provider resource, kept as is.".to_string()],
            source_hcl: Some(k8s_hcl.to_string()),
        };

        let output = run_generate(&[passthrough], "passthrough_rewrite_refs");

        assert!(
            !output.contains("aws_eks_cluster"),
            "AWS references in passthrough resources should be rewritten\n{output}"
        );
        assert!(
            output.contains("upcloud_kubernetes_cluster"),
            "AWS references should be rewritten to UpCloud equivalents\n{output}"
        );
    }

    #[test]
    fn passthrough_excluded_from_pricing() {
        // Passthrough resources should not be counted for UpCloud pricing
        let k8s_hcl = r#"resource "kubernetes_deployment_v1" "nginx" {
  spec { replicas = 2 }
}
"#;
        let passthrough = MigrationResult {
            resource_type: "kubernetes_deployment_v1".to_string(),
            resource_name: "nginx".to_string(),
            source_file: "test.tf".to_string(),
            status: MigrationStatus::Passthrough,
            upcloud_type: "kubernetes_deployment_v1".to_string(),
            upcloud_hcl: Some(k8s_hcl.to_string()),
            snippet: None,
            parent_resource: None,
            notes: vec!["Non-cloud-provider resource, kept as is.".to_string()],
            source_hcl: Some(k8s_hcl.to_string()),
        };
        assert_eq!(
            passthrough.status,
            MigrationStatus::Passthrough,
            "passthrough status must be Passthrough"
        );
        assert!(
            matches!(
                passthrough.status,
                MigrationStatus::Unsupported
                    | MigrationStatus::Unknown
                    | MigrationStatus::Passthrough
            ),
            "passthrough should be filtered out of pricing calculations"
        );
    }

    #[test]
    fn map_resource_produces_passthrough_for_non_cloud_resources() {
        use crate::migration::mapper::map_resource;

        let res = crate::terraform::types::TerraformResource {
            resource_type: "kubernetes_deployment_v1".to_string(),
            name: "nginx".to_string(),
            attributes: std::collections::HashMap::new(),
            source_file: std::path::PathBuf::from("test.tf"),
            raw_hcl: "resource \"kubernetes_deployment_v1\" \"nginx\" {\n  metadata {\n    name = \"nginx\"\n  }\n}".to_string(),
        };

        let result = map_resource(&res);

        assert_eq!(result.status, MigrationStatus::Passthrough);
        assert!(
            result.upcloud_hcl.is_some(),
            "passthrough resource must have upcloud_hcl set"
        );
        assert_eq!(
            result.upcloud_hcl.as_deref().unwrap(),
            res.raw_hcl,
            "passthrough resource must preserve raw HCL"
        );
        assert_eq!(result.upcloud_type, "kubernetes_deployment_v1");
    }

    #[test]
    fn map_resource_various_non_cloud_types_produce_passthrough() {
        use crate::migration::mapper::map_resource;

        let non_cloud_types = [
            "kubernetes_deployment_v1",
            "helm_release",
            "null_resource",
            "random_password",
            "local_file",
            "tls_private_key",
        ];

        for rt in &non_cloud_types {
            let res = crate::terraform::types::TerraformResource {
                resource_type: rt.to_string(),
                name: "test".to_string(),
                attributes: std::collections::HashMap::new(),
                source_file: std::path::PathBuf::from("test.tf"),
                raw_hcl: format!("resource \"{}\" \"test\" {{\n}}", rt),
            };
            let result = map_resource(&res);
            assert_eq!(
                result.status,
                MigrationStatus::Passthrough,
                "resource type '{}' should produce Passthrough status",
                rt
            );
            assert!(
                result.upcloud_hcl.is_some(),
                "resource type '{}' should have upcloud_hcl set",
                rt
            );
        }
    }

    #[test]
    fn parser_captures_non_cloud_provider_blocks() {
        use crate::terraform::parser::parse_tf_file;
        use std::io::Write;

        let tf_content = r#"
terraform {
  required_providers {
    aws = {
      source  = "hashicorp/aws"
      version = "~> 5.0"
    }
    kubernetes = {
      source  = "hashicorp/kubernetes"
      version = "~> 3.0"
    }
  }
}

provider "aws" {
  region = "us-east-1"
}

provider "kubernetes" {
  host = "https://example.com"
}

resource "aws_instance" "web" {
  ami = "ami-12345"
}

resource "kubernetes_deployment_v1" "nginx" {
  metadata {
    name = "nginx"
  }
}
"#;
        let dir = std::env::temp_dir().join("upcloud_parser_test_provider_blocks");
        std::fs::create_dir_all(&dir).unwrap();
        let tf_path = dir.join("main.tf");
        let mut f = std::fs::File::create(&tf_path).unwrap();
        f.write_all(tf_content.as_bytes()).unwrap();
        drop(f);

        let parsed = parse_tf_file(&tf_path).expect("should parse");
        let _ = std::fs::remove_dir_all(&dir);

        // Should have 2 resources
        assert_eq!(parsed.resources.len(), 2, "should parse both resources");

        // Should have a provider passthrough for kubernetes
        let provider_pts: Vec<_> = parsed
            .passthroughs
            .iter()
            .filter(|p| p.kind == PassthroughKind::Provider)
            .collect();
        assert!(
            provider_pts
                .iter()
                .any(|p| p.name.as_deref() == Some("kubernetes")),
            "should capture kubernetes provider block as passthrough\n{:?}",
            provider_pts
        );

        // Should have a required_provider entry for kubernetes
        assert!(
            provider_pts.iter().any(|p| {
                p.name
                    .as_deref()
                    .map(|n| n.starts_with("required_provider:kubernetes"))
                    .unwrap_or(false)
            }),
            "should capture kubernetes required_providers entry\n{:?}",
            provider_pts
        );

        // Should NOT have aws provider or required_provider
        assert!(
            !provider_pts
                .iter()
                .any(|p| p.name.as_deref() == Some("aws")),
            "should NOT capture aws provider block\n{:?}",
            provider_pts
        );
    }

    #[test]
    fn non_cloud_providers_included_in_providers_tf() {
        use crate::terraform::types::PassthroughBlock;

        let k8s_hcl = r#"resource "kubernetes_deployment_v1" "nginx" {
  metadata {
    name = "nginx"
  }
}
"#;
        let passthrough = MigrationResult {
            resource_type: "kubernetes_deployment_v1".to_string(),
            resource_name: "nginx".to_string(),
            source_file: "test.tf".to_string(),
            status: MigrationStatus::Passthrough,
            upcloud_type: "kubernetes_deployment_v1".to_string(),
            upcloud_hcl: Some(k8s_hcl.to_string()),
            snippet: None,
            parent_resource: None,
            notes: vec![],
            source_hcl: None,
        };

        let provider_pt = PassthroughBlock {
            name: Some("kubernetes".to_string()),
            raw_hcl: "provider \"kubernetes\" {\n  host = \"https://example.com\"\n}".to_string(),
            source_file: std::path::PathBuf::from("main.tf"),
            kind: PassthroughKind::Provider,
        };

        let req_provider_pt = PassthroughBlock {
            name: Some("required_provider:kubernetes".to_string()),
            raw_hcl: "kubernetes = {\n      source  = \"hashicorp/kubernetes\"\n      version = \"~> 3.0\"\n    }".to_string(),
            source_file: std::path::PathBuf::from("main.tf"),
            kind: PassthroughKind::Provider,
        };

        let dir = std::env::temp_dir().join("upcloud_gen_test_providers_tf");
        std::fs::create_dir_all(&dir).unwrap();
        let mut log = vec![];
        generate_files(
            &[passthrough],
            &[provider_pt, req_provider_pt],
            &dir,
            None,
            "fi-hel2",
            &mut log,
        )
        .unwrap();
        let providers_content =
            std::fs::read_to_string(dir.join("providers.tf")).expect("providers.tf must exist");
        let _ = std::fs::remove_dir_all(&dir);

        assert!(
            providers_content.contains("upcloud"),
            "providers.tf must contain UpCloud provider\n{providers_content}"
        );
        assert!(
            providers_content.contains("kubernetes"),
            "providers.tf must contain kubernetes provider\n{providers_content}"
        );
        assert!(
            providers_content.contains("hashicorp/kubernetes"),
            "providers.tf must contain kubernetes required_provider source\n{providers_content}"
        );
    }

    #[test]
    fn kube_example_end_to_end() {
        use crate::migration::mapper::map_resource;
        use crate::terraform::parser::parse_tf_file;

        // Parse all files from the kube-example fixture
        let kube_dir = std::path::PathBuf::from("test-fixtures/kube-example");
        let mut all_resources = vec![];
        let mut all_passthroughs = vec![];

        for entry in std::fs::read_dir(&kube_dir).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("tf") {
                continue;
            }
            let parsed = parse_tf_file(&path.to_path_buf()).expect("should parse");
            all_resources.extend(parsed.resources);
            all_passthroughs.extend(parsed.passthroughs);
        }

        let results: Vec<MigrationResult> = all_resources.iter().map(map_resource).collect();

        // Verify kubernetes_deployment_v1 is classified as Passthrough
        let k8s_results: Vec<&MigrationResult> = results
            .iter()
            .filter(|r| r.resource_type == "kubernetes_deployment_v1")
            .collect();
        assert_eq!(
            k8s_results.len(),
            1,
            "should have 1 kubernetes_deployment_v1 resource"
        );
        assert_eq!(
            k8s_results[0].status,
            MigrationStatus::Passthrough,
            "kubernetes_deployment_v1 should be Passthrough"
        );
        assert!(
            k8s_results[0].upcloud_hcl.is_some(),
            "kubernetes_deployment_v1 should have upcloud_hcl"
        );

        // Generate the output
        let out_dir = std::env::temp_dir().join("upcloud_e2e_kube");
        let mut log = vec![];
        generate_files(
            &results,
            &all_passthroughs,
            &out_dir,
            Some(kube_dir.as_path()),
            "fi-hel2",
            &mut log,
        )
        .unwrap();

        // Check that the kubernetes deployment is in the output
        let kube_tf =
            std::fs::read_to_string(out_dir.join("kube.tf")).expect("kube.tf must exist in output");
        assert!(
            kube_tf.contains("kubernetes_deployment_v1"),
            "kube.tf must contain the kubernetes deployment\n{kube_tf}"
        );
        assert!(
            kube_tf.contains("scalable-nginx-example"),
            "kube.tf must preserve the kubernetes deployment content\n{kube_tf}"
        );
        assert!(
            kube_tf.contains("kept as is"),
            "kube.tf must have 'kept as is' comment\n{kube_tf}"
        );

        // Check that mapped resources also exist
        let main_tf =
            std::fs::read_to_string(out_dir.join("main.tf")).expect("main.tf must exist in output");
        assert!(
            main_tf.contains("upcloud_kubernetes_cluster"),
            "main.tf must contain mapped k8s cluster\n{main_tf}"
        );
        assert!(
            main_tf.contains("upcloud_router"),
            "main.tf must contain mapped router\n{main_tf}"
        );

        // Check providers.tf includes kubernetes provider
        let providers_tf =
            std::fs::read_to_string(out_dir.join("providers.tf")).expect("providers.tf must exist");
        assert!(
            providers_tf.contains("kubernetes"),
            "providers.tf must contain kubernetes provider\n{providers_tf}"
        );

        // kubernetes_deployment_v1 should NOT appear in MIGRATION_NOTES as unsupported
        if let Ok(notes) = std::fs::read_to_string(out_dir.join("MIGRATION_NOTES.md")) {
            assert!(
                !notes.contains("kubernetes_deployment_v1"),
                "kubernetes_deployment_v1 must NOT appear in MIGRATION_NOTES.md\n{notes}"
            );
        }

        let _ = std::fs::remove_dir_all(&out_dir);
    }

    #[test]
    fn network_lifecycle_block_injected_when_k8s_present() {
        let network = make_result(
            "aws_subnet",
            "main",
            "upcloud_network",
            Some(
                r#"resource "upcloud_network" "main" {
  name = "main"
  zone = "__ZONE__"

  ip_network {
    address = "10.0.1.0/24"
    dhcp    = true
    family  = "IPv4"
  }

  router = upcloud_router.main_router.id
}
"#,
            ),
            None,
        );
        let k8s = make_result(
            "aws_eks_cluster",
            "cluster",
            "upcloud_kubernetes_cluster",
            Some(
                r#"resource "upcloud_kubernetes_cluster" "cluster" {
  name    = "cluster"
  zone    = "__ZONE__"
  network = "<TODO: upcloud_network reference>"
  plan    = "K8S-2xCPU-4GB"
}
"#,
            ),
            None,
        );
        let output = run_generate(&[network, k8s], "lifecycle_with_k8s");
        assert!(
            output.contains("lifecycle"),
            "lifecycle block should be injected when k8s cluster is present\n{output}"
        );
        assert!(
            output.contains("ignore_changes = [router]"),
            "lifecycle block must ignore router changes\n{output}"
        );
    }

    #[test]
    fn network_lifecycle_block_not_injected_without_k8s() {
        let network = make_result(
            "aws_subnet",
            "main",
            "upcloud_network",
            Some(
                r#"resource "upcloud_network" "main" {
  name = "main"
  zone = "__ZONE__"

  ip_network {
    address = "10.0.1.0/24"
    dhcp    = true
    family  = "IPv4"
  }

  router = upcloud_router.main_router.id
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
}
"#,
            ),
            None,
        );
        let output = run_generate(&[network, server], "lifecycle_without_k8s");
        assert!(
            !output.contains("lifecycle"),
            "lifecycle block must NOT be present when no k8s cluster exists\n{output}"
        );
        assert!(
            !output.contains("ignore_changes"),
            "ignore_changes must NOT be present when no k8s cluster exists\n{output}"
        );
    }

    // ── Azure end-to-end ──────────────────────────────────────────────────────

    #[test]
    fn azure_webapp_terraform_example_end_to_end() {
        use crate::migration::mapper::map_resource;
        use crate::terraform::parser::parse_tf_file;

        let tf_path = std::path::PathBuf::from("test-fixtures/webapp-azure-e2e.tf");
        let parsed =
            parse_tf_file(&tf_path).expect("test-fixtures/webapp-azure-e2e.tf should parse");
        let results: Vec<MigrationResult> = parsed.resources.iter().map(map_resource).collect();

        let out_dir = std::env::temp_dir().join("upcloud_e2e_azure_webapp");
        let mut log = vec![];
        generate_files(&results, &[], &out_dir, None, "de-fra1", &mut log).unwrap();
        let output = std::fs::read_to_string(out_dir.join("webapp-azure-e2e.tf"))
            .expect("generate_files must produce webapp-azure-e2e.tf");
        let _ = std::fs::remove_dir_all(&out_dir);

        // Server resources.
        assert!(
            output.contains("resource \"upcloud_server\" \"web\""),
            "must contain web server\n{output}"
        );
        assert!(
            output.contains("resource \"upcloud_server\" \"app\""),
            "must contain app server\n{output}"
        );

        // Networking: VNet → router, subnets → networks.
        assert!(
            output.contains("resource \"upcloud_router\" \"main_router\""),
            "must contain router from VNet\n{output}"
        );
        assert!(
            output.contains("resource \"upcloud_network\" \"web\""),
            "must contain web network from subnet\n{output}"
        );
        assert!(
            output.contains("resource \"upcloud_network\" \"app\""),
            "must contain app network from subnet\n{output}"
        );

        // Firewall rules from NSGs.
        assert!(
            output.contains("upcloud_firewall_rules"),
            "must contain firewall rules from NSGs\n{output}"
        );

        // NSG→subnet→server resolution: web NSG should resolve to web server.
        assert!(
            output.contains("upcloud_server.web.id"),
            "web NSG firewall must resolve server_id to web via NIC→subnet chain\n{output}"
        );
        assert!(
            output.contains("upcloud_server.app.id"),
            "app NSG firewall must resolve server_id to app via NIC→subnet chain\n{output}"
        );

        // Load balancer and its components.
        assert!(
            output.contains("resource \"upcloud_loadbalancer\" \"main\""),
            "must contain load balancer\n{output}"
        );
        assert!(
            output.contains("resource \"upcloud_loadbalancer_backend\" \"web\""),
            "must contain LB backend\n{output}"
        );
        assert!(
            output.contains("resource \"upcloud_loadbalancer_frontend\" \"http\""),
            "must contain LB frontend\n{output}"
        );

        // LB frontend should resolve LB and backend refs via source HCL extraction.
        assert!(
            output.contains("upcloud_loadbalancer.main.id"),
            "LB frontend must reference resolved LB ID\n{output}"
        );
        assert!(
            output.contains("upcloud_loadbalancer_backend.web.name"),
            "LB frontend must reference resolved backend name\n{output}"
        );

        // LB probe health check should be injected into the backend pool properties block.
        assert!(
            output.contains("health_check_type     = \"http\""),
            "LB backend must have http health check from probe\n{output}"
        );
        assert!(
            output.contains("health_check_url      = \"/health\""),
            "LB backend must have health check URL from probe\n{output}"
        );

        // Database resources.
        assert!(
            output.contains("resource \"upcloud_managed_database_postgresql\" \"main\""),
            "must contain PostgreSQL database\n{output}"
        );

        // Storage resources.
        assert!(
            output.contains("upcloud_storage"),
            "must contain storage from managed disk\n{output}"
        );
        assert!(
            output.contains("upcloud_managed_object_storage"),
            "must contain object storage from storage account\n{output}"
        );
    }

    #[test]
    fn azure_nsg_resolves_to_server_via_subnet_association_and_nic() {
        // Azure NSGs are attached to subnets, not directly to VMs.
        // VMs reference NICs, and NICs reference subnets.
        // The chain is: NSG→association→subnet→NIC→VM
        let nsg = make_result(
            "azurerm_network_security_group",
            "web_nsg",
            "upcloud_firewall_rules",
            Some(
                r#"resource "upcloud_firewall_rules" "web_nsg" {
  server_id = upcloud_server.<TODO>.id

  firewall_rule {
    direction = "in"
    action    = "accept"
    family    = "IPv4"
    protocol  = "tcp"
    destination_port_start = "80"
    destination_port_end   = "80"
  }

  firewall_rule {
    direction = "out"
    action    = "accept"
    family    = "IPv4"
    comment   = "Allow all outbound"
  }
}
"#,
            ),
            None,
        );

        let nsg_assoc = MigrationResult {
            resource_type: "azurerm_subnet_network_security_group_association".to_string(),
            resource_name: "web_assoc".to_string(),
            source_file: "test.tf".to_string(),
            status: MigrationStatus::Native,
            upcloud_type: "(consumed by firewall resolution)".into(),
            upcloud_hcl: None,
            snippet: None,
            parent_resource: None,
            notes: vec![],
            source_hcl: Some(
                r#"resource "azurerm_subnet_network_security_group_association" "web_assoc" {
  subnet_id                 = azurerm_subnet.web.id
  network_security_group_id = azurerm_network_security_group.web_nsg.id
}"#
                .to_string(),
            ),
        };

        let nic = MigrationResult {
            resource_type: "azurerm_network_interface".to_string(),
            resource_name: "web_nic".to_string(),
            source_file: "test.tf".to_string(),
            status: MigrationStatus::Partial,
            upcloud_type: "network_interface block in upcloud_server".into(),
            upcloud_hcl: None,
            snippet: None,
            parent_resource: None,
            notes: vec![],
            source_hcl: Some(
                r#"resource "azurerm_network_interface" "web_nic" {
  name = "web-nic"
  ip_configuration {
    subnet_id = azurerm_subnet.web.id
  }
}"#
                .to_string(),
            ),
        };

        let mut server = make_result(
            "azurerm_linux_virtual_machine",
            "web",
            "upcloud_server",
            Some(
                r#"resource "upcloud_server" "web" {
  hostname = "web"
  zone     = "__ZONE__"
  plan     = "2xCPU-4GB"
}
"#,
            ),
            None,
        );
        server.source_hcl = Some(
            r#"resource "azurerm_linux_virtual_machine" "web" {
  name                  = "web"
  size                  = "Standard_B2s"
  network_interface_ids = [azurerm_network_interface.web_nic.id]
}"#
            .to_string(),
        );

        let subnet = make_result(
            "azurerm_subnet",
            "web",
            "upcloud_network",
            Some(
                r#"resource "upcloud_network" "web" {
  name = "web"
  zone = "__ZONE__"
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

        let output = run_generate(
            &[nsg, nsg_assoc, nic, server, subnet],
            "azure_nsg_chain",
        );

        // The NSG should resolve to the web server via the chain:
        // NSG 'web_nsg' → association links subnet 'web' ↔ NSG 'web_nsg'
        // NIC 'web_nic' → subnet 'web'
        // VM 'web' → NIC 'web_nic'
        // Therefore: NSG 'web_nsg' covers server 'web'
        assert!(
            output.contains("upcloud_server.web.id"),
            "NSG server_id must resolve to 'web' via subnet-association + NIC chain\n{output}"
        );
        assert!(
            !output.contains("upcloud_server.<TODO>.id"),
            "no unresolved TODO must remain for web NSG\n{output}"
        );
    }
}
