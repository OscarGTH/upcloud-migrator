//! Azure-specific source HCL extraction and output processing helpers.
//!
//! These functions are used by [`AzureSourceProvider`](super::AzureSourceProvider) to
//! implement the [`SourceProvider`](crate::migration::providers::SourceProvider) trait.

use crate::migration::generator::{inside_todo_marker, remove_todo_interpolations};

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

/// Extract the subnet name from an Azure VM source HCL block.
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

// ── Output reference sanitization and rewriting ───────────────────────────────

/// Replace Azure-specific references that leaked into output HCL.
pub fn sanitize_azure_refs(mut s: String) -> String {
    // data.azurerm_* data source references
    let mut search_from = 0;
    while let Some(rel) = s[search_from..].find("data.azurerm_") {
        let start = search_from + rel;
        if inside_todo_marker(&s, start) {
            search_from = start + "data.azurerm_".len();
            continue;
        }
        let end = s[start..]
            .find(|c: char| !c.is_alphanumeric() && c != '.' && c != '_')
            .map(|off| start + off)
            .unwrap_or(s.len());
        let azure_ref = s[start..end].to_string();
        s = s.replacen(&azure_ref, "<TODO: remove Azure data source ref>", 1);
        search_from = 0;
    }
    // azurerm_type.name.attr resource references
    let mut search_from = 0;
    while let Some(rel) = s[search_from..].find("azurerm_") {
        let start = search_from + rel;
        if inside_todo_marker(&s, start) {
            search_from = start + "azurerm_".len();
            continue;
        }
        let end = s[start..]
            .find(|c: char| !c.is_alphanumeric() && c != '.' && c != '_')
            .map(|off| start + off)
            .unwrap_or(s.len());
        let candidate = &s[start..end];
        if candidate.matches('.').count() >= 1 {
            let owned = candidate.to_string();
            s = s.replacen(&owned, "<TODO: remove Azure resource ref>", 1);
            search_from = 0;
        } else {
            search_from = end;
        }
    }
    s = remove_todo_interpolations(s);
    s
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
        (t, "port") if t.starts_with("upcloud_managed_database") => Some("service_port"),
        ("upcloud_kubernetes_cluster", "id") => Some("id"),
        ("upcloud_kubernetes_cluster", "name") => Some("name"),
        ("upcloud_floating_ip_address", "id") => Some("id"),
        ("upcloud_floating_ip_address", "ip_address") => Some("ip_address"),
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
        } else {
            let replacement = format!("<TODO: was {}>", candidate);
            result = format!("{}{}{}", &result[..start], replacement, &result[end..]);
            search_from = start + replacement.len();
        }
    }

    result
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
}
