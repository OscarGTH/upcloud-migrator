//! HCL helpers and template builders shared across provider modules.

pub fn is_tf_expr(val: &str) -> bool {
    val.starts_with("var.") || val.starts_with("local.")
}

pub fn hcl_value(val: &str) -> String {
    if is_tf_expr(val) {
        val.to_string()
    } else {
        format!("\"{}\"", val)
    }
}

pub const FIREWALL_CATCHALL_EGRESS: &str =
    "  firewall_rule {\n    direction = \"out\"\n    action    = \"accept\"\n    family    = \"IPv4\"\n    comment   = \"Allow all outbound\"\n  }\n";

// When `is_all_traffic` is true, protocol/port constraints are omitted.
// Callers map provider-specific wildcards (AWS `-1`, Azure `*`) to that bool.
pub fn build_firewall_rule(
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

    if !is_all_traffic {
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

    if let Some(desc) = description
        && !desc.is_empty()
    {
        s.push_str(&format!("    comment = \"{}\"\n", desc));
    }

    s.push_str("  }");
    s
}

pub fn upcloud_router_hcl(name: &str) -> String {
    format!(
        r#"resource "upcloud_router" "{name}_router" {{
  name = "{name}-router"
}}
"#,
        name = name,
    )
}

pub fn upcloud_object_storage_hcl(resource_name: &str, bucket_name: &str) -> String {
    format!(
        r#"resource "upcloud_managed_object_storage" "{name}" {{
  name              = "{name}-storage"
  region            = "__OBJSTORAGE_REGION__"
  configured_status = "started"
}}

resource "upcloud_managed_object_storage_bucket" "{name}_bucket" {{
  service_uuid = upcloud_managed_object_storage.{name}.id
  name         = "{bucket}"
}}
"#,
        name = resource_name,
        bucket = bucket_name,
    )
}

pub fn storage_devices_snippet(server_ref: &str, storage_ref: &str) -> String {
    format!(
        "# Add inside resource \"upcloud_server\" \"{server_ref}\" {{\n  storage_devices {{\n    storage = upcloud_storage.{storage_ref}.id\n    type    = \"disk\"\n  }}"
    )
}

pub fn upcloud_kubernetes_cluster_hcl(id: &str, name_hcl: &str, version_hcl: &str) -> String {
    format!(
        r#"resource "upcloud_kubernetes_cluster" "{id}" {{
  control_plane_ip_filter = ["0.0.0.0/0"]  # restrict to known CIDRs in production
  name                    = {name_hcl}
  network                 = "<TODO: upcloud_network reference>"
  zone                    = "__ZONE__"
  version                 = {version_hcl}
  # plan = "prod-md"  # use "dev" for non-production clusters (check `upctl kubernetes plans`)
  # private_node_groups = true  # uncomment if node-group subnets are private
}}
"#,
        id = id,
        name_hcl = name_hcl,
        version_hcl = version_hcl,
    )
}
