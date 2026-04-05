//! Azure-specific source HCL extraction and output processing helpers.
//!
//! These functions are used by [`AzureSourceProvider`](super::AzureSourceProvider) to
//! implement the [`SourceProvider`](crate::migration::providers::SourceProvider) trait.

use crate::migration::generator::inside_todo_marker;
use crate::migration::providers::shared;

// ── Source HCL extraction helpers ─────────────────────────────────────────────

/// Scan an Azure VM source HCL block and return all NSG resource names
/// referenced in `network_security_group_id` attributes.
pub fn extract_nsg_refs_from_instance_hcl(hcl: &str) -> Vec<String> {
    let mut refs = Vec::new();
    const PREFIX: &str = "azurerm_network_security_group.";
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

/// Extract the subnet name from an Azure VM or NIC source HCL block.
/// Returns the subnet resource name from `subnet_id = azurerm_subnet.NAME.id`.
pub fn extract_subnet_from_instance_hcl(hcl: &str) -> Option<String> {
    for line in hcl.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("subnet_id") && trimmed.contains('=') {
            if let Some(pos) = trimmed.find("azurerm_subnet.") {
                let after = &trimmed[pos + "azurerm_subnet.".len()..];
                let name = after.split('.').next().unwrap_or("").trim_matches('"');
                if !name.is_empty() {
                    return Some(name.to_string());
                }
            }
        }
    }
    None
}

/// Extract NIC resource names from an Azure VM's `network_interface_ids` attribute.
pub fn extract_nic_refs_from_instance_hcl(hcl: &str) -> Vec<String> {
    let mut refs = Vec::new();
    const PREFIX: &str = "azurerm_network_interface.";
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

/// Parse an `azurerm_subnet_network_security_group_association` source HCL block.
/// Returns `(subnet_name, nsg_name)` if both can be extracted.
pub fn extract_nsg_subnet_association_hcl(hcl: &str) -> Option<(String, String)> {
    let mut subnet_name: Option<String> = None;
    let mut nsg_name: Option<String> = None;
    for line in hcl.lines() {
        let trimmed = line.trim();
        if subnet_name.is_none() && trimmed.starts_with("subnet_id") {
            if let Some(pos) = trimmed.find("azurerm_subnet.") {
                let after = &trimmed[pos + "azurerm_subnet.".len()..];
                let name = after.split('.').next().unwrap_or("").trim_matches('"');
                if !name.is_empty() {
                    subnet_name = Some(name.to_string());
                }
            }
        }
        if nsg_name.is_none() && trimmed.starts_with("network_security_group_id") {
            if let Some(pos) = trimmed.find("azurerm_network_security_group.") {
                let after =
                    &trimmed[pos + "azurerm_network_security_group.".len()..];
                let name = after.split('.').next().unwrap_or("").trim_matches('"');
                if !name.is_empty() {
                    nsg_name = Some(name.to_string());
                }
            }
        }
    }
    match (subnet_name, nsg_name) {
        (Some(s), Some(n)) => Some((s, n)),
        _ => None,
    }
}

/// Extract (backend_pool_name, server_name) from an `azurerm_lb_backend_address_pool_association` source HCL.
pub fn extract_backend_pool_server_from_association_hcl(
    hcl: &str,
) -> Option<(String, String)> {
    let mut pool_name: Option<String> = None;
    let mut server_name: Option<String> = None;
    for line in hcl.lines() {
        let trimmed = line.trim();
        if pool_name.is_none()
            && trimmed.starts_with("backend_address_pool_id")
        {
            if let Some(pos) = trimmed.find("azurerm_lb_backend_address_pool.") {
                let after = &trimmed[pos + "azurerm_lb_backend_address_pool.".len()..];
                let name = after.split('.').next().unwrap_or("").trim_matches('"');
                if !name.is_empty() {
                    pool_name = Some(name.to_string());
                }
            }
        }
        if server_name.is_none() && trimmed.starts_with("network_interface_id") {
            for prefix in &[
                "azurerm_network_interface.",
            ] {
                if let Some(pos) = trimmed.find(prefix) {
                    let after = &trimmed[pos + prefix.len()..];
                    let name: String = after
                        .chars()
                        .take_while(|c| c.is_alphanumeric() || *c == '_')
                        .collect();
                    if !name.is_empty() {
                        server_name = Some(name);
                        break;
                    }
                }
            }
        }
    }
    match (pool_name, server_name) {
        (Some(pool), Some(srv)) => Some((pool, srv)),
        _ => None,
    }
}

/// Extract backend pool name from an `azurerm_lb_rule` source HCL.
pub fn extract_backend_pool_from_lb_rule_hcl(hcl: &str) -> Option<String> {
    const PREFIX: &str = "azurerm_lb_backend_address_pool.";
    for line in hcl.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("backend_address_pool_ids") || trimmed.starts_with("backend_address_pool_id") {
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

/// Extract LB resource name from an `azurerm_lb_rule` source HCL.
pub fn extract_lb_name_from_rule_hcl(hcl: &str) -> Option<String> {
    hcl.lines()
        .find(|l| l.trim().starts_with("loadbalancer_id"))
        .and_then(|l| l.find("azurerm_lb.").map(|p| &l[p + "azurerm_lb.".len()..]))
        .and_then(|s| s.split('.').next())
        .map(|s| s.trim_matches('"').to_string())
        .filter(|s| !s.is_empty())
}

/// Extract (backend_pool_name, probe_name) from an `azurerm_lb_rule` source HCL.
/// Used to chain probe health check properties into the backend pool during generation.
pub fn extract_probe_and_backend_from_lb_rule_hcl(hcl: &str) -> Option<(String, String)> {
    let backend = extract_backend_pool_from_lb_rule_hcl(hcl)?;
    let probe_name = hcl
        .lines()
        .find(|l| {
            let t = l.trim();
            t.starts_with("probe_id") && t.contains("azurerm_lb_probe.")
        })
        .and_then(|l| {
            l.find("azurerm_lb_probe.").map(|pos| {
                let after = &l[pos + "azurerm_lb_probe.".len()..];
                after
                    .chars()
                    .take_while(|c| c.is_alphanumeric() || *c == '_')
                    .collect::<String>()
            })
        })
        .filter(|s| !s.is_empty())?;
    Some((backend, probe_name))
}

/// Extract health check property lines from an `azurerm_lb_probe` source HCL block.
/// Returns the `health_check_*` property lines suitable for an UpCloud backend `properties {}` block.
pub fn extract_probe_health_check_props_hcl(hcl: &str) -> String {
    let protocol = extract_hcl_attr(hcl, "protocol").unwrap_or_default();
    let hc_type = match protocol.to_lowercase().as_str() {
        "http" => "http",
        "https" => "https",
        _ => "tcp",
    };
    let mut props = format!("    health_check_type     = \"{hc_type}\"\n");

    if matches!(hc_type, "http" | "https") {
        if let Some(path) = extract_hcl_attr(hcl, "request_path") {
            props.push_str(&format!("    health_check_url      = \"{path}\"\n"));
        }
    }
    if let Some(interval) = extract_hcl_attr(hcl, "interval_in_seconds") {
        if let Ok(v) = interval.parse::<u32>() {
            props.push_str(&format!("    health_check_interval = {v}\n"));
        }
    }
    if let Some(n) = extract_hcl_attr(hcl, "number_of_probes") {
        if let Ok(v) = n.parse::<u32>() {
            props.push_str(&format!("    health_check_rise     = {v}\n"));
            props.push_str(&format!("    health_check_fall     = {v}\n"));
        }
    }
    props
}

/// Extract a simple `attr = "value"` or `attr = value` from source HCL.
fn extract_hcl_attr(hcl: &str, attr: &str) -> Option<String> {
    hcl.lines()
        .find(|l| {
            let t = l.trim();
            t.starts_with(attr)
                && t[attr.len()..].trim_start().starts_with('=')
        })
        .and_then(|l| l.find('=').map(|pos| l[pos + 1..].trim().trim_matches('"').to_string()))
        .filter(|s| !s.is_empty())
}

// ── Output reference sanitization and rewriting ───────────────────────────────

/// Replace Azure-specific references that leaked into output HCL.
pub fn sanitize_azure_refs(s: String) -> String {
    shared::sanitize_provider_refs(s, "azurerm_", "azurerm_", "Azure")
}

/// Map an Azure resource type name to its UpCloud equivalent.
pub fn upcloud_type_for_azure(azure_type: &str) -> Option<&'static str> {
    match azure_type {
        "azurerm_linux_virtual_machine" | "azurerm_windows_virtual_machine"
        | "azurerm_virtual_machine" => Some("upcloud_server"),
        "azurerm_lb" => Some("upcloud_loadbalancer"),
        "azurerm_virtual_network" => Some("upcloud_router"),
        "azurerm_subnet" => Some("upcloud_network"),
        "azurerm_network_security_group" => Some("upcloud_firewall_rules"),
        "azurerm_postgresql_server" | "azurerm_postgresql_flexible_server" => {
            Some("upcloud_managed_database_postgresql")
        }
        "azurerm_mysql_server" | "azurerm_mysql_flexible_server" => {
            Some("upcloud_managed_database_mysql")
        }
        "azurerm_redis_cache" => Some("upcloud_managed_database_valkey"),
        "azurerm_storage_account" => Some("upcloud_managed_object_storage"),
        "azurerm_storage_share" | "azurerm_storage_share_file" => Some("upcloud_file_storage"),
        "azurerm_linux_virtual_machine_scale_set" => Some("upcloud_server"),
        "azurerm_kubernetes_cluster" => Some("upcloud_kubernetes_cluster"),
        "azurerm_public_ip" => Some("upcloud_floating_ip_address"),
        _ => None,
    }
}

/// Map an Azure resource type name to its UpCloud resource name.
pub fn upcloud_resource_name_for(azure_type: &str, resource_name: &str) -> String {
    match azure_type {
        "azurerm_virtual_network" => format!("{}_router", resource_name),
        _ => resource_name.to_string(),
    }
}

/// Map an Azure attribute name to its UpCloud equivalent for a given UpCloud type.
pub fn upcloud_attr_for(upcloud_type: &str, azure_attr: &str) -> Option<&'static str> {
    match (upcloud_type, azure_attr) {
        ("upcloud_server", "id") => Some("id"),
        ("upcloud_server", "public_ip_address") => Some("network_interface[0].ip_address"),
        ("upcloud_server", "private_ip_address") => Some("network_interface[1].ip_address"),
        ("upcloud_loadbalancer", "id") => Some("id"),
        ("upcloud_router", "id") => Some("id"),
        ("upcloud_network", "id") => Some("id"),
        ("upcloud_firewall_rules", "id") => Some("id"),
        (t, "id") if t.starts_with("upcloud_managed_database") => Some("id"),
        (t, "fqdn") if t.starts_with("upcloud_managed_database") => Some("service_host"),
        (t, "hostname") if t.starts_with("upcloud_managed_database") => Some("service_host"),
        (t, "port") if t.starts_with("upcloud_managed_database") => Some("service_port"),
        ("upcloud_kubernetes_cluster", "id") => Some("id"),
        ("upcloud_kubernetes_cluster", "name") => Some("name"),
        ("upcloud_floating_ip_address", "id") => Some("id"),
        ("upcloud_floating_ip_address", "ip_address") => Some("ip_address"),
        ("upcloud_managed_object_storage", "name") => Some("name"),
        ("upcloud_file_storage", "name") => Some("name"),
        _ => None,
    }
}

/// Rewrite Azure resource references in output/locals blocks to UpCloud equivalents.
pub fn rewrite_output_refs(s: &str) -> String {
    let mut result = s.to_string();
    let mut search_from = 0usize;

    while let Some(rel) = result[search_from..].find("azurerm_") {
        let start = search_from + rel;

        if inside_todo_marker(&result, start) {
            search_from = start + 8;
            continue;
        }

        let end = result[start..]
            .find(|c: char| !c.is_alphanumeric() && !matches!(c, '.' | '_' | '[' | ']' | '*'))
            .map(|off| start + off)
            .unwrap_or(result.len());

        let candidate = &result[start..end];

        if candidate.matches('.').count() < 2 {
            search_from = end;
            continue;
        }

        // Parse: azurerm_TYPE.NAME.ATTR
        let parts: Vec<&str> = candidate.splitn(3, '.').collect();
        if parts.len() < 3 {
            search_from = end;
            continue;
        }

        let azure_type = parts[0]; // e.g. azurerm_linux_virtual_machine
        let resource_name = parts[1];
        let attr_trail = parts[2]; // e.g. id, public_ip_address
        let attr = attr_trail.split('[').next().unwrap_or(attr_trail);

        if let Some(uc_type) = upcloud_type_for_azure(azure_type) {
            let uc_name = upcloud_resource_name_for(azure_type, resource_name);
            if let Some(uc_attr) = upcloud_attr_for(uc_type, attr) {
                let replacement = format!("{}.{}.{}", uc_type, uc_name, uc_attr);
                result = format!("{}{}{}", &result[..start], replacement, &result[end..]);
                search_from = start + replacement.len();
            } else {
                let replacement = format!(
                    "{}.{}.<TODO: was .{}>",
                    uc_type, uc_name, attr_trail
                );
                result = format!("{}{}{}", &result[..start], replacement, &result[end..]);
                search_from = start + replacement.len();
            }
        } else if azure_type == "azurerm_resource_group" {
            let replacement = "null # Azure resource group: no UpCloud equivalent";
            result = format!("{}{}{}", &result[..start], replacement, &result[end..]);
            search_from = start + replacement.len();
        } else {
            let replacement = format!("<TODO: was {}>", candidate);
            result = format!("{}{}{}", &result[..start], replacement, &result[end..]);
            search_from = start + replacement.len();
        }
    }

    result
}

// ── Application Gateway sub-resource generation ──────────────────────────────

/// Convert a hyphenated Azure name to a valid Terraform resource identifier.
fn tf_id(name: &str) -> String {
    name.replace('-', "_")
}

/// Extract all named blocks of `block_type` from Application Gateway source HCL.
///
/// Scans for lines where `block_type {` is the only non-whitespace content, then
/// extracts the brace-balanced block content and the `name` attribute from it.
/// Returns `Vec<(name, block_content)>`.
fn extract_appgw_named_blocks(hcl: &str, block_type: &str) -> Vec<(String, String)> {
    let mut results = Vec::new();
    let needle = format!("{} {{", block_type);
    let mut search_start = 0usize;
    let bytes = hcl.as_bytes();

    while let Some(rel) = hcl[search_start..].find(needle.as_str()) {
        let abs = search_start + rel;

        // Only match when block_type starts at the beginning of an (indented) line.
        let line_start = hcl[..abs].rfind('\n').map_or(0, |p| p + 1);
        if !hcl[line_start..abs].chars().all(|c| c == ' ' || c == '\t') {
            search_start = abs + 1;
            continue;
        }

        // Position of the opening '{' (last char of needle).
        let brace_open = abs + needle.len() - 1;
        let mut depth = 1i32;
        let mut i = brace_open + 1;

        while i < bytes.len() {
            match bytes[i] {
                b'{' => depth += 1,
                b'}' => {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
                _ => {}
            }
            i += 1;
        }

        let content = &hcl[brace_open + 1..i];
        if let Some(name) = extract_hcl_attr(content, "name") {
            results.push((name, content.to_string()));
        }
        search_start = i + 1;
    }

    results
}

/// Extract `upcloud_network.SUBNET_NAME.id` from the AppGw `gateway_ip_configuration` subnet.
pub fn extract_appgw_subnet_ref(hcl: &str) -> Option<String> {
    for line in hcl.lines() {
        let t = line.trim();
        if t.starts_with("subnet_id") && t.contains("azurerm_subnet.") {
            let pos = t.find("azurerm_subnet.")?;
            let after = &t[pos + "azurerm_subnet.".len()..];
            let name: String = after
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_')
                .collect();
            if !name.is_empty() {
                return Some(format!("upcloud_network.{}.id", name));
            }
        }
    }
    None
}

/// Extract health check properties from an AppGw inline `probe` block.
/// Uses AppGw attribute names (`path`, `interval`, `unhealthy_threshold`) which differ
/// from `azurerm_lb_probe` (`request_path`, `interval_in_seconds`, `number_of_probes`).
fn appgw_probe_health_check_props(block_content: &str) -> String {
    let protocol = extract_hcl_attr(block_content, "protocol").unwrap_or_default();
    let hc_type = match protocol.to_lowercase().as_str() {
        "http" => "http",
        "https" => "https",
        _ => "tcp",
    };
    let mut props = format!("    health_check_type     = \"{hc_type}\"\n");
    if matches!(hc_type, "http" | "https") {
        if let Some(path) = extract_hcl_attr(block_content, "path") {
            props.push_str(&format!("    health_check_url      = \"{path}\"\n"));
        }
    }
    if let Some(iv) = extract_hcl_attr(block_content, "interval") {
        if let Ok(v) = iv.parse::<u32>() {
            props.push_str(&format!("    health_check_interval = {v}\n"));
        }
    }
    if let Some(n) = extract_hcl_attr(block_content, "unhealthy_threshold") {
        if let Ok(v) = n.parse::<u32>() {
            props.push_str(&format!("    health_check_fall     = {v}\n"));
        }
    }
    props
}

/// Generate UpCloud LB sub-resources for an `azurerm_application_gateway`.
///
/// Parses inline blocks (backend pools, probes, listeners, routing rules, TLS certs)
/// and emits:
/// - `upcloud_loadbalancer_backend` (with health checks from probe chain)
/// - `upcloud_loadbalancer_frontend` (from http_listener + request_routing_rule)
/// - `upcloud_loadbalancer_frontend_tls_config` (for HTTPS listeners)
/// - `upcloud_loadbalancer_frontend_rule` (for redirect configurations)
pub fn generate_appgw_sub_resources(lb_name: &str, hcl: &str) -> String {
    use std::collections::{HashMap, HashSet};

    // frontend_port name → port number
    let mut port_map: HashMap<String, u32> = HashMap::new();
    for (name, content) in extract_appgw_named_blocks(hcl, "frontend_port") {
        if let Some(s) = extract_hcl_attr(&content, "port") {
            if let Ok(p) = s.parse::<u32>() {
                port_map.insert(name, p);
            }
        }
    }

    // probe name → health check properties string
    let probe_map: HashMap<String, String> = extract_appgw_named_blocks(hcl, "probe")
        .into_iter()
        .map(|(name, content)| (name, appgw_probe_health_check_props(&content)))
        .collect();

    // backend_http_settings name → probe name
    let settings_probe_map: HashMap<String, String> =
        extract_appgw_named_blocks(hcl, "backend_http_settings")
            .into_iter()
            .filter_map(|(name, content)| {
                extract_hcl_attr(&content, "probe_name").map(|p| (name, p))
            })
            .collect();

    // routing rules: (name, listener_name, backend_pool?, http_settings?, redirect_cfg?)
    let routing_rules: Vec<(String, String, Option<String>, Option<String>, Option<String>)> =
        extract_appgw_named_blocks(hcl, "request_routing_rule")
            .into_iter()
            .map(|(name, content)| {
                let listener =
                    extract_hcl_attr(&content, "http_listener_name").unwrap_or_default();
                let backend = extract_hcl_attr(&content, "backend_address_pool_name");
                let settings = extract_hcl_attr(&content, "backend_http_settings_name");
                let redirect = extract_hcl_attr(&content, "redirect_configuration_name");
                (name, listener, backend, settings, redirect)
            })
            .collect();

    // http_listener name → (port_name, protocol, ssl_cert?, host_name?)
    let listener_map: HashMap<String, (String, String, Option<String>, Option<String>)> =
        extract_appgw_named_blocks(hcl, "http_listener")
            .into_iter()
            .map(|(name, content)| {
                let port_name =
                    extract_hcl_attr(&content, "frontend_port_name").unwrap_or_default();
                let protocol =
                    extract_hcl_attr(&content, "protocol").unwrap_or_else(|| "Http".into());
                let ssl_cert = extract_hcl_attr(&content, "ssl_certificate_name");
                let host_name = extract_hcl_attr(&content, "host_name");
                (name, (port_name, protocol, ssl_cert, host_name))
            })
            .collect();

    // backend_address_pool names in source order
    let backend_pools: Vec<String> = extract_appgw_named_blocks(hcl, "backend_address_pool")
        .into_iter()
        .map(|(name, _)| name)
        .collect();

    // backend_pool → health check props (via routing rule → http_settings → probe)
    let mut backend_health_map: HashMap<String, String> = HashMap::new();
    for (_, _, backend_opt, settings_opt, _) in &routing_rules {
        if let (Some(backend), Some(settings)) = (backend_opt, settings_opt) {
            let probe_props = settings_probe_map
                .get(settings)
                .and_then(|p| probe_map.get(p))
                .cloned()
                .unwrap_or_else(|| "    health_check_type = \"tcp\"\n".into());
            backend_health_map.insert(backend.clone(), probe_props);
        }
    }

    let mut out = String::new();

    // ── Backends (source order) ───────────────────────────────────────────────
    for backend_name in &backend_pools {
        let health = backend_health_map
            .get(backend_name)
            .cloned()
            .unwrap_or_else(|| "    health_check_type = \"tcp\"\n".into());
        out.push_str(&format!(
            "resource \"upcloud_loadbalancer_backend\" \"{id}\" {{\n  loadbalancer = upcloud_loadbalancer.{lb}.id\n  name         = \"{name}\"\n\n  properties {{\n{health}  }}\n}}\n\n",
            id = tf_id(backend_name),
            lb = lb_name,
            name = backend_name,
            health = health,
        ));
    }

    // ── Frontends (one per routing rule) ─────────────────────────────────────
    let mut seen_tls_certs: HashSet<String> = HashSet::new();
    for (_, listener_name, backend_opt, _, redirect_opt) in &routing_rules {
        if listener_name.is_empty() {
            continue;
        }
        let (port_name, protocol, ssl_cert_opt, _host) = match listener_map.get(listener_name) {
            Some(v) => v,
            None => continue,
        };
        let port = port_map.get(port_name).copied().unwrap_or(80);
        let lid = tf_id(listener_name);

        if let Some(backend_name) = backend_opt {
            let bid = tf_id(backend_name);
            out.push_str(&format!(
                "resource \"upcloud_loadbalancer_frontend\" \"{lid}\" {{\n  loadbalancer         = upcloud_loadbalancer.{lb}.id\n  name                 = \"{name}\"\n  mode                 = \"http\"\n  port                 = {port}\n  default_backend_name = upcloud_loadbalancer_backend.{bid}.name\n\n  networks {{\n    name = \"public\"\n  }}\n}}\n\n",
                lid = lid,
                lb = lb_name,
                name = listener_name,
                port = port,
                bid = bid,
            ));
            if protocol.to_lowercase() == "https" {
                if let Some(cert_name) = ssl_cert_opt {
                    let cid = tf_id(cert_name);
                    seen_tls_certs.insert(cert_name.clone());
                    out.push_str(&format!(
                        "resource \"upcloud_loadbalancer_frontend_tls_config\" \"{lid}_tls\" {{\n  frontend           = upcloud_loadbalancer_frontend.{lid}.name\n  name               = \"{cert_name}\"\n  certificate_bundle = \"<TODO: upcloud_tls_certificate.{cid}.id>\"\n}}\n\n",
                        lid = lid,
                        cert_name = cert_name,
                        cid = cid,
                    ));
                }
            }
        } else if let Some(redirect_cfg) = redirect_opt {
            out.push_str(&format!(
                "# NOTE: HTTP\u{2192}HTTPS redirect \u{2014} the rule below handles all traffic.\nresource \"upcloud_loadbalancer_frontend\" \"{lid}\" {{\n  loadbalancer         = upcloud_loadbalancer.{lb}.id\n  name                 = \"{name}\"\n  mode                 = \"http\"\n  port                 = {port}\n  default_backend_name = \"<TODO: required \u{2014} assign a backend (redirect rule handles all traffic)>\"\n\n  networks {{\n    name = \"public\"\n  }}\n}}\n\n",
                lid = lid,
                lb = lb_name,
                name = listener_name,
                port = port,
            ));
            out.push_str(&format!(
                "resource \"upcloud_loadbalancer_frontend_rule\" \"{lid}_redirect\" {{\n  frontend = upcloud_loadbalancer_frontend.{lid}.name\n  name     = \"{cfg_name}\"\n  priority = 100\n\n  actions {{\n    http_redirect {{\n      scheme = \"https\"\n    }}\n  }}\n\n  matchers {{}}\n}}\n\n",
                lid = lid,
                cfg_name = redirect_cfg,
            ));
        }
    }

    // TLS certificate placeholder (informational, once per unique cert name)
    if !seen_tls_certs.is_empty() {
        out.push_str("# NOTE: provide SSL/TLS certificates for the frontends above.\n");
        let mut sorted_certs: Vec<_> = seen_tls_certs.iter().collect();
        sorted_certs.sort();
        for cert_name in sorted_certs {
            out.push_str(&format!(
                "# resource \"upcloud_tls_certificate\" \"{cid}\" {{\n#   # certificate, private_key (PEM format) \u{2014} was: {cert_name}\n# }}\n",
                cid = tf_id(cert_name),
                cert_name = cert_name,
            ));
        }
        out.push('\n');
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── extract_nsg_refs_from_instance_hcl ────────────────────────────────────

    #[test]
    fn extracts_nsg_name_from_instance_hcl() {
        let hcl = concat!(
            "resource \"azurerm_linux_virtual_machine\" \"web\" {\n",
            "  network_security_group_id = azurerm_network_security_group.web_nsg.id\n",
            "}\n"
        );
        let refs = extract_nsg_refs_from_instance_hcl(hcl);
        assert!(refs.contains(&"web_nsg".to_string()), "should extract NSG name: {refs:?}");
    }

    #[test]
    fn extracts_multiple_nsg_refs() {
        let hcl = concat!(
            "  network_security_group_id = azurerm_network_security_group.nsg_a.id\n",
            "  network_security_group_id = azurerm_network_security_group.nsg_b.id\n",
        );
        let refs = extract_nsg_refs_from_instance_hcl(hcl);
        assert!(refs.contains(&"nsg_a".to_string()), "{refs:?}");
        assert!(refs.contains(&"nsg_b".to_string()), "{refs:?}");
    }

    #[test]
    fn returns_empty_when_no_nsg_refs() {
        let refs = extract_nsg_refs_from_instance_hcl("resource \"azurerm_linux_virtual_machine\" \"web\" {}");
        assert!(refs.is_empty());
    }

    // ── extract_subnet_from_instance_hcl ─────────────────────────────────────

    #[test]
    fn extracts_subnet_name_from_instance_hcl() {
        let hcl = concat!(
            "resource \"azurerm_linux_virtual_machine\" \"web\" {\n",
            "  network_interface_ids = [azurerm_network_interface.nic.id]\n",
            "}\n",
            "resource \"azurerm_network_interface\" \"nic\" {\n",
            "  ip_configuration {\n",
            "    subnet_id = azurerm_subnet.app_subnet.id\n",
            "  }\n",
            "}\n"
        );
        let result = extract_subnet_from_instance_hcl(hcl);
        assert_eq!(result, Some("app_subnet".to_string()));
    }

    #[test]
    fn returns_none_when_no_subnet_ref() {
        let result = extract_subnet_from_instance_hcl("resource \"azurerm_linux_virtual_machine\" \"web\" {}");
        assert!(result.is_none());
    }

    // ── extract_backend_pool_server_from_association_hcl ──────────────────────

    #[test]
    fn extracts_pool_and_server_from_association_hcl() {
        let hcl = concat!(
            "resource \"azurerm_lb_backend_address_pool_association\" \"assoc\" {\n",
            "  backend_address_pool_id = azurerm_lb_backend_address_pool.pool.id\n",
            "  network_interface_id    = azurerm_network_interface.web_nic.id\n",
            "}\n"
        );
        let result = extract_backend_pool_server_from_association_hcl(hcl);
        assert_eq!(result, Some(("pool".to_string(), "web_nic".to_string())));
    }

    #[test]
    fn returns_none_when_association_hcl_incomplete() {
        let result = extract_backend_pool_server_from_association_hcl("resource {} ");
        assert!(result.is_none());
    }

    // ── extract_backend_pool_from_lb_rule_hcl ────────────────────────────────

    #[test]
    fn extracts_backend_pool_from_lb_rule_hcl() {
        let hcl = concat!(
            "resource \"azurerm_lb_rule\" \"rule\" {\n",
            "  backend_address_pool_ids = [azurerm_lb_backend_address_pool.pool.id]\n",
            "}\n"
        );
        let result = extract_backend_pool_from_lb_rule_hcl(hcl);
        assert_eq!(result, Some("pool".to_string()));
    }

    // ── extract_lb_name_from_rule_hcl ─────────────────────────────────────────

    #[test]
    fn extracts_lb_name_from_rule_hcl() {
        let hcl = concat!(
            "resource \"azurerm_lb_rule\" \"rule\" {\n",
            "  loadbalancer_id = azurerm_lb.main.id\n",
            "}\n"
        );
        let result = extract_lb_name_from_rule_hcl(hcl);
        assert_eq!(result, Some("main".to_string()));
    }

    // ── sanitize_azure_refs ───────────────────────────────────────────────────

    #[test]
    fn sanitize_removes_azurerm_resource_refs() {
        let input = "value = azurerm_linux_virtual_machine.web.id".to_string();
        let out = sanitize_azure_refs(input);
        assert!(out.contains("<TODO: remove Azure resource ref>"), "{out}");
        assert!(!out.contains("azurerm_linux_virtual_machine"), "{out}");
    }

    #[test]
    fn sanitize_removes_data_source_refs() {
        let input = "value = data.azurerm_subscription.current.id".to_string();
        let out = sanitize_azure_refs(input);
        assert!(out.contains("<TODO: remove Azure data source ref>"), "{out}");
    }

    #[test]
    fn sanitize_plain_azurerm_type_without_dots_is_not_replaced() {
        // A bare resource type name (no dot, not a reference) should remain unchanged
        let input = "resource \"azurerm_subnet\" \"s\" {}".to_string();
        let out = sanitize_azure_refs(input.clone());
        assert!(!out.contains("<TODO: remove Azure resource ref>"), "{out}");
    }

    // ── upcloud_type_for_azure ────────────────────────────────────────────────

    #[test]
    fn azure_vm_maps_to_upcloud_server() {
        assert_eq!(
            upcloud_type_for_azure("azurerm_linux_virtual_machine"),
            Some("upcloud_server")
        );
        assert_eq!(
            upcloud_type_for_azure("azurerm_windows_virtual_machine"),
            Some("upcloud_server")
        );
    }

    #[test]
    fn azure_lb_maps_to_upcloud_loadbalancer() {
        assert_eq!(upcloud_type_for_azure("azurerm_lb"), Some("upcloud_loadbalancer"));
    }

    #[test]
    fn azure_vnet_maps_to_upcloud_router() {
        assert_eq!(upcloud_type_for_azure("azurerm_virtual_network"), Some("upcloud_router"));
    }

    #[test]
    fn azure_subnet_maps_to_upcloud_network() {
        assert_eq!(upcloud_type_for_azure("azurerm_subnet"), Some("upcloud_network"));
    }

    #[test]
    fn azure_redis_maps_to_upcloud_valkey() {
        assert_eq!(
            upcloud_type_for_azure("azurerm_redis_cache"),
            Some("upcloud_managed_database_valkey")
        );
    }

    #[test]
    fn unknown_azure_type_returns_none() {
        assert_eq!(upcloud_type_for_azure("azurerm_something_unknown"), None);
    }

    // ── upcloud_attr_for ──────────────────────────────────────────────────────

    #[test]
    fn server_id_maps_to_id() {
        assert_eq!(upcloud_attr_for("upcloud_server", "id"), Some("id"));
    }

    #[test]
    fn server_public_ip_maps_to_network_interface_0() {
        assert_eq!(
            upcloud_attr_for("upcloud_server", "public_ip_address"),
            Some("network_interface[0].ip_address")
        );
    }

    #[test]
    fn managed_db_fqdn_maps_to_service_host() {
        assert_eq!(
            upcloud_attr_for("upcloud_managed_database_postgresql", "fqdn"),
            Some("service_host")
        );
    }

    #[test]
    fn unknown_attr_returns_none() {
        assert_eq!(upcloud_attr_for("upcloud_server", "nonexistent_attr"), None);
    }

    // ── rewrite_output_refs for azurerm_resource_group ────────────────────────

    #[test]
    fn rewrite_output_refs_resource_group_becomes_null() {
        let input = "value = azurerm_resource_group.main.name";
        let result = rewrite_output_refs(input);
        assert!(result.contains("null"), "{result}");
        assert!(result.contains("no UpCloud equivalent"), "{result}");
        assert!(!result.contains("<TODO:"), "{result}");
    }

    // ── Application Gateway helpers ───────────────────────────────────────────

    #[test]
    fn appgw_named_blocks_extracted_correctly() {
        let hcl = concat!(
            "  probe {\n",
            "    name     = \"my-probe\"\n",
            "    protocol = \"Http\"\n",
            "  }\n",
        );
        let blocks = extract_appgw_named_blocks(hcl, "probe");
        assert_eq!(blocks.len(), 1, "{blocks:?}");
        assert_eq!(blocks[0].0, "my-probe");
    }

    #[test]
    fn appgw_named_blocks_skips_non_block_occurrences() {
        // "probe_name = ..." should not be treated as a probe block
        let hcl = concat!(
            "  backend_http_settings {\n",
            "    name       = \"settings\"\n",
            "    probe_name = \"my-probe\"\n",
            "  }\n",
        );
        let blocks = extract_appgw_named_blocks(hcl, "probe");
        assert!(blocks.is_empty(), "should not match probe_name: {blocks:?}");
    }

    #[test]
    fn appgw_subnet_ref_extracted() {
        let hcl = concat!(
            "  gateway_ip_configuration {\n",
            "    name      = \"gw-ip-config\"\n",
            "    subnet_id = azurerm_subnet.appgw.id\n",
            "  }\n",
        );
        assert_eq!(
            extract_appgw_subnet_ref(hcl),
            Some("upcloud_network.appgw.id".to_string())
        );
    }

    #[test]
    fn appgw_sub_resources_empty_hcl_returns_empty() {
        assert!(generate_appgw_sub_resources("main", "").is_empty());
    }

    #[test]
    fn appgw_sub_resources_backends_with_health_check() {
        let hcl = concat!(
            "  backend_address_pool {\n",
            "    name = \"web-pool\"\n",
            "  }\n",
            "  probe {\n",
            "    name     = \"web-probe\"\n",
            "    protocol = \"Http\"\n",
            "    path     = \"/health\"\n",
            "    interval = 15\n",
            "  }\n",
            "  backend_http_settings {\n",
            "    name       = \"web-settings\"\n",
            "    probe_name = \"web-probe\"\n",
            "  }\n",
            "  request_routing_rule {\n",
            "    name                       = \"rule1\"\n",
            "    http_listener_name         = \"web-listener\"\n",
            "    backend_address_pool_name  = \"web-pool\"\n",
            "    backend_http_settings_name = \"web-settings\"\n",
            "  }\n",
            "  http_listener {\n",
            "    name               = \"web-listener\"\n",
            "    frontend_port_name = \"http-port\"\n",
            "    protocol           = \"Http\"\n",
            "  }\n",
            "  frontend_port {\n",
            "    name = \"http-port\"\n",
            "    port = 80\n",
            "  }\n",
        );
        let result = generate_appgw_sub_resources("main", hcl);
        assert!(result.contains("upcloud_loadbalancer_backend"), "{result}");
        assert!(result.contains("health_check_type     = \"http\""), "{result}");
        assert!(result.contains("health_check_url      = \"/health\""), "{result}");
        assert!(result.contains("upcloud_loadbalancer_backend.web_pool.name"), "{result}");
    }

    #[test]
    fn appgw_sub_resources_https_generates_tls_config() {
        let hcl = concat!(
            "  backend_address_pool {\n",
            "    name = \"api-pool\"\n",
            "  }\n",
            "  backend_http_settings {\n",
            "    name = \"api-settings\"\n",
            "  }\n",
            "  ssl_certificate {\n",
            "    name = \"my-cert\"\n",
            "  }\n",
            "  request_routing_rule {\n",
            "    name                       = \"rule1\"\n",
            "    http_listener_name         = \"api-listener\"\n",
            "    backend_address_pool_name  = \"api-pool\"\n",
            "    backend_http_settings_name = \"api-settings\"\n",
            "  }\n",
            "  http_listener {\n",
            "    name                 = \"api-listener\"\n",
            "    frontend_port_name   = \"https-port\"\n",
            "    protocol             = \"Https\"\n",
            "    ssl_certificate_name = \"my-cert\"\n",
            "  }\n",
            "  frontend_port {\n",
            "    name = \"https-port\"\n",
            "    port = 443\n",
            "  }\n",
        );
        let result = generate_appgw_sub_resources("main", hcl);
        assert!(result.contains("upcloud_loadbalancer_frontend_tls_config"), "{result}");
        assert!(result.contains("my-cert"), "{result}");
        assert!(result.contains("upcloud_tls_certificate"), "{result}");
    }

    #[test]
    fn appgw_sub_resources_redirect_generates_frontend_rule() {
        let hcl = concat!(
            "  http_listener {\n",
            "    name               = \"http-listener\"\n",
            "    frontend_port_name = \"http-port\"\n",
            "    protocol           = \"Http\"\n",
            "  }\n",
            "  request_routing_rule {\n",
            "    name                        = \"redirect-rule\"\n",
            "    http_listener_name          = \"http-listener\"\n",
            "    redirect_configuration_name = \"http-to-https\"\n",
            "  }\n",
            "  frontend_port {\n",
            "    name = \"http-port\"\n",
            "    port = 80\n",
            "  }\n",
        );
        let result = generate_appgw_sub_resources("main", hcl);
        assert!(result.contains("upcloud_loadbalancer_frontend_rule"), "{result}");
        assert!(result.contains("http_redirect"), "{result}");
        assert!(result.contains("scheme = \"https\""), "{result}");
    }
}
