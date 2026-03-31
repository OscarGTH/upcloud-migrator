use super::super::shared;
use crate::migration::types::{MigrationResult, MigrationStatus};
use crate::terraform::types::TerraformResource;

pub fn map_lb(res: &TerraformResource) -> MigrationResult {
    let sku = res
        .attributes
        .get("sku")
        .map(|s| s.trim_matches('"'))
        .unwrap_or("Standard");

    let networks_block = r#"  networks {
    name    = "private"
    type    = "private"
    family  = "IPv4"
    network = "<TODO: upcloud_network reference>"
  }

  networks {
    name   = "public"
    type   = "public"
    family = "IPv4"
  }"#;
    let hcl = shared::upcloud_loadbalancer_hcl(&res.name, "development", networks_block, "  # update to production-small or higher for production");

    MigrationResult {
        resource_type: res.resource_type.clone(),
        resource_name: res.name.clone(),
        source_file: res.source_file.display().to_string(),
        status: MigrationStatus::Compatible,
        upcloud_type: "upcloud_loadbalancer".into(),
        upcloud_hcl: Some(hcl),
        snippet: None,
        parent_resource: None,
        notes: vec![
            format!("Azure Load Balancer (SKU: {}) → UpCloud Load Balancer", sku),
            "Backends and frontends are defined as separate Terraform resources.".into(),
        ],
        source_hcl: None,
    }
}

pub fn map_lb_backend_address_pool(res: &TerraformResource) -> MigrationResult {
    let lb_name = res
        .attributes
        .get("loadbalancer_id")
        .and_then(|v| {
            let v = v.trim_matches('"');
            if v.starts_with("azurerm_lb.") {
                v.split('.').nth(1).map(str::to_string)
            } else {
                None
            }
        });

    let lb_ref = lb_name
        .as_deref()
        .map(|n| format!("upcloud_loadbalancer.{}.id", n))
        .unwrap_or_else(|| "upcloud_loadbalancer.<TODO>.id".into());

    let hcl = format!(
        r#"resource "upcloud_loadbalancer_backend" "{name}" {{
  loadbalancer = {lb_ref}
  name         = "{name}"

  properties {{
    health_check_type = "tcp"
  }}
}}
"#,
        name = res.name,
        lb_ref = lb_ref,
    );

    MigrationResult {
        resource_type: res.resource_type.clone(),
        resource_name: res.name.clone(),
        source_file: res.source_file.display().to_string(),
        status: MigrationStatus::Compatible,
        upcloud_type: "upcloud_loadbalancer_backend".into(),
        upcloud_hcl: Some(hcl),
        snippet: None,
        parent_resource: lb_name,
        notes: vec![
            "Azure Backend Address Pool → upcloud_loadbalancer_backend.".into(),
            "Add backend members as upcloud_loadbalancer_static_backend_member resources.".into(),
        ],
        source_hcl: None,
    }
}

pub fn map_lb_rule(res: &TerraformResource) -> MigrationResult {
    let protocol = res
        .attributes
        .get("protocol")
        .map(|p| p.trim_matches('"'))
        .unwrap_or("Tcp");
    let port = res
        .attributes
        .get("frontend_port")
        .map(|p| p.trim_matches('"'))
        .unwrap_or("80");

    let upcloud_mode = match protocol.to_lowercase().as_str() {
        "tcp" | "udp" => "tcp",
        _ => "http",
    };

    let hcl = format!(
        r#"resource "upcloud_loadbalancer_frontend" "{name}" {{
  loadbalancer         = upcloud_loadbalancer.<TODO>.id
  name                 = "{name}"
  mode                 = "{mode}"
  port                 = {port}
  default_backend_name = upcloud_loadbalancer_backend.<TODO>.name

  networks {{
    name = "public"
  }}
}}
"#,
        name = res.name,
        mode = upcloud_mode,
        port = port,
    );

    MigrationResult {
        resource_type: res.resource_type.clone(),
        resource_name: res.name.clone(),
        source_file: res.source_file.display().to_string(),
        status: MigrationStatus::Compatible,
        upcloud_type: "upcloud_loadbalancer_frontend".into(),
        upcloud_hcl: Some(hcl),
        snippet: None,
        parent_resource: None,
        notes: vec![
            format!("LB Rule ({}/{}) → UpCloud LB Frontend (mode={})", protocol, port, upcloud_mode),
        ],
        source_hcl: None,
    }
}

pub fn map_lb_backend_address_pool_association(res: &TerraformResource) -> MigrationResult {
    let pool_name = res
        .attributes
        .get("backend_address_pool_id")
        .and_then(|v| {
            let v = v.trim_matches('"');
            if v.starts_with("azurerm_lb_backend_address_pool.") {
                v.split('.').nth(1).map(str::to_string)
            } else {
                None
            }
        });

    let backend_ref = pool_name
        .as_deref()
        .map(|n| format!("upcloud_loadbalancer_backend.{}.id", n))
        .unwrap_or_else(|| "upcloud_loadbalancer_backend.<TODO>.id".into());

    let hcl = format!(
        r#"resource "upcloud_loadbalancer_static_backend_member" "{name}" {{
  backend      = {backend_ref}
  name         = "{name}"
  ip           = "<TODO: server IP>"
  port         = 80
  weight       = 100
  max_sessions = 1000
  enabled      = true
}}
"#,
        name = res.name,
        backend_ref = backend_ref,
    );

    MigrationResult {
        resource_type: res.resource_type.clone(),
        resource_name: res.name.clone(),
        source_file: res.source_file.display().to_string(),
        status: MigrationStatus::Compatible,
        upcloud_type: "upcloud_loadbalancer_static_backend_member".into(),
        upcloud_hcl: Some(hcl),
        snippet: None,
        parent_resource: pool_name,
        notes: vec![
            "Backend pool association → upcloud_loadbalancer_static_backend_member.".into(),
            "Set ip to the server's private IP address.".into(),
        ],
        source_hcl: None,
    }
}

pub fn map_application_gateway(res: &TerraformResource) -> MigrationResult {
    let networks_block = r#"  networks {
    name    = "private"
    type    = "private"
    family  = "IPv4"
    network = "<TODO: upcloud_network reference>"
  }

  networks {
    name   = "public"
    type   = "public"
    family = "IPv4"
  }"#;
    let hcl = shared::upcloud_loadbalancer_hcl(&res.name, "production-small", networks_block, "  # Application Gateway features (WAF, path-based routing, etc.) need manual migration");

    MigrationResult {
        resource_type: res.resource_type.clone(),
        resource_name: res.name.clone(),
        source_file: res.source_file.display().to_string(),
        status: MigrationStatus::Partial,
        upcloud_type: "upcloud_loadbalancer".into(),
        upcloud_hcl: Some(hcl),
        snippet: None,
        parent_resource: None,
        notes: vec![
            "Azure Application Gateway → UpCloud Load Balancer (L7 features need manual migration).".into(),
            "WAF, URL-based routing, and SSL termination require separate configuration.".into(),
        ],
        source_hcl: None,
    }
}

pub fn map_lb_probe(res: &TerraformResource) -> MigrationResult {
    MigrationResult {
        resource_type: res.resource_type.clone(),
        resource_name: res.name.clone(),
        source_file: res.source_file.display().to_string(),
        status: MigrationStatus::Partial,
        upcloud_type: "properties block in upcloud_loadbalancer_backend".into(),
        upcloud_hcl: None,
        snippet: None,
        parent_resource: None,
        notes: vec![
            "Azure LB Probe → configure health_check_* properties on upcloud_loadbalancer_backend.".into(),
        ],
        source_hcl: None,
    }
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

    // ── map_lb ────────────────────────────────────────────────────────────────

    #[test]
    fn lb_generates_upcloud_loadbalancer() {
        let res = make_res("azurerm_lb", "main", &[("sku", "Standard")]);
        let r = map_lb(&res);
        assert_eq!(r.upcloud_type, "upcloud_loadbalancer");
        let hcl = r.upcloud_hcl.unwrap();
        assert!(hcl.contains("resource \"upcloud_loadbalancer\" \"main\""), "{hcl}");
        assert!(hcl.contains("configured_status = \"started\""), "{hcl}");
    }

    #[test]
    fn lb_has_public_and_private_networks_blocks() {
        let res = make_res("azurerm_lb", "lb", &[]);
        let hcl = map_lb(&res).upcloud_hcl.unwrap();
        assert!(hcl.contains("type    = \"private\""), "{hcl}");
        assert!(hcl.contains("type   = \"public\""), "{hcl}");
    }

    #[test]
    fn lb_has_network_todo() {
        let res = make_res("azurerm_lb", "lb", &[]);
        let hcl = map_lb(&res).upcloud_hcl.unwrap();
        assert!(hcl.contains("<TODO: upcloud_network reference>"), "{hcl}");
    }

    // ── map_lb_backend_address_pool ───────────────────────────────────────────

    #[test]
    fn backend_pool_generates_loadbalancer_backend() {
        let res = make_res(
            "azurerm_lb_backend_address_pool",
            "pool",
            &[("loadbalancer_id", "azurerm_lb.main.id")],
        );
        let r = map_lb_backend_address_pool(&res);
        assert_eq!(r.upcloud_type, "upcloud_loadbalancer_backend");
        let hcl = r.upcloud_hcl.unwrap();
        assert!(hcl.contains("resource \"upcloud_loadbalancer_backend\" \"pool\""), "{hcl}");
        assert!(hcl.contains("upcloud_loadbalancer.main.id"), "{hcl}");
    }

    #[test]
    fn backend_pool_without_lb_ref_has_todo() {
        let res = make_res("azurerm_lb_backend_address_pool", "pool", &[]);
        let hcl = map_lb_backend_address_pool(&res).upcloud_hcl.unwrap();
        assert!(hcl.contains("upcloud_loadbalancer.<TODO>.id"), "{hcl}");
    }

    // ── map_lb_rule ───────────────────────────────────────────────────────────

    #[test]
    fn lb_rule_tcp_protocol_maps_to_tcp_mode() {
        let res = make_res(
            "azurerm_lb_rule",
            "http_rule",
            &[("protocol", "Tcp"), ("frontend_port", "80")],
        );
        let r = map_lb_rule(&res);
        assert_eq!(r.upcloud_type, "upcloud_loadbalancer_frontend");
        let hcl = r.upcloud_hcl.unwrap();
        assert!(hcl.contains("mode                 = \"tcp\""), "{hcl}");
        assert!(hcl.contains("port                 = 80"), "{hcl}");
    }

    #[test]
    fn lb_rule_http_protocol_maps_to_http_mode() {
        let res = make_res(
            "azurerm_lb_rule",
            "rule",
            &[("protocol", "Http"), ("frontend_port", "443")],
        );
        let hcl = map_lb_rule(&res).upcloud_hcl.unwrap();
        assert!(hcl.contains("mode                 = \"http\""), "{hcl}");
    }

    // ── map_lb_backend_address_pool_association ────────────────────────────────

    #[test]
    fn pool_association_generates_backend_member() {
        let res = make_res(
            "azurerm_lb_backend_address_pool_association",
            "assoc",
            &[("backend_address_pool_id", "azurerm_lb_backend_address_pool.pool.id")],
        );
        let r = map_lb_backend_address_pool_association(&res);
        assert_eq!(r.upcloud_type, "upcloud_loadbalancer_static_backend_member");
        let hcl = r.upcloud_hcl.unwrap();
        assert!(hcl.contains("upcloud_loadbalancer_backend.pool.id"), "{hcl}");
    }

    #[test]
    fn pool_association_no_ref_has_todo() {
        let res = make_res("azurerm_lb_backend_address_pool_association", "a", &[]);
        let hcl = map_lb_backend_address_pool_association(&res).upcloud_hcl.unwrap();
        assert!(hcl.contains("upcloud_loadbalancer_backend.<TODO>.id"), "{hcl}");
    }

    // ── map_application_gateway ───────────────────────────────────────────────

    #[test]
    fn application_gateway_is_partial_status() {
        let res = make_res("azurerm_application_gateway", "agw", &[]);
        let r = map_application_gateway(&res);
        assert_eq!(r.status, crate::migration::types::MigrationStatus::Partial);
        assert_eq!(r.upcloud_type, "upcloud_loadbalancer");
    }
}
