use super::super::shared;
use super::generator_support;
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
    # __PROBE_HC__
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

    // Resolve LB and backend pool names directly from source HCL where possible.
    let lb_ref = generator_support::extract_lb_name_from_rule_hcl(&res.raw_hcl)
        .map(|n| format!("upcloud_loadbalancer.{}.id", n))
        .unwrap_or_else(|| "upcloud_loadbalancer.<TODO>.id".into());

    let backend_ref = generator_support::extract_backend_pool_from_lb_rule_hcl(&res.raw_hcl)
        .map(|n| format!("upcloud_loadbalancer_backend.{}.name", n))
        .unwrap_or_else(|| "upcloud_loadbalancer_backend.<TODO>.name".into());

    let hcl = format!(
        r#"resource "upcloud_loadbalancer_frontend" "{name}" {{
  loadbalancer         = {lb_ref}
  name                 = "{name}"
  mode                 = "{mode}"
  port                 = {port}
  default_backend_name = {backend_ref}

  networks {{
    name = "public"
  }}
}}
"#,
        name = res.name,
        lb_ref = lb_ref,
        mode = upcloud_mode,
        port = port,
        backend_ref = backend_ref,
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

    // Extract the NIC name to give users a clearer hint for IP resolution.
    let nic_name = generator_support::extract_backend_pool_server_from_association_hcl(&res.raw_hcl)
        .map(|(_, nic)| nic);
    let ip_ref = nic_name
        .as_deref()
        .map(|nic| format!("upcloud_server.<TODO: server using NIC {}>.network_interface[1].ip_address", nic))
        .unwrap_or_else(|| "<TODO: server IP>".into());

    let hcl = format!(
        r#"resource "upcloud_loadbalancer_static_backend_member" "{name}" {{
  backend      = {backend_ref}
  name         = "{name}"
  ip           = "{ip_ref}"
  port         = 80
  weight       = 100
  max_sessions = 1000
  enabled      = true
}}
"#,
        name = res.name,
        backend_ref = backend_ref,
        ip_ref = ip_ref,
    );

    let mut notes = vec![
        "Backend pool association → upcloud_loadbalancer_static_backend_member.".into(),
    ];
    if let Some(ref nic) = nic_name {
        notes.push(format!(
            "Set ip to the private IP of the server using azurerm_network_interface.{} (e.g. upcloud_server.NAME.network_interface[1].ip_address).",
            nic
        ));
    } else {
        notes.push("Set ip to the server's private IP address.".into());
    }

    MigrationResult {
        resource_type: res.resource_type.clone(),
        resource_name: res.name.clone(),
        source_file: res.source_file.display().to_string(),
        status: MigrationStatus::Compatible,
        upcloud_type: "upcloud_loadbalancer_static_backend_member".into(),
        upcloud_hcl: Some(hcl),
        snippet: None,
        parent_resource: pool_name,
        notes,
        source_hcl: None,
    }
}

pub fn map_application_gateway(res: &TerraformResource) -> MigrationResult {
    let network_val = generator_support::extract_appgw_subnet_ref(&res.raw_hcl)
        .unwrap_or_else(|| "\"<TODO: upcloud_network reference>\"".into());
    let networks_block = format!(
        r#"  networks {{
    name    = "private"
    type    = "private"
    family  = "IPv4"
    network = {network_val}
  }}

  networks {{
    name   = "public"
    type   = "public"
    family = "IPv4"
  }}"#
    );
    let main_hcl = shared::upcloud_loadbalancer_hcl(
        &res.name,
        "production-small",
        &networks_block,
        "  # WAF and path-based routing require manual configuration",
    );
    let sub_resources = generator_support::generate_appgw_sub_resources(&res.name, &res.raw_hcl);
    let hcl = if sub_resources.is_empty() {
        main_hcl
    } else {
        format!("{}\n{}", main_hcl, sub_resources)
    };

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
            "Azure Application Gateway → UpCloud Load Balancer with backends and frontends.".into(),
            "WAF and advanced routing features require manual migration.".into(),
            "Update TLS certificate references before applying.".into(),
        ],
        source_hcl: None,
    }
}

pub fn map_lb_probe(res: &TerraformResource) -> MigrationResult {
    let protocol = res
        .attributes
        .get("protocol")
        .map(|p| p.trim_matches('"'))
        .unwrap_or("Tcp");
    let hc_type = match protocol.to_lowercase().as_str() {
        "http" => "http",
        "https" => "https",
        _ => "tcp",
    };

    // Extract the health check properties so the generator can inject them
    // into the associated backend pool via probe_health_map cross-reference.
    let props = generator_support::extract_probe_health_check_props_hcl(&res.raw_hcl);

    let mut notes = vec![
        format!(
            "Azure LB Probe ({}) → health_check_* properties on upcloud_loadbalancer_backend.",
            protocol
        ),
        "Health check settings are automatically injected into the associated backend pool.".into(),
    ];
    if hc_type != "tcp" {
        if let Some(path) = res.attributes.get("request_path") {
            notes.push(format!("Health check path: {}", path.trim_matches('"')));
        }
    }

    MigrationResult {
        resource_type: res.resource_type.clone(),
        resource_name: res.name.clone(),
        source_file: res.source_file.display().to_string(),
        status: MigrationStatus::Native,
        upcloud_type: "properties block in upcloud_loadbalancer_backend".into(),
        upcloud_hcl: None,
        snippet: if props.is_empty() { None } else { Some(props) },
        parent_resource: None,
        notes,
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

    fn make_res_with_hcl(resource_type: &str, name: &str, attrs: &[(&str, &str)], raw_hcl: &str) -> TerraformResource {
        TerraformResource {
            resource_type: resource_type.to_string(),
            name: name.to_string(),
            attributes: attrs
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            source_file: PathBuf::from("test.tf"),
            raw_hcl: raw_hcl.to_string(),
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

    #[test]
    fn backend_pool_has_probe_marker_for_cross_ref_injection() {
        let res = make_res("azurerm_lb_backend_address_pool", "pool", &[]);
        let hcl = map_lb_backend_address_pool(&res).upcloud_hcl.unwrap();
        // The generator will replace # __PROBE_HC__ with actual health check properties.
        assert!(hcl.contains("# __PROBE_HC__"), "{hcl}");
    }

    // ── map_lb_probe ──────────────────────────────────────────────────────────

    #[test]
    fn lb_probe_http_is_native_and_has_snippet() {
        let res = make_res_with_hcl(
            "azurerm_lb_probe",
            "health",
            &[("protocol", "Http"), ("request_path", "/health")],
            r#"resource "azurerm_lb_probe" "health" {
  protocol            = "Http"
  port                = 80
  request_path        = "/health"
  interval_in_seconds = 15
  number_of_probes    = 2
}"#,
        );
        let r = map_lb_probe(&res);
        assert_eq!(r.status, crate::migration::types::MigrationStatus::Native);
        let snippet = r.snippet.expect("http probe should have a snippet");
        assert!(snippet.contains("health_check_type     = \"http\""), "{snippet}");
        assert!(snippet.contains("health_check_url      = \"/health\""), "{snippet}");
        assert!(snippet.contains("health_check_interval = 15"), "{snippet}");
        assert!(snippet.contains("health_check_rise     = 2"), "{snippet}");
        assert!(snippet.contains("health_check_fall     = 2"), "{snippet}");
    }

    #[test]
    fn lb_probe_tcp_has_tcp_type() {
        let res = make_res_with_hcl(
            "azurerm_lb_probe",
            "tcp_probe",
            &[("protocol", "Tcp")],
            r#"resource "azurerm_lb_probe" "tcp_probe" {
  protocol = "Tcp"
  port     = 443
}"#,
        );
        let r = map_lb_probe(&res);
        assert_eq!(r.status, crate::migration::types::MigrationStatus::Native);
        let snippet = r.snippet.expect("tcp probe should have a snippet");
        assert!(snippet.contains("health_check_type     = \"tcp\""), "{snippet}");
        // No URL for tcp
        assert!(!snippet.contains("health_check_url"), "{snippet}");
    }

    #[test]
    fn lb_probe_without_attributes_is_native_no_snippet() {
        let res = make_res("azurerm_lb_probe", "empty_probe", &[]);
        let r = map_lb_probe(&res);
        // Default protocol is "Tcp" with no extra attrs — still produces a snippet
        assert_eq!(r.status, crate::migration::types::MigrationStatus::Native);
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

    #[test]
    fn lb_rule_resolves_lb_and_backend_from_raw_hcl() {
        let res = make_res_with_hcl(
            "azurerm_lb_rule",
            "http",
            &[("protocol", "Tcp"), ("frontend_port", "80")],
            r#"resource "azurerm_lb_rule" "http" {
  loadbalancer_id          = azurerm_lb.main.id
  protocol                 = "Tcp"
  frontend_port            = 80
  backend_port             = 80
  backend_address_pool_ids = [azurerm_lb_backend_address_pool.web.id]
}"#,
        );
        let hcl = map_lb_rule(&res).upcloud_hcl.unwrap();
        assert!(hcl.contains("upcloud_loadbalancer.main.id"), "{hcl}");
        assert!(hcl.contains("upcloud_loadbalancer_backend.web.name"), "{hcl}");
    }

    #[test]
    fn lb_rule_without_source_hcl_has_todo_refs() {
        let res = make_res(
            "azurerm_lb_rule",
            "rule",
            &[("protocol", "Tcp"), ("frontend_port", "80")],
        );
        let hcl = map_lb_rule(&res).upcloud_hcl.unwrap();
        assert!(hcl.contains("upcloud_loadbalancer.<TODO>.id"), "{hcl}");
        assert!(hcl.contains("upcloud_loadbalancer_backend.<TODO>.name"), "{hcl}");
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

    #[test]
    fn pool_association_includes_nic_name_in_ip_hint() {
        let res = make_res_with_hcl(
            "azurerm_lb_backend_address_pool_association",
            "web_assoc",
            &[("backend_address_pool_id", "azurerm_lb_backend_address_pool.web.id")],
            r#"resource "azurerm_lb_backend_address_pool_association" "web_assoc" {
  network_interface_id    = azurerm_network_interface.web.id
  ip_configuration_name   = "internal"
  backend_address_pool_id = azurerm_lb_backend_address_pool.web.id
}"#,
        );
        let hcl = map_lb_backend_address_pool_association(&res).upcloud_hcl.unwrap();
        assert!(hcl.contains("web"), "NIC name 'web' should appear in IP hint\n{hcl}");
    }

    // ── map_application_gateway ───────────────────────────────────────────────

    #[test]
    fn application_gateway_is_partial_status() {
        let res = make_res("azurerm_application_gateway", "agw", &[]);
        let r = map_application_gateway(&res);
        assert_eq!(r.status, crate::migration::types::MigrationStatus::Partial);
        assert_eq!(r.upcloud_type, "upcloud_loadbalancer");
    }

    #[test]
    fn application_gateway_resolves_subnet_from_raw_hcl() {
        let raw = concat!(
            "resource \"azurerm_application_gateway\" \"main\" {\n",
            "  gateway_ip_configuration {\n",
            "    name      = \"gw-ip\"\n",
            "    subnet_id = azurerm_subnet.appgw.id\n",
            "  }\n",
            "}\n",
        );
        let res = make_res_with_hcl("azurerm_application_gateway", "main", &[], raw);
        let hcl = map_application_gateway(&res).upcloud_hcl.unwrap();
        assert!(hcl.contains("upcloud_network.appgw.id"), "subnet should be resolved\n{hcl}");
        assert!(!hcl.contains("<TODO: upcloud_network reference>"), "{hcl}");
    }

    #[test]
    fn application_gateway_generates_backends_and_frontends() {
        let raw = concat!(
            "resource \"azurerm_application_gateway\" \"main\" {\n",
            "  gateway_ip_configuration {\n",
            "    name      = \"gw-ip\"\n",
            "    subnet_id = azurerm_subnet.appgw.id\n",
            "  }\n",
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
            "}\n",
        );
        let res = make_res_with_hcl("azurerm_application_gateway", "main", &[], raw);
        let hcl = map_application_gateway(&res).upcloud_hcl.unwrap();
        assert!(hcl.contains("upcloud_loadbalancer_backend"), "{hcl}");
        assert!(hcl.contains("upcloud_loadbalancer_frontend"), "{hcl}");
        assert!(hcl.contains("health_check_type     = \"http\""), "{hcl}");
        assert!(hcl.contains("health_check_url      = \"/health\""), "{hcl}");
    }
}
