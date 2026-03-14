use crate::migration::types::{MigrationResult, MigrationStatus};
use crate::terraform::types::TerraformResource;

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

    // ── map_lb ────────────────────────────────────────────────────────────────

    #[test]
    fn public_lb_generates_public_networks_block() {
        let res = make_res("aws_lb", "main", &[("internal", "false")]);
        let r = map_lb(&res);
        assert_eq!(r.upcloud_type, "upcloud_loadbalancer");
        let hcl = r.upcloud_hcl.unwrap();
        assert!(hcl.contains("resource \"upcloud_loadbalancer\" \"main\""), "{hcl}");
        assert!(hcl.contains("type   = \"public\""), "{hcl}");
        // no deprecated `network = ...` attribute
        assert!(!hcl.contains("network = \"<TODO"), "must not use deprecated network attr\n{hcl}");
    }

    #[test]
    fn internal_lb_generates_private_networks_block_with_todo() {
        let res = make_res("aws_lb", "internal", &[("internal", "true")]);
        let hcl = map_lb(&res).upcloud_hcl.unwrap();
        assert!(hcl.contains("type    = \"private\""), "{hcl}");
        assert!(
            hcl.contains("<TODO: upcloud_network reference>"),
            "internal LB should have network TODO\n{hcl}"
        );
    }

    #[test]
    fn lb_without_internal_attr_defaults_to_public() {
        let res = make_res("aws_lb", "def", &[]);
        let hcl = map_lb(&res).upcloud_hcl.unwrap();
        assert!(hcl.contains("type   = \"public\""), "{hcl}");
    }

    #[test]
    fn lb_has_configured_status_started() {
        let res = make_res("aws_lb", "lb", &[]);
        let hcl = map_lb(&res).upcloud_hcl.unwrap();
        assert!(hcl.contains("configured_status = \"started\""), "{hcl}");
    }

    // ── map_lb_target_group ───────────────────────────────────────────────────

    #[test]
    fn lb_target_group_generates_static_backend_member() {
        let res = make_res("aws_lb_target_group", "web", &[
            ("protocol", "HTTP"),
            ("port", "80"),
        ]);
        let r = map_lb_target_group(&res);
        assert_eq!(r.upcloud_type, "upcloud_loadbalancer_backend");
        let hcl = r.upcloud_hcl.unwrap();
        // CRITICAL: must be static_backend_member, not backend_member
        assert!(
            hcl.contains("upcloud_loadbalancer_static_backend_member"),
            "must use static_backend_member resource type\n{hcl}"
        );
        assert!(
            !hcl.contains("\"upcloud_loadbalancer_backend_member\""),
            "must NOT use deprecated backend_member type\n{hcl}"
        );
    }

    #[test]
    fn lb_target_group_port_propagated() {
        let res = make_res("aws_lb_target_group", "api", &[("port", "8080")]);
        let hcl = map_lb_target_group(&res).upcloud_hcl.unwrap();
        assert!(hcl.contains("port         = 8080"), "{hcl}");
    }

    #[test]
    fn lb_target_group_has_max_sessions() {
        let res = make_res("aws_lb_target_group", "tg", &[]);
        let hcl = map_lb_target_group(&res).upcloud_hcl.unwrap();
        // max_sessions is Required in the provider schema
        assert!(hcl.contains("max_sessions = 1000"), "{hcl}");
    }

    #[test]
    fn lb_target_group_has_lb_cross_ref_todo() {
        let res = make_res("aws_lb_target_group", "tg", &[]);
        let hcl = map_lb_target_group(&res).upcloud_hcl.unwrap();
        assert!(hcl.contains("upcloud_loadbalancer.<TODO>.id"), "{hcl}");
    }

    #[test]
    fn lb_target_group_maps_health_check_properties() {
        let res = make_res("aws_lb_target_group", "web", &[
            ("protocol", "HTTP"),
            ("port", "80"),
            ("health_check.path", "\"/health\""),
            ("health_check.matcher", "\"200\""),
            ("health_check.healthy_threshold", "\"3\""),
            ("health_check.unhealthy_threshold", "\"2\""),
            ("health_check.interval", "\"30\""),
        ]);
        let hcl = map_lb_target_group(&res).upcloud_hcl.unwrap();
        assert!(hcl.contains("health_check_type   = \"http\""), "{hcl}");
        assert!(hcl.contains("health_check_url    = \"/health\""), "{hcl}");
        assert!(hcl.contains("health_check_expected_status = 200"), "{hcl}");
        assert!(hcl.contains("health_check_rise   = 3"), "{hcl}");
        assert!(hcl.contains("health_check_fall   = 2"), "{hcl}");
        assert!(hcl.contains("health_check_interval = 30"), "{hcl}");
    }

    #[test]
    fn lb_target_group_without_health_check_still_has_type() {
        // health_check_type defaults based on protocol even without explicit health_check block
        let res = make_res("aws_lb_target_group", "api", &[("protocol", "HTTP")]);
        let hcl = map_lb_target_group(&res).upcloud_hcl.unwrap();
        assert!(hcl.contains("health_check_type   = \"http\""), "{hcl}");
        assert!(!hcl.contains("health_check_url"), "no url when path not set\n{hcl}");
    }

    #[test]
    fn lb_target_group_tcp_protocol_uses_tcp_health_check() {
        let res = make_res("aws_lb_target_group", "nl", &[("protocol", "TCP")]);
        let hcl = map_lb_target_group(&res).upcloud_hcl.unwrap();
        assert!(hcl.contains("health_check_type   = \"tcp\""), "{hcl}");
    }

    // ── map_lb_listener ───────────────────────────────────────────────────────

    #[test]
    fn lb_listener_generates_frontend() {
        let res = make_res("aws_lb_listener", "http", &[
            ("protocol", "HTTP"),
            ("port", "80"),
        ]);
        let r = map_lb_listener(&res);
        assert_eq!(r.upcloud_type, "upcloud_loadbalancer_frontend");
        let hcl = r.upcloud_hcl.unwrap();
        assert!(hcl.contains("resource \"upcloud_loadbalancer_frontend\" \"http\""), "{hcl}");
        assert!(hcl.contains("port                 = 80"), "{hcl}");
    }

    #[test]
    fn lb_listener_has_lb_and_backend_cross_ref_todos() {
        let res = make_res("aws_lb_listener", "lst", &[]);
        let hcl = map_lb_listener(&res).upcloud_hcl.unwrap();
        assert!(hcl.contains("upcloud_loadbalancer.<TODO>.id"), "{hcl}");
        assert!(hcl.contains("upcloud_loadbalancer_backend.<TODO>.name"), "{hcl}");
    }

    // ── map_acm_certificate ───────────────────────────────────────────────────

    #[test]
    fn acm_cert_generates_manual_cert_bundle() {
        let res = make_res("aws_acm_certificate", "cert", &[]);
        let r = map_acm_certificate(&res);
        assert_eq!(r.upcloud_type, "upcloud_loadbalancer_manual_certificate_bundle");
        let hcl = r.upcloud_hcl.unwrap();
        assert!(hcl.contains("resource \"upcloud_loadbalancer_manual_certificate_bundle\" \"cert\""), "{hcl}");
    }

    #[test]
    fn acm_cert_has_certificate_and_private_key_todos() {
        let res = make_res("aws_acm_certificate", "cert", &[]);
        let hcl = map_acm_certificate(&res).upcloud_hcl.unwrap();
        assert!(hcl.contains("<TODO: base64 encoded certificate>"), "{hcl}");
        assert!(hcl.contains("<TODO: base64 encoded private key>"), "{hcl}");
    }

    #[test]
    fn acm_cert_is_partial_status() {
        let res = make_res("aws_acm_certificate", "cert", &[]);
        assert_eq!(
            map_acm_certificate(&res).status,
            crate::migration::types::MigrationStatus::Partial
        );
    }
}

pub fn map_lb(res: &TerraformResource) -> MigrationResult {
    let lb_type = res.attributes.get("load_balancer_type").map(|t| t.trim_matches('"')).unwrap_or("application");
    let is_internal = res.attributes.get("internal").map(|v| v.trim_matches('"') == "true").unwrap_or(false);

    // Public LBs use a public networks block (no network attribute needed).
    // Internal LBs use a private networks block with a network reference that gets cross-resolved.
    let networks_block = if is_internal {
        r#"  networks {
    name    = "private"
    type    = "private"
    family  = "IPv4"
    network = "<TODO: upcloud_network reference>"
  }"#
    } else {
        r#"  networks {
    name   = "public"
    type   = "public"
    family = "IPv4"
  }"#
    };

    let hcl = format!(
        r#"resource "upcloud_loadbalancer" "{name}" {{
  name              = "{name}"
  plan              = "production-small"
  zone              = "__ZONE__"
  configured_status = "started"

{networks}

  # Backends and frontends are separate resources
}}
"#,
        name = res.name,
        networks = networks_block,
    );

    let mut notes = vec![
        format!("ALB/NLB ({}) → UpCloud Load Balancer", lb_type),
        "Backends and frontends are defined as separate Terraform resources".into(),
    ];
    if is_internal {
        notes.push("Internal LB: private network reference will be auto-resolved if a subnet exists.".into());
    }

    MigrationResult {
        resource_type: res.resource_type.clone(),
        resource_name: res.name.clone(),
        source_file: res.source_file.display().to_string(),
        status: MigrationStatus::Compatible,
        score: 78,
        upcloud_type: "upcloud_loadbalancer".into(),
        upcloud_hcl: Some(hcl),
        snippet: None,
        parent_resource: None,
        notes,
        source_hcl: None,
    }
}

pub fn map_lb_target_group(res: &TerraformResource) -> MigrationResult {
    let protocol = res.attributes.get("protocol").map(|p| p.trim_matches('"')).unwrap_or("HTTP");
    let port = res.attributes.get("port").map(|p| p.trim_matches('"')).unwrap_or("80");

    // Map health_check block attributes to UpCloud backend properties.
    // hcl-rs stores nested block attrs with dot-prefix: "health_check.path", etc.
    let hc_type = match protocol.to_uppercase().as_str() {
        "HTTP" | "HTTPS" => "http",
        _ => "tcp",
    };
    let hc_url = res.attributes.get("health_check.path")
        .map(|v| v.trim_matches('"').to_string());
    let hc_status = res.attributes.get("health_check.matcher")
        .and_then(|v| v.trim_matches('"').parse::<u32>().ok());
    let hc_rise = res.attributes.get("health_check.healthy_threshold")
        .and_then(|v| v.trim_matches('"').parse::<u32>().ok());
    let hc_fall = res.attributes.get("health_check.unhealthy_threshold")
        .and_then(|v| v.trim_matches('"').parse::<u32>().ok());
    let hc_interval = res.attributes.get("health_check.interval")
        .and_then(|v| v.trim_matches('"').parse::<u32>().ok());

    let mut hc_lines = format!("    health_check_type   = \"{hc_type}\"\n");
    if let Some(url) = &hc_url {
        hc_lines.push_str(&format!("    health_check_url    = \"{url}\"\n"));
    }
    if let Some(status) = hc_status {
        hc_lines.push_str(&format!("    health_check_expected_status = {status}\n"));
    }
    if let Some(rise) = hc_rise {
        hc_lines.push_str(&format!("    health_check_rise   = {rise}\n"));
    }
    if let Some(fall) = hc_fall {
        hc_lines.push_str(&format!("    health_check_fall   = {fall}\n"));
    }
    if let Some(interval) = hc_interval {
        hc_lines.push_str(&format!("    health_check_interval = {interval}\n"));
    }

    let hcl = format!(
        r#"resource "upcloud_loadbalancer_backend" "{name}" {{
  loadbalancer = upcloud_loadbalancer.<TODO>.id
  name         = "{name}"

  properties {{
{hc_lines}  }}
}}

resource "upcloud_loadbalancer_static_backend_member" "{name}_member" {{
  backend      = upcloud_loadbalancer_backend.{name}.id
  name         = "{name}-member"
  weight       = 100
  max_sessions = 1000
  enabled      = true
  ip           = "<TODO: server IP>"
  port         = {port}
}}
"#,
        name = res.name,
        hc_lines = hc_lines,
        port = port,
    );

    let mut notes = vec![
        format!("Target group ({}/{}) → UpCloud LB Backend + static_backend_member", protocol, port),
    ];
    if hc_url.is_some() || hc_status.is_some() {
        notes.push("Health check settings mapped from aws_lb_target_group health_check block.".into());
    }

    MigrationResult {
        resource_type: res.resource_type.clone(),
        resource_name: res.name.clone(),
        source_file: res.source_file.display().to_string(),
        status: MigrationStatus::Compatible,
        score: 74,
        upcloud_type: "upcloud_loadbalancer_backend".into(),
        upcloud_hcl: Some(hcl),
        snippet: None,
        parent_resource: None,
        notes,
        source_hcl: None,
    }
}

pub fn map_lb_listener(res: &TerraformResource) -> MigrationResult {
    let protocol = res.attributes.get("protocol").map(|p| p.trim_matches('"')).unwrap_or("HTTP");
    let port = res.attributes.get("port").map(|p| p.trim_matches('"')).unwrap_or("80");
    let upcloud_mode = if protocol == "HTTPS" { "http" } else { "http" };

    let hcl = format!(
        r#"resource "upcloud_loadbalancer_frontend" "{name}" {{
  loadbalancer         = upcloud_loadbalancer.<TODO>.id
  name                 = "{name}"
  mode                 = "{mode}"
  port                 = {port}
  default_backend_name = upcloud_loadbalancer_backend.<TODO>.name
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
        score: 74,
        upcloud_type: "upcloud_loadbalancer_frontend".into(),
        upcloud_hcl: Some(hcl),
        snippet: None,
        parent_resource: None,
        notes: vec![
            format!("Listener ({}/{}) → UpCloud LB Frontend", protocol, port),
        ],
        source_hcl: None,
    }
}

pub fn map_acm_certificate(res: &TerraformResource) -> MigrationResult {
    let hcl = format!(
        r#"resource "upcloud_loadbalancer_manual_certificate_bundle" "{name}" {{
  name        = "{name}"
  certificate = "<TODO: base64 encoded certificate>"
  private_key = "<TODO: base64 encoded private key>"
}}
"#,
        name = res.name,
    );

    MigrationResult {
        resource_type: res.resource_type.clone(),
        resource_name: res.name.clone(),
        source_file: res.source_file.display().to_string(),
        status: MigrationStatus::Partial,
        score: 40,
        upcloud_type: "upcloud_loadbalancer_manual_certificate_bundle".into(),
        upcloud_hcl: Some(hcl),
        snippet: None,
        parent_resource: None,
        notes: vec![
            "ACM → UpCloud manual cert bundle. Export cert/key from ACM first.".into(),
            "No automatic certificate provisioning equivalent.".into(),
        ],
        source_hcl: None,
    }
}
