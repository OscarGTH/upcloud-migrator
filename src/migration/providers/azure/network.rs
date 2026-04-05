use super::super::shared;
use crate::migration::types::{MigrationResult, MigrationStatus};
use crate::terraform::types::TerraformResource;

pub fn map_virtual_network(res: &TerraformResource) -> MigrationResult {
    let hcl = shared::upcloud_router_hcl(&res.name);

    MigrationResult {
        resource_type: res.resource_type.clone(),
        resource_name: res.name.clone(),
        source_file: res.source_file.display().to_string(),
        status: MigrationStatus::Compatible,
        upcloud_type: "upcloud_router".into(),
        upcloud_hcl: Some(hcl),
        snippet: None,
        parent_resource: None,
        notes: vec![
            "Azure VNet → UpCloud Router. Each subnet becomes a separate upcloud_network attached to this router.".into(),
        ],
        source_hcl: None,
    }
}

pub fn map_subnet(res: &TerraformResource) -> MigrationResult {
    let cidr = res
        .attributes
        .get("address_prefixes")
        .or_else(|| res.attributes.get("address_prefix"))
        .map(|c| {
            // address_prefixes is a list; extract the first element
            let c = c.trim_matches(|ch: char| ch == '"' || ch == '[' || ch == ']');
            c.split(',')
                .next()
                .unwrap_or(c)
                .trim()
                .trim_matches('"')
                .to_string()
        })
        .unwrap_or_else(|| "10.0.1.0/24".into());

    // Parse the VNet resource name from a reference like "azurerm_virtual_network.main.name"
    let vnet_name = res.attributes.get("virtual_network_name").and_then(|v| {
        let v = v.trim_matches('"');
        let parts: Vec<&str> = v.splitn(3, '.').collect();
        if parts.len() >= 2 && parts[0] == "azurerm_virtual_network" {
            Some(parts[1].to_string())
        } else {
            None
        }
    });

    let router_ref = vnet_name
        .as_deref()
        .map(|n| format!("upcloud_router.{}_router.id", n))
        .unwrap_or_else(|| "\"<TODO: router id>\"".to_string());

    let hcl = format!(
        r#"resource "upcloud_network" "{name}" {{
  name = "{name}"
  zone = "__ZONE__"

  ip_network {{
    address            = "{cidr}"
    dhcp               = true
    dhcp_default_route = false
    family             = "IPv4"
  }}

  router = {router_ref}
}}
"#,
        name = res.name,
        cidr = cidr,
        router_ref = router_ref,
    );

    MigrationResult {
        resource_type: res.resource_type.clone(),
        resource_name: res.name.clone(),
        source_file: res.source_file.display().to_string(),
        status: MigrationStatus::Compatible,
        upcloud_type: "upcloud_network".into(),
        upcloud_hcl: Some(hcl),
        snippet: None,
        parent_resource: vnet_name,
        notes: vec![
            "Azure Subnet → upcloud_network (private SDN; public internet via server network_interface type=public).".into(),
        ],
        source_hcl: None,
    }
}

pub fn map_network_security_group(res: &TerraformResource) -> MigrationResult {
    let rules = parse_nsg_rules(&res.raw_hcl);
    let mut rule_blocks = String::new();
    let mut has_egress_allow_all = false;

    for (direction, from_port, to_port, protocol, description, is_all_traffic) in &rules {
        if *direction == "out" && *is_all_traffic {
            has_egress_allow_all = true;
        }
        rule_blocks.push_str(&shared::build_firewall_rule(
            direction,
            *from_port,
            *to_port,
            protocol,
            description.as_deref(),
            *is_all_traffic,
        ));
        rule_blocks.push('\n');
    }

    if !has_egress_allow_all {
        rule_blocks.push_str(shared::FIREWALL_CATCHALL_EGRESS);
    }

    let status = if rules.is_empty() {
        MigrationStatus::Partial
    } else {
        MigrationStatus::Compatible
    };

    let hcl = shared::upcloud_firewall_rules_hcl(&res.name, &rule_blocks);

    let mut notes = vec![
        "Network Security Group → UpCloud Firewall Rules (attached per-server, not per-network)".into(),
    ];
    if rules.is_empty() {
        notes.push("No security rules found — add firewall_rule blocks manually.".into());
    } else {
        notes.push(format!(
            "{} rule(s) auto-generated from source security_rule blocks.",
            rules.len()
        ));
    }

    MigrationResult {
        resource_type: res.resource_type.clone(),
        resource_name: res.name.clone(),
        source_file: res.source_file.display().to_string(),
        status,
        upcloud_type: "upcloud_firewall_rules".into(),
        upcloud_hcl: Some(hcl),
        snippet: None,
        parent_resource: None,
        notes,
        source_hcl: None,
    }
}

pub fn map_network_security_rule(res: &TerraformResource) -> MigrationResult {
    let direction = res
        .attributes
        .get("direction")
        .map(|d| {
            if d.trim_matches('"').eq_ignore_ascii_case("inbound") {
                "in"
            } else {
                "out"
            }
        })
        .unwrap_or("in");

    let from_port = res
        .attributes
        .get("destination_port_range")
        .and_then(|v| {
            let v = v.trim_matches('"');
            if v == "*" {
                return Some(0);
            }
            v.split('-').next().and_then(|p| p.parse::<i32>().ok())
        })
        .unwrap_or(0);
    let to_port = res
        .attributes
        .get("destination_port_range")
        .and_then(|v| {
            let v = v.trim_matches('"');
            if v == "*" {
                return Some(65535);
            }
            v.split('-').last().and_then(|p| p.parse::<i32>().ok())
        })
        .unwrap_or(65535);
    let protocol = res
        .attributes
        .get("protocol")
        .map(|v| v.trim_matches('"').to_lowercase())
        .unwrap_or_else(|| "*".into());

    let is_all_traffic = protocol == "*" || (from_port == 0 && to_port == 65535);

    let nsg_name = res
        .attributes
        .get("network_security_group_name")
        .and_then(|v| {
            let v = v.trim_matches('"');
            if v.starts_with("azurerm_network_security_group.") {
                v.split('.').nth(1).map(str::to_string)
            } else {
                None
            }
        });

    let rule_block = shared::build_firewall_rule(
        direction,
        from_port,
        to_port,
        &protocol,
        res.attributes
            .get("description")
            .map(|v| v.trim_matches('"')),
        is_all_traffic,
    );

    let target = nsg_name
        .as_deref()
        .map(|n| format!("\"upcloud_firewall_rules\" \"{}\"", n))
        .unwrap_or_else(|| "\"upcloud_firewall_rules\" \"<TODO: nsg_name>\"".into());

    let snippet = format!("# Add to resource {} {{\n{}\n}}", target, rule_block);

    MigrationResult {
        resource_type: res.resource_type.clone(),
        resource_name: res.name.clone(),
        source_file: res.source_file.display().to_string(),
        status: MigrationStatus::Partial,
        upcloud_type: "firewall_rule block in upcloud_firewall_rules".into(),
        upcloud_hcl: None,
        snippet: Some(snippet),
        parent_resource: nsg_name,
        notes: vec![
            "Standalone security rule → add firewall_rule block to the parent upcloud_firewall_rules resource.".into(),
        ],
        source_hcl: None,
    }
}

pub fn map_public_ip(res: &TerraformResource) -> MigrationResult {
    let hcl = format!(
        r#"resource "upcloud_floating_ip_address" "{name}" {{
  mac_address = upcloud_server.<TODO>.network_interface[0].mac_address
  zone        = "__ZONE__"
}}
"#,
        name = res.name,
    );

    MigrationResult {
        resource_type: res.resource_type.clone(),
        resource_name: res.name.clone(),
        source_file: res.source_file.display().to_string(),
        status: MigrationStatus::Compatible,
        upcloud_type: "upcloud_floating_ip_address".into(),
        upcloud_hcl: Some(hcl),
        snippet: None,
        parent_resource: None,
        notes: vec![
            "Azure Public IP → UpCloud Floating IP. Attach via mac_address to the target server.".into(),
        ],
        source_hcl: None,
    }
}

pub fn map_network_interface(res: &TerraformResource) -> MigrationResult {
    let subnet_name = res
        .attributes
        .get("ip_configuration.subnet_id")
        .and_then(|v| {
            let v = v.trim_matches('"');
            if v.starts_with("azurerm_subnet.") {
                v.split('.').nth(1).map(str::to_string)
            } else {
                None
            }
        });

    let network_ref = subnet_name
        .as_deref()
        .map(|n| format!("upcloud_network.{}.id", n))
        .unwrap_or_else(|| "\"<TODO: upcloud_network UUID>\"".into());

    let snippet = format!(
        "# Add to resource \"upcloud_server\" \"<TODO: server_name>\" {{\n  network_interface {{\n    type    = \"private\"\n    network = {network_ref}\n  }}\n}}",
    );

    MigrationResult {
        resource_type: res.resource_type.clone(),
        resource_name: res.name.clone(),
        source_file: res.source_file.display().to_string(),
        status: MigrationStatus::Partial,
        upcloud_type: "network_interface block in upcloud_server".into(),
        upcloud_hcl: None,
        snippet: Some(snippet),
        parent_resource: subnet_name,
        notes: vec![
            "UpCloud has no standalone NIC resource — network interfaces are blocks within upcloud_server.".into(),
        ],
        source_hcl: None,
    }
}

/// Map an `azurerm_subnet_network_security_group_association` to a silent consumed result.
///
/// This resource is a join table that links subnets to NSGs. It has no standalone
/// UpCloud equivalent but its source HCL is consumed by the generator's cross-reference
/// resolution to discover which servers each security group covers.
pub fn map_subnet_nsg_association(res: &TerraformResource) -> MigrationResult {
    MigrationResult {
        resource_type: res.resource_type.clone(),
        resource_name: res.name.clone(),
        source_file: res.source_file.display().to_string(),
        status: MigrationStatus::Native,
        upcloud_type: "(consumed by firewall resolution)".into(),
        upcloud_hcl: None,
        snippet: None,
        parent_resource: None,
        notes: vec![],
        source_hcl: None,
    }
}

/// Parse security_rule blocks from a raw `resource "azurerm_network_security_group" ...` HCL string.
fn parse_nsg_rules(raw_hcl: &str) -> Vec<(String, i32, i32, String, Option<String>, bool)> {
    let Ok(body) = hcl::from_str::<hcl::Body>(raw_hcl) else {
        return vec![];
    };

    let mut rules = Vec::new();
    for outer in body.blocks() {
        for block in outer.body().blocks() {
            if block.identifier() != "security_rule" {
                continue;
            }

            let mut direction = String::from("in");
            let mut from_port = 0i32;
            let mut to_port = 65535i32;
            let mut protocol = String::from("*");
            let mut description: Option<String> = None;
            let mut access = String::from("Allow");

            for attr in block.body().attributes() {
                let val = format!("{}", attr.expr());
                let bare = val.trim_matches('"');
                match attr.key() {
                    "direction" => {
                        direction = if bare.eq_ignore_ascii_case("inbound") {
                            "in".into()
                        } else {
                            "out".into()
                        };
                    }
                    "destination_port_range" => {
                        if bare == "*" {
                            from_port = 0;
                            to_port = 65535;
                        } else if let Some((f, t)) = bare.split_once('-') {
                            from_port = f.parse().unwrap_or(0);
                            to_port = t.parse().unwrap_or(65535);
                        } else {
                            let port = bare.parse().unwrap_or(0);
                            from_port = port;
                            to_port = port;
                        }
                    }
                    "protocol" => protocol = bare.to_lowercase(),
                    "description" => description = Some(bare.to_string()),
                    "access" => access = bare.to_string(),
                    _ => {}
                }
            }

            if !access.eq_ignore_ascii_case("Allow") {
                continue; // UpCloud firewall rules only support allow actions
            }

            let is_all_traffic = protocol == "*" || (from_port == 0 && to_port == 65535);
            rules.push((direction, from_port, to_port, protocol, description, is_all_traffic));
        }
    }
    rules
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn make_res(resource_type: &str, name: &str, attrs: &[(&str, &str)]) -> TerraformResource {
        TerraformResource {
            resource_type: resource_type.to_string(),
            name: name.to_string(),
            attributes: attrs
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            source_file: PathBuf::from("test.tf"),
            raw_hcl: String::new(),
        }
    }

    // ── map_virtual_network ───────────────────────────────────────────────────

    #[test]
    fn vnet_generates_router_only() {
        let res = make_res("azurerm_virtual_network", "main", &[]);
        let r = map_virtual_network(&res);
        assert_eq!(r.upcloud_type, "upcloud_router");
        let hcl = r.upcloud_hcl.unwrap();
        assert!(hcl.contains("resource \"upcloud_router\" \"main_router\""), "{hcl}");
        assert!(!hcl.contains("upcloud_network"), "VNet must not produce upcloud_network\n{hcl}");
        assert!(!hcl.contains("ip_network"), "{hcl}");
    }

    #[test]
    fn vnet_status_is_compatible() {
        let res = make_res("azurerm_virtual_network", "vnet", &[]);
        assert_eq!(
            map_virtual_network(&res).status,
            crate::migration::types::MigrationStatus::Compatible
        );
    }

    // ── map_subnet ────────────────────────────────────────────────────────────

    #[test]
    fn subnet_generates_upcloud_network_with_cidr() {
        let res = make_res(
            "azurerm_subnet",
            "app_subnet",
            &[("address_prefixes", "10.0.1.0/24")],
        );
        let r = map_subnet(&res);
        assert_eq!(r.upcloud_type, "upcloud_network");
        let hcl = r.upcloud_hcl.unwrap();
        assert!(hcl.contains("resource \"upcloud_network\" \"app_subnet\""), "{hcl}");
        assert!(hcl.contains("10.0.1.0/24"), "{hcl}");
    }

    #[test]
    fn subnet_with_vnet_ref_links_router() {
        let res = make_res(
            "azurerm_subnet",
            "web",
            &[
                ("address_prefixes", "10.0.2.0/24"),
                ("virtual_network_name", "azurerm_virtual_network.main.name"),
            ],
        );
        let hcl = map_subnet(&res).upcloud_hcl.unwrap();
        assert!(hcl.contains("upcloud_router.main_router.id"), "{hcl}");
    }

    #[test]
    fn subnet_without_vnet_has_router_todo() {
        let res = make_res(
            "azurerm_subnet",
            "orphan",
            &[("address_prefixes", "10.0.3.0/24")],
        );
        let hcl = map_subnet(&res).upcloud_hcl.unwrap();
        assert!(hcl.contains("<TODO: router id>"), "{hcl}");
    }

    #[test]
    fn subnet_has_exactly_one_ip_network_block() {
        let res = make_res(
            "azurerm_subnet",
            "s",
            &[("address_prefixes", "10.0.4.0/24")],
        );
        let hcl = map_subnet(&res).upcloud_hcl.unwrap();
        assert_eq!(hcl.matches("ip_network").count(), 1, "must have exactly 1 ip_network block\n{hcl}");
    }

    #[test]
    fn subnet_snippet_is_none() {
        let res = make_res("azurerm_subnet", "s", &[("address_prefixes", "10.0.5.0/24")]);
        assert!(map_subnet(&res).snippet.is_none(), "subnets are standalone resources, not snippets");
    }

    // ── map_network_security_group ─────────────────────────────────────────────

    #[test]
    fn nsg_generates_upcloud_firewall_rules() {
        let res = make_res("azurerm_network_security_group", "nsg", &[]);
        let r = map_network_security_group(&res);
        assert_eq!(r.upcloud_type, "upcloud_firewall_rules");
        let hcl = r.upcloud_hcl.unwrap();
        assert!(hcl.contains("resource \"upcloud_firewall_rules\" \"nsg\""), "{hcl}");
    }

    #[test]
    fn nsg_always_includes_default_egress_allow_all() {
        let res = make_res("azurerm_network_security_group", "nsg", &[]);
        let hcl = map_network_security_group(&res).upcloud_hcl.unwrap();
        assert!(hcl.contains("direction = \"out\""), "{hcl}");
        assert!(hcl.contains("action    = \"accept\""), "{hcl}");
    }

    // ── map_public_ip ─────────────────────────────────────────────────────────

    #[test]
    fn public_ip_generates_floating_ip() {
        let res = make_res("azurerm_public_ip", "myip", &[]);
        let r = map_public_ip(&res);
        assert_eq!(r.upcloud_type, "upcloud_floating_ip_address");
        let hcl = r.upcloud_hcl.unwrap();
        assert!(hcl.contains("resource \"upcloud_floating_ip_address\" \"myip\""), "{hcl}");
    }
}
