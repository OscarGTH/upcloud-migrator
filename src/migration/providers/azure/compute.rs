use super::super::shared;
use crate::migration::types::{MigrationResult, MigrationStatus};
use crate::terraform::types::TerraformResource;

/// Map Azure VM size (e.g. Standard_B2s) → UpCloud plan slug.
pub fn azure_vm_size_to_upcloud_plan(vm_size: &str) -> Option<&'static str> {
    match vm_size {
        // B-series (burstable)
        "Standard_B1s" | "Standard_B1ls" => Some("1xCPU-1GB"),
        "Standard_B1ms" => Some("1xCPU-2GB"),
        "Standard_B2s" | "Standard_B2ms" => Some("2xCPU-4GB"),
        "Standard_B2ts_v2" | "Standard_B2s_v2" => Some("2xCPU-4GB"),
        "Standard_B4ms" => Some("4xCPU-8GB"),

        // D-series (general purpose)
        "Standard_D2s_v3" | "Standard_D2s_v4" | "Standard_D2s_v5"
        | "Standard_D2as_v4" | "Standard_D2as_v5" => Some("2xCPU-4GB"),
        "Standard_D4s_v3" | "Standard_D4s_v4" | "Standard_D4s_v5"
        | "Standard_D4as_v4" | "Standard_D4as_v5" => Some("4xCPU-8GB"),
        "Standard_D8s_v3" | "Standard_D8s_v4" | "Standard_D8s_v5"
        | "Standard_D8as_v4" | "Standard_D8as_v5" => Some("8xCPU-32GB"),
        "Standard_D16s_v3" | "Standard_D16s_v4" | "Standard_D16s_v5" => Some("16xCPU-64GB"),

        // F-series (compute optimized)
        "Standard_F2s_v2" => Some("2xCPU-4GB"),
        "Standard_F4s_v2" => Some("4xCPU-8GB"),
        "Standard_F8s_v2" => Some("6xCPU-16GB"),

        // E-series (memory optimized)
        "Standard_E2s_v3" | "Standard_E2s_v4" | "Standard_E2s_v5" => Some("2xCPU-8GB"),
        "Standard_E4s_v3" | "Standard_E4s_v4" | "Standard_E4s_v5" => Some("4xCPU-16GB"),
        "Standard_E8s_v3" | "Standard_E8s_v4" | "Standard_E8s_v5" => Some("8xCPU-32GB"),

        // A-series (basic / previous gen)
        "Standard_A1_v2" => Some("1xCPU-2GB"),
        "Standard_A2_v2" => Some("2xCPU-4GB"),
        "Standard_A4_v2" => Some("4xCPU-8GB"),

        _ => None,
    }
}

fn map_vm_size(vm_size: &str) -> &'static str {
    azure_vm_size_to_upcloud_plan(vm_size).unwrap_or("2xCPU-4GB")
}

fn map_region(region: &str) -> &'static str {
    match region {
        "eastus" | "eastus2" | "centralus" | "northcentralus" | "southcentralus" => "us-nyc1",
        "westus" | "westus2" | "westus3" => "us-chi1",
        "canadacentral" | "canadaeast" => "us-nyc1",
        "northeurope" | "uksouth" | "ukwest" => "de-fra1",
        "westeurope" | "germanywestcentral" | "francecentral" | "francesouth" => "de-fra1",
        "swedencentral" | "norwayeast" => "fi-hel1",
        "southeastasia" | "eastasia" => "sg-sin1",
        "japaneast" | "japanwest" | "koreacentral" => "sg-sin1",
        "australiaeast" | "australiasoutheast" => "au-syd1",
        _ => "__ZONE__",
    }
}

pub fn map_linux_virtual_machine(res: &TerraformResource) -> MigrationResult {
    let vm_size = res
        .attributes
        .get("size")
        .map(|s| s.trim_matches('"'))
        .unwrap_or("Standard_B2s");
    let is_expr = shared::is_tf_expr(vm_size);
    let plan = if is_expr { "" } else { map_vm_size(vm_size) };
    let zone = res
        .attributes
        .get("location")
        .map(|l| map_region(l.trim_matches('"')))
        .unwrap_or("__ZONE__");
    let hostname = res.name.replace('_', "-");

    let admin_username = res
        .attributes
        .get("admin_username")
        .map(|v| v.trim_matches('"').to_string())
        .unwrap_or_else(|| "root".into());

    let login_block = format!(
        "\n  login {{\n    user = \"{}\"\n    keys = [\"<TODO: paste SSH public key>\"]\n  }}\n",
        admin_username
    );

    let plan_line = if is_expr {
        format!("  plan     = {}\n", vm_size)
    } else {
        format!("  plan     = \"{}\"\n", plan)
    };

    let os_disk_size = res
        .attributes
        .get("os_disk.disk_size_gb")
        .and_then(|v| v.trim_matches('"').parse::<u32>().ok())
        .unwrap_or(50);

    let storage_sentinel = format!("  # __STORAGE_END_{}__\n", res.name);

    let custom_data_line = res
        .attributes
        .get("custom_data")
        .map(|_| "\n  # custom_data was set — review and migrate to user_data (cloud-init)\n  metadata  = true")
        .unwrap_or("\n  metadata  = true");

    let hcl = format!(
        r#"resource "upcloud_server" "{name}" {{
  hostname = "{hostname}"
  zone     = "{zone}"
{plan_line}  firewall = true{custom_data}

  template {{
    storage = "Ubuntu Server 24.04 LTS (Noble Numbat)"
    size    = {os_disk_size}
  }}

  network_interface {{
    type = "public"
  }}

  network_interface {{
    type    = "private"
    network = "<TODO: upcloud_network reference>"
  }}{login}
{sentinel}}}
"#,
        name = res.name,
        hostname = hostname,
        zone = zone,
        plan_line = plan_line,
        os_disk_size = os_disk_size,
        custom_data = custom_data_line,
        login = login_block,
        sentinel = storage_sentinel,
    );

    let mut notes = vec![
        if is_expr {
            format!(
                "VM size '{}' is a variable — update its default in variables.tf to an UpCloud plan.",
                vm_size
            )
        } else {
            format!("VM size '{}' → plan '{}'", vm_size, plan)
        },
        "OS image → Ubuntu 24.04 LTS (update if needed: upctl storage list --public --template)".into(),
    ];
    if res.attributes.contains_key("custom_data") {
        notes.push("custom_data set — migrate to user_data for cloud-init.".into());
    }

    MigrationResult {
        resource_type: res.resource_type.clone(),
        resource_name: res.name.clone(),
        source_file: res.source_file.display().to_string(),
        status: MigrationStatus::Native,
        upcloud_type: "upcloud_server".into(),
        upcloud_hcl: Some(hcl),
        snippet: None,
        parent_resource: None,
        notes,
        source_hcl: None,
    }
}

pub fn map_windows_virtual_machine(res: &TerraformResource) -> MigrationResult {
    let vm_size = res
        .attributes
        .get("size")
        .map(|s| s.trim_matches('"'))
        .unwrap_or("Standard_B2s");
    let plan = map_vm_size(vm_size);
    let zone = res
        .attributes
        .get("location")
        .map(|l| map_region(l.trim_matches('"')))
        .unwrap_or("__ZONE__");
    let hostname = res.name.replace('_', "-");

    let os_disk_size = res
        .attributes
        .get("os_disk.disk_size_gb")
        .and_then(|v| v.trim_matches('"').parse::<u32>().ok())
        .unwrap_or(50);

    let hcl = format!(
        r#"resource "upcloud_server" "{name}" {{
  hostname = "{hostname}"
  zone     = "{zone}"
  plan     = "{plan}"
  firewall = true

  template {{
    storage = "Windows Server 2022 Standard"
    size    = {os_disk_size}
  }}

  network_interface {{
    type = "public"
  }}

  network_interface {{
    type    = "private"
    network = "<TODO: upcloud_network reference>"
  }}

  login {{
    user            = "Administrator"
    create_password = true
  }}
}}
"#,
        name = res.name,
        hostname = hostname,
        zone = zone,
        plan = plan,
        os_disk_size = os_disk_size,
    );

    MigrationResult {
        resource_type: res.resource_type.clone(),
        resource_name: res.name.clone(),
        source_file: res.source_file.display().to_string(),
        status: MigrationStatus::Compatible,
        upcloud_type: "upcloud_server".into(),
        upcloud_hcl: Some(hcl),
        snippet: None,
        parent_resource: None,
        notes: vec![
            format!("VM size '{}' → plan '{}'", vm_size, plan),
            "Windows VM → UpCloud Windows Server 2022 template.".into(),
            "RDP access requires firewall rule for port 3389.".into(),
        ],
        source_hcl: None,
    }
}

pub fn map_virtual_machine(res: &TerraformResource) -> MigrationResult {
    // Legacy azurerm_virtual_machine resource (deprecated in favor of linux/windows variants)
    let vm_size = res
        .attributes
        .get("vm_size")
        .map(|s| s.trim_matches('"'))
        .unwrap_or("Standard_B2s");
    let plan = map_vm_size(vm_size);
    let zone = res
        .attributes
        .get("location")
        .map(|l| map_region(l.trim_matches('"')))
        .unwrap_or("__ZONE__");
    let hostname = res.name.replace('_', "-");

    let hcl = format!(
        r#"resource "upcloud_server" "{name}" {{
  hostname = "{hostname}"
  zone     = "{zone}"
  plan     = "{plan}"
  firewall = true
  metadata = true

  template {{
    storage = "Ubuntu Server 24.04 LTS (Noble Numbat)"
    size    = 50
  }}

  network_interface {{
    type = "public"
  }}

  network_interface {{
    type    = "private"
    network = "<TODO: upcloud_network reference>"
  }}

  login {{
    user = "root"
    keys = ["<TODO: paste SSH public key>"]
  }}
}}
"#,
        name = res.name,
        hostname = hostname,
        zone = zone,
        plan = plan,
    );

    MigrationResult {
        resource_type: res.resource_type.clone(),
        resource_name: res.name.clone(),
        source_file: res.source_file.display().to_string(),
        status: MigrationStatus::Compatible,
        upcloud_type: "upcloud_server".into(),
        upcloud_hcl: Some(hcl),
        snippet: None,
        parent_resource: None,
        notes: vec![
            format!("VM size '{}' → plan '{}'", vm_size, plan),
            "Legacy azurerm_virtual_machine — consider using azurerm_linux_virtual_machine.".into(),
        ],
        source_hcl: None,
    }
}

pub fn map_ssh_public_key(res: &TerraformResource) -> MigrationResult {
    let public_key = res
        .attributes
        .get("public_key")
        .map(|v| {
            if v.starts_with('"') && v.ends_with('"') && v.len() >= 2 {
                v[1..v.len() - 1].to_string()
            } else {
                v.to_string()
            }
        });

    let key_value = public_key
        .as_deref()
        .unwrap_or("<TODO: paste SSH public key>");

    let snippet = format!(
        "login {{\n  user = \"root\"\n  keys = [\"{key}\"]  # was azurerm_ssh_public_key.{name}\n}}",
        key = key_value,
        name = res.name,
    );

    MigrationResult {
        resource_type: res.resource_type.clone(),
        resource_name: res.name.clone(),
        source_file: res.source_file.display().to_string(),
        status: MigrationStatus::Partial,
        upcloud_type: "login block (server resource)".into(),
        upcloud_hcl: None,
        snippet: Some(snippet),
        parent_resource: None,
        notes: vec![
            "Azure SSH key → UpCloud login block inside upcloud_server — not a standalone resource.".into(),
        ],
        source_hcl: None,
    }
}

pub fn map_availability_set(res: &TerraformResource) -> MigrationResult {
    MigrationResult {
        resource_type: res.resource_type.clone(),
        resource_name: res.name.clone(),
        source_file: res.source_file.display().to_string(),
        status: MigrationStatus::Unsupported,
        upcloud_type: "(no availability set equivalent)".into(),
        upcloud_hcl: None,
        snippet: None,
        parent_resource: None,
        notes: vec![
            "Azure Availability Sets have no direct UpCloud equivalent.".into(),
            "UpCloud provides high availability through zone placement.".into(),
        ],
        source_hcl: None,
    }
}

pub fn map_virtual_machine_scale_set(res: &TerraformResource) -> MigrationResult {
    let vm_size = res
        .attributes
        .get("sku")
        .map(|s| s.trim_matches('"'))
        .unwrap_or("Standard_B2s");
    let is_expr = shared::is_tf_expr(vm_size);
    let plan = if is_expr { "" } else { map_vm_size(vm_size) };
    let zone = res
        .attributes
        .get("location")
        .map(|l| map_region(l.trim_matches('"')))
        .unwrap_or("__ZONE__");
    let hostname_base = res.name.replace('_', "-");
    let admin_username = res
        .attributes
        .get("admin_username")
        .map(|v| v.trim_matches('"').to_string())
        .unwrap_or_else(|| "root".into());
    let os_disk_size = res
        .attributes
        .get("os_disk.disk_size_gb")
        .and_then(|v| v.trim_matches('"').parse::<u32>().ok())
        .unwrap_or(50);
    let instances = res
        .attributes
        .get("instances")
        .map(|v| v.trim_matches('"').to_string())
        .unwrap_or_else(|| "2".into());

    let plan_line = if is_expr {
        format!("  plan     = {}\n", vm_size)
    } else {
        format!("  plan     = \"{}\"\n", plan)
    };

    let storage_sentinel = format!("  # __STORAGE_END_{}__\n", res.name);

    let hcl = format!(
        r#"resource "upcloud_server" "{name}" {{
  count    = {instances}
  hostname = "{hostname_base}-${{count.index}}"
  zone     = "{zone}"
{plan_line}  firewall = true

  template {{
    storage = "Ubuntu Server 24.04 LTS (Noble Numbat)"
    size    = {os_disk_size}
  }}

  network_interface {{
    type = "public"
  }}

  network_interface {{
    type    = "private"
    network = "<TODO: upcloud_network reference>"
  }}

  login {{
    user = "{admin_username}"
    keys = ["<TODO: paste SSH public key>"]
  }}

{sentinel}}}
"#,
        name = res.name,
        instances = instances,
        hostname_base = hostname_base,
        zone = zone,
        plan_line = plan_line,
        os_disk_size = os_disk_size,
        admin_username = admin_username,
        sentinel = storage_sentinel,
    );

    let size_note = if is_expr {
        format!(
            "VM SKU '{}' is a variable — update its default in variables.tf to an UpCloud plan.",
            vm_size
        )
    } else {
        format!("VM SKU '{}' → plan '{}'", vm_size, plan)
    };

    MigrationResult {
        resource_type: res.resource_type.clone(),
        resource_name: res.name.clone(),
        source_file: res.source_file.display().to_string(),
        status: MigrationStatus::Native,
        upcloud_type: "upcloud_server".into(),
        upcloud_hcl: Some(hcl),
        snippet: None,
        parent_resource: None,
        notes: vec![
            size_note,
            "Azure VMSS → UpCloud servers with count. No auto-scaling equivalent — manage instance count manually.".into(),
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

    // ── azure_vm_size_to_upcloud_plan ─────────────────────────────────────────

    #[test]
    fn b1s_maps_to_1cpu_1gb() {
        assert_eq!(azure_vm_size_to_upcloud_plan("Standard_B1s"), Some("1xCPU-1GB"));
    }

    #[test]
    fn b2s_maps_to_2cpu_4gb() {
        assert_eq!(azure_vm_size_to_upcloud_plan("Standard_B2s"), Some("2xCPU-4GB"));
    }

    #[test]
    fn d4s_v3_maps_to_4cpu_8gb() {
        assert_eq!(azure_vm_size_to_upcloud_plan("Standard_D4s_v3"), Some("4xCPU-8GB"));
    }

    #[test]
    fn d8s_v5_maps_to_8cpu_32gb() {
        assert_eq!(azure_vm_size_to_upcloud_plan("Standard_D8s_v5"), Some("8xCPU-32GB"));
    }

    #[test]
    fn e4s_v3_maps_to_4cpu_16gb() {
        assert_eq!(azure_vm_size_to_upcloud_plan("Standard_E4s_v3"), Some("4xCPU-16GB"));
    }

    #[test]
    fn f8s_v2_maps_to_6cpu_16gb() {
        assert_eq!(azure_vm_size_to_upcloud_plan("Standard_F8s_v2"), Some("6xCPU-16GB"));
    }

    #[test]
    fn unknown_vm_size_returns_none() {
        assert_eq!(azure_vm_size_to_upcloud_plan("Standard_X99_MEGA"), None);
    }

    // ── map_linux_virtual_machine ─────────────────────────────────────────────

    #[test]
    fn linux_vm_generates_upcloud_server() {
        let res = make_res("azurerm_linux_virtual_machine", "web", &[("size", "Standard_B2s")]);
        let r = map_linux_virtual_machine(&res);
        assert_eq!(r.upcloud_type, "upcloud_server");
        assert_eq!(r.resource_name, "web");
        let hcl = r.upcloud_hcl.unwrap();
        assert!(hcl.contains("resource \"upcloud_server\" \"web\""), "{hcl}");
        assert!(hcl.contains("plan     = \"2xCPU-4GB\""), "{hcl}");
    }

    #[test]
    fn linux_vm_maps_eastus_location_to_zone() {
        let res = make_res(
            "azurerm_linux_virtual_machine",
            "app",
            &[("size", "Standard_B2s"), ("location", "eastus")],
        );
        let hcl = map_linux_virtual_machine(&res).upcloud_hcl.unwrap();
        assert!(hcl.contains("zone     = \"us-nyc1\""), "{hcl}");
    }

    #[test]
    fn linux_vm_maps_swedencentral_to_fi_hel1() {
        let res = make_res(
            "azurerm_linux_virtual_machine",
            "app",
            &[("size", "Standard_B2s"), ("location", "swedencentral")],
        );
        let hcl = map_linux_virtual_machine(&res).upcloud_hcl.unwrap();
        assert!(hcl.contains("zone     = \"fi-hel1\""), "{hcl}");
    }

    #[test]
    fn linux_vm_unknown_location_uses_placeholder() {
        let res = make_res(
            "azurerm_linux_virtual_machine",
            "app",
            &[("size", "Standard_B2s"), ("location", "unknownplace")],
        );
        let hcl = map_linux_virtual_machine(&res).upcloud_hcl.unwrap();
        assert!(hcl.contains("__ZONE__"), "{hcl}");
    }

    #[test]
    fn linux_vm_always_has_firewall_true() {
        let res = make_res("azurerm_linux_virtual_machine", "vm", &[]);
        let hcl = map_linux_virtual_machine(&res).upcloud_hcl.unwrap();
        assert!(hcl.contains("firewall = true"), "{hcl}");
    }

    #[test]
    fn linux_vm_with_variable_size_emits_unquoted() {
        let res = make_res(
            "azurerm_linux_virtual_machine",
            "vm",
            &[("size", "var.vm_size")],
        );
        let hcl = map_linux_virtual_machine(&res).upcloud_hcl.unwrap();
        assert!(hcl.contains("plan     = var.vm_size"), "{hcl}");
        assert!(!hcl.contains("\"var.vm_size\""), "{hcl}");
    }

    #[test]
    fn linux_vm_unknown_size_falls_back_to_default_plan() {
        let res = make_res(
            "azurerm_linux_virtual_machine",
            "vm",
            &[("size", "Standard_UNKNOWN_X9")],
        );
        let hcl = map_linux_virtual_machine(&res).upcloud_hcl.unwrap();
        assert!(hcl.contains("2xCPU-4GB"), "{hcl}");
    }

    #[test]
    fn linux_vm_has_login_block() {
        let res = make_res("azurerm_linux_virtual_machine", "vm", &[]);
        let hcl = map_linux_virtual_machine(&res).upcloud_hcl.unwrap();
        assert!(hcl.contains("login {"), "{hcl}");
        assert!(hcl.contains("keys = [\"<TODO: paste SSH public key>\"]"), "{hcl}");
    }

    // ── map_windows_virtual_machine ───────────────────────────────────────────

    #[test]
    fn windows_vm_generates_upcloud_server() {
        let res = make_res(
            "azurerm_windows_virtual_machine",
            "win",
            &[("size", "Standard_D4s_v3")],
        );
        let r = map_windows_virtual_machine(&res);
        assert_eq!(r.upcloud_type, "upcloud_server");
        let hcl = r.upcloud_hcl.unwrap();
        assert!(hcl.contains("resource \"upcloud_server\" \"win\""), "{hcl}");
        assert!(hcl.contains("4xCPU-8GB"), "{hcl}");
    }

    #[test]
    fn windows_vm_has_firewall_true() {
        let res = make_res("azurerm_windows_virtual_machine", "win", &[]);
        let hcl = map_windows_virtual_machine(&res).upcloud_hcl.unwrap();
        assert!(hcl.contains("firewall = true"), "{hcl}");
    }
}
