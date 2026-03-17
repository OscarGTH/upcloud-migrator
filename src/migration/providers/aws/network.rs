use crate::migration::types::{MigrationResult, MigrationStatus};
use crate::terraform::types::TerraformResource;

pub fn map_vpc(res: &TerraformResource) -> MigrationResult {
    // AWS VPC → UpCloud Router only.
    // Each subnet maps to its own upcloud_network resource (provider allows exactly 1 ip_network
    // block per upcloud_network, so subnets cannot be injected as ip_network blocks).
    let hcl = format!(
        r#"resource "upcloud_router" "{name}_router" {{
  name = "{name}-router"
}}
"#,
        name = res.name,
    );

    MigrationResult {
        resource_type: res.resource_type.clone(),
        resource_name: res.name.clone(),
        source_file: res.source_file.display().to_string(),
        status: MigrationStatus::Compatible,
        score: 72,
        upcloud_type: "upcloud_router".into(),
        upcloud_hcl: Some(hcl),
        snippet: None,
        parent_resource: None,
        notes: vec![
            "AWS VPC → UpCloud Router. Each subnet becomes a separate upcloud_network attached to this router.".into(),
        ],
        source_hcl: None,
    }
}

pub fn map_subnet(res: &TerraformResource) -> MigrationResult {
    let cidr = res.attributes.get("cidr_block")
        .map(|c| c.trim_matches('"').to_string())
        .unwrap_or_else(|| "10.0.1.0/24".into());

    // Parse the VPC resource name from a reference like "aws_vpc.main.id"
    let vpc_name = res.attributes.get("vpc_id").and_then(|v| {
        let v = v.trim_matches('"');
        let parts: Vec<&str> = v.splitn(3, '.').collect();
        if parts.len() >= 2 && parts[0] == "aws_vpc" {
            Some(parts[1].to_string())
        } else {
            None
        }
    });

    // Reference the router created by the parent VPC mapping
    let router_ref = vpc_name.as_deref()
        .map(|n| format!("upcloud_router.{}_router.id", n))
        .unwrap_or_else(|| "\"<TODO: router id>\"".to_string());

    // Propagate count if the source subnet used count (e.g. count = 2 with dynamic CIDRs)
    let count_attr = res.attributes.get("count").map(|v| v.trim_matches('"').to_string());
    let count_line = match &count_attr {
        Some(n) => format!("  count = {}\n", n),
        None    => String::new(),
    };
    // With count the name must be unique per instance
    let name_val = if count_attr.is_some() {
        format!("{}-${{count.index + 1}}", res.name)
    } else {
        res.name.clone()
    };

    let hcl = format!(
        r#"resource "upcloud_network" "{name}" {{
{count_line}  name = "{name_val}"
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
        count_line = count_line,
        name_val = name_val,
        cidr = cidr,
        router_ref = router_ref,
    );

    MigrationResult {
        resource_type: res.resource_type.clone(),
        resource_name: res.name.clone(),
        source_file: res.source_file.display().to_string(),
        status: MigrationStatus::Compatible,
        score: 80,
        upcloud_type: "upcloud_network".into(),
        upcloud_hcl: Some(hcl),
        snippet: None,
        parent_resource: vpc_name,
        notes: {
            let mut n = vec![
                "AWS Subnet → upcloud_network (private SDN; public internet via server network_interface type=public).".into(),
            ];
            let is_public = res.attributes.get("map_public_ip_on_launch")
                .map(|v| v.trim_matches('"') == "true")
                .unwrap_or(false);
            if is_public {
                n.push("map_public_ip_on_launch ignored — public access is via server network_interface type=public.".into());
            }
            if let Some(ref c) = count_attr {
                n.push(format!("count = {} propagated.", c));
            }
            n
        },
        source_hcl: None,
    }
}

pub fn map_security_group(res: &TerraformResource) -> MigrationResult {
    let rules = parse_sg_rules(&res.raw_hcl);
    let mut rule_blocks = String::new();
    let mut has_egress_allow_all = false;

    for (direction, from_port, to_port, protocol, description, is_all_traffic) in &rules {
        if *direction == "out" && *is_all_traffic {
            has_egress_allow_all = true;
        }
        rule_blocks.push_str(&build_firewall_rule(
            direction, *from_port, *to_port, protocol,
            description.as_deref(), *is_all_traffic,
        ));
        rule_blocks.push('\n');
    }

    // Always include a catch-all outbound rule if none was generated
    if !has_egress_allow_all {
        rule_blocks.push_str(
            "  firewall_rule {\n    direction = \"out\"\n    action    = \"accept\"\n    family    = \"IPv4\"\n    comment   = \"Allow all outbound\"\n  }\n"
        );
    }

    let (score, status) = if rules.is_empty() {
        (65u8, MigrationStatus::Partial)
    } else {
        (82u8, MigrationStatus::Compatible)
    };

    let hcl = format!(
        "resource \"upcloud_firewall_rules\" \"{name}\" {{\n  server_id = upcloud_server.<TODO>.id\n\n{rules}}}\n",
        name = res.name,
        rules = rule_blocks,
    );

    let mut notes = vec![
        "Security groups → UpCloud Firewall Rules (attached per-server, not per-network)".into(),
    ];
    if rules.is_empty() {
        notes.push("No ingress/egress rules — add firewall_rule blocks manually.".into());
    } else {
        notes.push(format!("{} rule(s) auto-generated from source ingress/egress blocks.", rules.len()));
    }
    notes.push("server_id auto-resolved from vpc_security_group_ids on the instance.".into());

    MigrationResult {
        resource_type: res.resource_type.clone(),
        resource_name: res.name.clone(),
        source_file: res.source_file.display().to_string(),
        status,
        score,
        upcloud_type: "upcloud_firewall_rules".into(),
        upcloud_hcl: Some(hcl),
        snippet: None,
        parent_resource: None,
        notes,
        source_hcl: None,
    }
}

/// Parse ingress/egress blocks from a raw `resource "aws_security_group" ...` HCL string.
/// Returns a list of (direction, from_port, to_port, protocol, description, is_all_traffic).
fn parse_sg_rules(raw_hcl: &str) -> Vec<(String, i32, i32, String, Option<String>, bool)> {
    let Ok(body) = hcl::from_str::<hcl::Body>(raw_hcl) else {
        return vec![];
    };

    let mut rules = Vec::new();
    for outer in body.blocks() {
        for block in outer.body().blocks() {
            let direction = match block.identifier() {
                "ingress" => "in",
                "egress" => "out",
                _ => continue,
            };

            let mut from_port = 0i32;
            let mut to_port = 65535i32;
            let mut protocol = String::from("-1");
            let mut description: Option<String> = None;
            let mut cidr_all = true; // assume allow-all unless a specific CIDR is found

            for attr in block.body().attributes() {
                let val = format!("{}", attr.expr());
                let bare = val.trim_matches('"');
                match attr.key() {
                    "from_port" => from_port = bare.parse().unwrap_or(0),
                    "to_port" => to_port = bare.parse().unwrap_or(65535),
                    "protocol" => protocol = bare.to_string(),
                    "description" => description = Some(bare.to_string()),
                    "cidr_blocks" | "ipv6_cidr_blocks" => {
                        // If it's not the universal CIDR, flag it
                        if !val.contains("0.0.0.0/0") && !val.contains("::/0") {
                            cidr_all = false;
                        }
                    }
                    _ => {}
                }
            }

            let is_all_traffic = protocol == "-1" || (from_port == 0 && to_port == 0);
            rules.push((direction.to_string(), from_port, to_port, protocol, description, is_all_traffic && cidr_all));
        }
    }
    rules
}

fn build_firewall_rule(
    direction: &str,
    from_port: i32,
    to_port: i32,
    protocol: &str,
    description: Option<&str>,
    is_all_traffic: bool,
) -> String {
    let mut s = String::from("  firewall_rule {\n");
    s.push_str(&format!("    direction = \"{}\"\n", direction));
    s.push_str("    action    = \"accept\"\n");
    s.push_str("    family    = \"IPv4\"\n");

    if !is_all_traffic && protocol != "-1" {
        let proto = match protocol {
            "tcp" => "tcp",
            "udp" => "udp",
            "icmp" | "1" => "icmp",
            _ => "tcp",
        };
        s.push_str(&format!("    protocol  = \"{}\"\n", proto));
        if from_port > 0 || to_port < 65535 {
            s.push_str(&format!("    destination_port_start = \"{}\"\n", from_port));
            s.push_str(&format!("    destination_port_end   = \"{}\"\n", to_port));
        }
    }

    if let Some(desc) = description {
        if !desc.is_empty() {
            s.push_str(&format!("    comment = \"{}\"\n", desc));
        }
    }

    s.push_str("  }");
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn make_res(resource_type: &str, name: &str, attrs: &[(&str, &str)]) -> TerraformResource {
        TerraformResource {
            resource_type: resource_type.to_string(),
            name: name.to_string(),
            attributes: attrs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect(),
            source_file: PathBuf::from("test.tf"),
            raw_hcl: String::new(),
        }
    }

    // ── map_vpc ───────────────────────────────────────────────────────────────

    #[test]
    fn vpc_generates_router_only() {
        let res = make_res("aws_vpc", "main", &[("cidr_block", "10.0.0.0/16")]);
        let r = map_vpc(&res);
        assert_eq!(r.upcloud_type, "upcloud_router");
        let hcl = r.upcloud_hcl.unwrap();
        assert!(hcl.contains("resource \"upcloud_router\" \"main_router\""), "{hcl}");
        // must NOT generate an upcloud_network block (only 1 ip_network allowed per network)
        assert!(!hcl.contains("upcloud_network"), "VPC must not produce an upcloud_network\n{hcl}");
        assert!(!hcl.contains("ip_network"), "{hcl}");
    }

    #[test]
    fn vpc_status_is_compatible() {
        let res = make_res("aws_vpc", "main", &[]);
        assert_eq!(map_vpc(&res).status, crate::migration::types::MigrationStatus::Compatible);
    }

    // ── map_subnet ────────────────────────────────────────────────────────────

    #[test]
    fn subnet_generates_standalone_upcloud_network() {
        let res = make_res("aws_subnet", "pub", &[
            ("cidr_block", "10.0.1.0/24"),
            ("vpc_id", "aws_vpc.main.id"),
        ]);
        let r = map_subnet(&res);
        assert_eq!(r.upcloud_type, "upcloud_network");
        let hcl = r.upcloud_hcl.unwrap();
        assert!(hcl.contains("resource \"upcloud_network\" \"pub\""), "{hcl}");
        assert!(hcl.contains("address            = \"10.0.1.0/24\""), "{hcl}");
        // snippet must be None (no injection into parent)
        assert!(r.snippet.is_none(), "subnets are no longer injected as snippets");
    }

    #[test]
    fn subnet_references_parent_router() {
        let res = make_res("aws_subnet", "priv", &[
            ("cidr_block", "10.0.2.0/24"),
            ("vpc_id", "aws_vpc.main.id"),
        ]);
        let hcl = map_subnet(&res).upcloud_hcl.unwrap();
        assert!(hcl.contains("upcloud_router.main_router.id"), "{hcl}");
    }

    #[test]
    fn subnet_without_vpc_id_has_router_todo() {
        let res = make_res("aws_subnet", "orphan", &[("cidr_block", "10.0.3.0/24")]);
        let hcl = map_subnet(&res).upcloud_hcl.unwrap();
        assert!(hcl.contains("<TODO: router id>"), "{hcl}");
    }

    #[test]
    fn subnet_with_count_generates_count_line() {
        let res = make_res("aws_subnet", "public", &[
            ("cidr_block", "10.0.${count.index + 1}.0/24"),
            ("vpc_id", "aws_vpc.main.id"),
            ("count", "2"),
        ]);
        let hcl = map_subnet(&res).upcloud_hcl.unwrap();
        assert!(hcl.contains("count = 2"), "count should be propagated\n{hcl}");
    }

    #[test]
    fn subnet_with_count_uses_index_in_name() {
        let res = make_res("aws_subnet", "public", &[
            ("cidr_block", "10.0.0.0/24"),
            ("vpc_id", "aws_vpc.main.id"),
            ("count", "3"),
        ]);
        let hcl = map_subnet(&res).upcloud_hcl.unwrap();
        assert!(hcl.contains("${count.index + 1}"), "name should use count.index\n{hcl}");
        assert!(!hcl.contains("name = \"public\"\n"), "bare name should not appear when count is set\n{hcl}");
    }

    #[test]
    fn subnet_has_exactly_one_ip_network_block() {
        let res = make_res("aws_subnet", "s", &[
            ("cidr_block", "10.0.4.0/24"),
            ("vpc_id", "aws_vpc.main.id"),
        ]);
        let hcl = map_subnet(&res).upcloud_hcl.unwrap();
        let count = hcl.matches("ip_network").count();
        assert_eq!(count, 1, "must have exactly 1 ip_network block\n{hcl}");
    }

    // ── map_security_group ────────────────────────────────────────────────────

    #[test]
    fn security_group_generates_firewall_rules() {
        let raw = r#"resource "aws_security_group" "web" {
  ingress {
    from_port   = 80
    to_port     = 80
    protocol    = "tcp"
    cidr_blocks = ["0.0.0.0/0"]
    description = "HTTP"
  }
  egress {
    from_port   = 0
    to_port     = 0
    protocol    = "-1"
    cidr_blocks = ["0.0.0.0/0"]
  }
}"#;
        let mut res = make_res("aws_security_group", "web", &[]);
        res.raw_hcl = raw.to_string();
        let r = map_security_group(&res);
        assert_eq!(r.upcloud_type, "upcloud_firewall_rules");
        let hcl = r.upcloud_hcl.unwrap();
        assert!(hcl.contains("resource \"upcloud_firewall_rules\" \"web\""), "{hcl}");
        assert!(hcl.contains("upcloud_server.<TODO>.id"), "server_id needs TODO before cross-resolve\n{hcl}");
        assert!(hcl.contains("direction = \"in\""), "{hcl}");
        assert!(hcl.contains("direction = \"out\""), "{hcl}");
        assert!(hcl.contains("destination_port_start = \"80\""), "{hcl}");
        assert!(hcl.contains("destination_port_end   = \"80\""), "end must equal start for single-port rules\n{hcl}");
    }

    #[test]
    fn security_group_without_rules_has_catchall_outbound() {
        let res = make_res("aws_security_group", "empty", &[]);
        let hcl = map_security_group(&res).upcloud_hcl.unwrap();
        // catch-all outbound must always be present
        assert!(hcl.contains("direction = \"out\""), "{hcl}");
        assert!(hcl.contains("action    = \"accept\""), "{hcl}");
    }

    // ── map_internet_gateway ──────────────────────────────────────────────────

    #[test]
    fn internet_gateway_is_partial_no_hcl() {
        let res = make_res("aws_internet_gateway", "igw", &[]);
        let r = map_internet_gateway(&res);
        assert_eq!(r.status, crate::migration::types::MigrationStatus::Partial);
        assert!(r.upcloud_hcl.is_none(), "IGW has no standalone UpCloud resource");
    }

    // ── map_route_table ───────────────────────────────────────────────────────

    #[test]
    fn route_table_with_no_raw_hcl_is_automatic() {
        // When raw_hcl is empty (no custom routes detected), UpCloud handles routing
        let res = make_res("aws_route_table", "rt", &[]);
        let r = map_route_table(&res);
        // No snippet — UpCloud Router handles default routes automatically
        assert!(r.snippet.is_none(), "default-only route table should not generate snippet");
        assert!(r.notes.iter().any(|n| n.contains("automatically")), "should note automatic routing\n{:?}", r.notes);
    }

    #[test]
    fn route_table_with_default_only_route_is_automatic() {
        let mut res = make_res("aws_route_table", "public", &[]);
        res.raw_hcl = r#"resource "aws_route_table" "public" {
  route {
    cidr_block = "0.0.0.0/0"
    gateway_id = aws_internet_gateway.main.id
  }
}"#.to_string();
        let r = map_route_table(&res);
        assert!(r.snippet.is_none(), "default-only route table should not generate snippet\n{:?}", r.notes);
    }

    #[test]
    fn route_table_with_custom_route_produces_snippet() {
        let mut res = make_res("aws_route_table", "vpn", &[]);
        res.raw_hcl = r#"resource "aws_route_table" "vpn" {
  route {
    cidr_block = "10.100.0.0/16"
    gateway_id = aws_vpn_gateway.main.id
  }
}"#.to_string();
        let r = map_route_table(&res);
        let snippet = r.snippet.unwrap();
        assert!(snippet.contains("static_route"), "{snippet}");
        assert!(snippet.contains("\"vpn\""), "{snippet}");
    }

    // ── map_eip ───────────────────────────────────────────────────────────────

    #[test]
    fn eip_without_instance_generates_detached_floating_ip() {
        let res = make_res("aws_eip", "my_ip", &[]);
        let r = map_eip(&res);
        assert_eq!(r.upcloud_type, "upcloud_floating_ip_address");
        let hcl = r.upcloud_hcl.unwrap();
        assert!(hcl.contains("resource \"upcloud_floating_ip_address\" \"my_ip\""), "{hcl}");
        assert!(hcl.contains("zone = \"__ZONE__\""), "{hcl}");
        // Comment mentions mac_address as guidance — but it must not be set as an attribute
        assert!(!hcl.lines().any(|l| !l.trim().starts_with('#') && l.contains("mac_address")),
            "detached EIP should not have mac_address attribute\n{hcl}");
    }

    #[test]
    fn eip_with_instance_generates_attached_floating_ip() {
        let res = make_res("aws_eip", "bastion", &[
            ("instance", "aws_instance.bastion.id"),
        ]);
        let r = map_eip(&res);
        let hcl = r.upcloud_hcl.unwrap();
        assert!(hcl.contains("mac_address = upcloud_server.bastion.network_interface[0].mac_address"), "{hcl}");
        assert!(!hcl.contains("zone"), "attached EIP does not need zone attr\n{hcl}");
    }

    // ── map_eip_association ───────────────────────────────────────────────────

    #[test]
    fn eip_association_generates_mac_address_snippet() {
        let res = make_res("aws_eip_association", "assoc", &[
            ("allocation_id", "aws_eip.my_ip.id"),
            ("instance_id", "aws_instance.web.id"),
        ]);
        let r = map_eip_association(&res);
        assert_eq!(r.upcloud_type, "mac_address on upcloud_floating_ip_address");
        assert!(r.upcloud_hcl.is_none(), "no standalone HCL resource");
        let snippet = r.snippet.unwrap();
        assert!(snippet.contains("mac_address"), "{snippet}");
        assert!(snippet.contains("my_ip"), "should reference the EIP resource name\n{snippet}");
        assert!(snippet.contains("upcloud_server.web"), "{snippet}");
    }

    #[test]
    fn eip_association_without_refs_has_todo_snippet() {
        let res = make_res("aws_eip_association", "a", &[]);
        let snippet = map_eip_association(&res).snippet.unwrap();
        assert!(snippet.contains("<TODO>"), "{snippet}");
        assert!(snippet.contains("mac_address"), "{snippet}");
    }
}

pub fn map_eip_association(res: &TerraformResource) -> MigrationResult {
    // aws_eip_association links an EIP to an instance/interface.
    // UpCloud equivalent: set mac_address on upcloud_floating_ip_address.
    let eip_name = res.attributes.get("allocation_id").and_then(|v| {
        let v = v.trim_matches('"');
        if v.starts_with("aws_eip.") { v.split('.').nth(1).map(str::to_string) } else { None }
    });
    let instance_name = res.attributes.get("instance_id").and_then(|v| {
        let v = v.trim_matches('"');
        if v.starts_with("aws_instance.") { v.split('.').nth(1).map(str::to_string) } else { None }
    });

    let snippet = match (&eip_name, &instance_name) {
        (Some(eip), Some(inst)) => format!(
            "# In resource \"upcloud_floating_ip_address\" \"{eip}\" add:\nmac_address = upcloud_server.{inst}.network_interface[0].mac_address"
        ),
        (Some(eip), None) => format!(
            "# In resource \"upcloud_floating_ip_address\" \"{eip}\" add:\nmac_address = upcloud_server.<TODO>.network_interface[0].mac_address"
        ),
        _ => "# Set mac_address on the upcloud_floating_ip_address to attach it:\nmac_address = upcloud_server.<TODO>.network_interface[0].mac_address".into(),
    };

    MigrationResult {
        resource_type: res.resource_type.clone(),
        resource_name: res.name.clone(),
        source_file: res.source_file.display().to_string(),
        status: MigrationStatus::Partial,
        score: 55,
        upcloud_type: "mac_address on upcloud_floating_ip_address".into(),
        upcloud_hcl: None,
        snippet: Some(snippet),
        parent_resource: eip_name,
        notes: vec![
            "EIP associations → set mac_address on upcloud_floating_ip_address to attach to a server.".into(),
            "mac_address references the server's network_interface[0].mac_address output.".into(),
        ],
        source_hcl: None,
    }
}

pub fn map_nat_gateway(res: &TerraformResource) -> MigrationResult {
    MigrationResult {
        resource_type: res.resource_type.clone(),
        resource_name: res.name.clone(),
        source_file: res.source_file.display().to_string(),
        status: MigrationStatus::Partial,
        score: 50,
        upcloud_type: "upcloud_router (built-in NAT)".into(),
        upcloud_hcl: None,
        snippet: None,
        parent_resource: None,
        notes: vec![
            "UpCloud Router provides NAT automatically — no explicit NAT gateway resource is needed.".into(),
            "Remove this resource; ensure your upcloud_network has a router attached.".into(),
        ],
        source_hcl: None,
    }
}

pub fn map_internet_gateway(res: &TerraformResource) -> MigrationResult {
    MigrationResult {
        resource_type: res.resource_type.clone(),
        resource_name: res.name.clone(),
        source_file: res.source_file.display().to_string(),
        status: MigrationStatus::Partial,
        score: 50,
        upcloud_type: "upcloud_router (default route)".into(),
        upcloud_hcl: None,
        snippet: None,
        parent_resource: None,
        notes: vec!["UpCloud Router handles default routes automatically. No explicit IGW resource is needed.".into()],
        source_hcl: None,
    }
}

pub fn map_route_table(res: &TerraformResource) -> MigrationResult {
    // Route tables and their associations have no standalone UpCloud resource.
    // Static routes are embedded as static_route blocks inside upcloud_router.
    //
    // If the route table only contains default routes (0.0.0.0/0), no action is needed —
    // UpCloud Router provides internet routing and NAT automatically.
    // Only generate a static_route snippet for non-default (custom) routes.
    let has_custom_route = !res.raw_hcl.is_empty() && res.raw_hcl.lines().any(|line| {
        let t = line.trim();
        if !t.starts_with("cidr_block") { return false; }
        // Extract the value: cidr_block = "X.X.X.X/Y"
        let val = t.split('"').nth(1).unwrap_or("");
        !val.is_empty() && val != "0.0.0.0/0" && !val.starts_with(':')
    });

    if has_custom_route {
        let snippet = format!(
            r#"static_route {{
  name    = "{name}"
  nexthop = "<TODO: nexthop IP address>"
  route   = "<TODO: custom CIDR from aws_route_table>"
}}"#,
            name = res.name,
        );
        MigrationResult {
            resource_type: res.resource_type.clone(),
            resource_name: res.name.clone(),
            source_file: res.source_file.display().to_string(),
            status: MigrationStatus::Partial,
            score: 45,
            upcloud_type: "upcloud_router static_route".into(),
            upcloud_hcl: None,
            snippet: Some(snippet),
            parent_resource: None,
            notes: vec![
                "Route table has custom routes → static_route block needed inside upcloud_router.".into(),
                "Add the snippet to your upcloud_router resource and fill in the nexthop IP and CIDR.".into(),
            ],
            source_hcl: None,
        }
    } else {
        // Only default routes (0.0.0.0/0) — UpCloud Router handles these automatically
        MigrationResult {
            resource_type: res.resource_type.clone(),
            resource_name: res.name.clone(),
            source_file: res.source_file.display().to_string(),
            status: MigrationStatus::Partial,
            score: 60,
            upcloud_type: "upcloud_router (automatic routing)".into(),
            upcloud_hcl: None,
            snippet: None,
            parent_resource: None,
            notes: vec![
                "Route table only contains default routes (0.0.0.0/0).".into(),
                "UpCloud Router provides internet routing and NAT automatically — no action required.".into(),
            ],
            source_hcl: None,
        }
    }
}

pub fn map_eip(res: &TerraformResource) -> MigrationResult {
    // Check if the EIP is attached to an instance via the `instance` attribute.
    // aws_eip.bastion { instance = aws_instance.bastion.id }
    // → add mac_address = upcloud_server.bastion.network_interface[0].mac_address
    let instance_ref = res.attributes.get("instance").and_then(|v| {
        let v = v.trim_matches('"');
        if v.starts_with("aws_instance.") {
            v.split('.').nth(1).map(str::to_string)
        } else {
            None
        }
    });

    let (hcl, notes) = if let Some(ref inst) = instance_ref {
        let h = format!(
            r#"resource "upcloud_floating_ip_address" "{name}" {{
  mac_address = upcloud_server.{inst}.network_interface[0].mac_address
}}
"#,
            name = res.name,
            inst = inst,
        );
        let n = vec![
            format!("EIP → upcloud_floating_ip_address attached to upcloud_server.{}.", inst),
            "mac_address auto-resolved from aws_eip.instance attribute.".into(),
        ];
        (h, n)
    } else {
        let h = format!(
            r#"resource "upcloud_floating_ip_address" "{name}" {{
  zone = "__ZONE__"
  # To attach to a server, set:
  # mac_address = upcloud_server.<name>.network_interface[0].mac_address
}}
"#,
            name = res.name,
        );
        let n = vec![
            "EIP → upcloud_floating_ip_address. Set mac_address to attach it to a server's network interface.".into(),
            "Note: this EIP was not attached to an instance (e.g. NAT Gateway EIP) — attach manually if needed.".into(),
        ];
        (h, n)
    };

    MigrationResult {
        resource_type: res.resource_type.clone(),
        resource_name: res.name.clone(),
        source_file: res.source_file.display().to_string(),
        status: MigrationStatus::Compatible,
        score: 55,
        upcloud_type: "upcloud_floating_ip_address".into(),
        upcloud_hcl: Some(hcl),
        snippet: None,
        parent_resource: None,
        notes,
        source_hcl: None,
    }
}
