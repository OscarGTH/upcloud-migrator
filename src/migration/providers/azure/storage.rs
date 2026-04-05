use super::super::shared;
use crate::migration::types::{MigrationResult, MigrationStatus};
use crate::terraform::types::TerraformResource;

pub fn map_managed_disk(res: &TerraformResource) -> MigrationResult {
    let size = res
        .attributes
        .get("disk_size_gb")
        .and_then(|s| s.trim_matches('"').parse::<u32>().ok())
        .unwrap_or(20);
    let storage_type = res
        .attributes
        .get("storage_account_type")
        .map(|t| t.trim_matches('"'))
        .unwrap_or("Premium_LRS");
    let upcloud_tier = match storage_type {
        "Premium_LRS" | "Premium_ZRS" | "UltraSSD_LRS" => "maxiops",
        "StandardSSD_LRS" | "StandardSSD_ZRS" => "maxiops",
        "Standard_LRS" => "hdd",
        _ => "maxiops",
    };

    let hcl = shared::upcloud_storage_hcl(&res.name, &res.name, size, upcloud_tier, "");

    MigrationResult {
        resource_type: res.resource_type.clone(),
        resource_name: res.name.clone(),
        source_file: res.source_file.display().to_string(),
        status: MigrationStatus::Native,
        upcloud_type: "upcloud_storage".into(),
        upcloud_hcl: Some(hcl),
        snippet: None,
        parent_resource: None,
        notes: vec![format!(
            "Managed Disk ({}) → UpCloud tier '{}'",
            storage_type, upcloud_tier
        )],
        source_hcl: None,
    }
}

pub fn map_data_disk_attachment(res: &TerraformResource) -> MigrationResult {
    let disk_name = res.attributes.get("managed_disk_id").and_then(|v| {
        let v = v.trim_matches('"');
        let rest = v.strip_prefix("azurerm_managed_disk.")?;
        let base = rest.split(['.', '[']).next().unwrap_or(rest);
        if base.is_empty() {
            None
        } else {
            Some(base.to_string())
        }
    });
    let vm_name = res.attributes.get("virtual_machine_id").and_then(|v| {
        let v = v.trim_matches('"');
        for prefix in &[
            "azurerm_linux_virtual_machine.",
            "azurerm_windows_virtual_machine.",
            "azurerm_virtual_machine.",
        ] {
            if let Some(rest) = v.strip_prefix(prefix) {
                let base = rest.split(['.', '[']).next().unwrap_or(rest);
                if !base.is_empty() {
                    return Some(base.to_string());
                }
            }
        }
        None
    });

    let storage_ref = disk_name
        .as_deref()
        .unwrap_or("<TODO: storage>")
        .to_string();
    let server_ref = vm_name.as_deref().unwrap_or("<TODO: server>").to_string();

    let snippet = shared::storage_devices_snippet(&server_ref, &storage_ref);

    MigrationResult {
        resource_type: res.resource_type.clone(),
        resource_name: res.name.clone(),
        source_file: res.source_file.display().to_string(),
        status: MigrationStatus::Partial,
        upcloud_type: "storage_devices block in upcloud_server".into(),
        upcloud_hcl: None,
        snippet: Some(snippet),
        parent_resource: vm_name,
        notes: vec![
            "Disk attachment → add a storage_devices block inside the upcloud_server resource."
                .into(),
            format!(
                "Add the snippet to upcloud_server.{server_ref} to attach upcloud_storage.{storage_ref}."
            ),
        ],
        source_hcl: None,
    }
}

pub fn map_storage_account(res: &TerraformResource) -> MigrationResult {
    let account_name = res
        .attributes
        .get("name")
        .map(|b| b.trim_matches('"').to_string())
        .unwrap_or_else(|| res.name.replace('_', "-"));

    let hcl = shared::upcloud_object_storage_hcl(&res.name, &account_name);

    MigrationResult {
        resource_type: res.resource_type.clone(),
        resource_name: res.name.clone(),
        source_file: res.source_file.display().to_string(),
        status: MigrationStatus::Compatible,
        upcloud_type: "upcloud_managed_object_storage + _bucket".into(),
        upcloud_hcl: Some(hcl),
        snippet: None,
        parent_resource: None,
        notes: vec![
            "Azure Storage Account → UpCloud Managed Object Storage (S3-compatible).".into(),
            "Blob containers should be migrated to separate bucket resources.".into(),
        ],
        source_hcl: None,
    }
}

pub fn map_storage_container(res: &TerraformResource) -> MigrationResult {
    let container_name = res
        .attributes
        .get("name")
        .map(|b| b.trim_matches('"').to_string())
        .unwrap_or_else(|| res.name.replace('_', "-"));

    // The Azure provider used to expose `storage_account_name` (string) but switched to
    // `storage_account_id` (resource reference) in newer provider versions. Try both.
    let storage_account_name = res
        .attributes
        .get("storage_account_name")
        .or_else(|| res.attributes.get("storage_account_id"))
        .and_then(|v| {
            let v = v.trim_matches('"');
            if v.starts_with("azurerm_storage_account.") {
                v.split('.').nth(1).map(str::to_string)
            } else {
                None
            }
        });

    let service_ref = storage_account_name
        .as_deref()
        .map(|n| format!("upcloud_managed_object_storage.{}.id", n))
        .unwrap_or_else(|| "upcloud_managed_object_storage.<TODO>.id".into());

    let hcl = format!(
        r#"resource "upcloud_managed_object_storage_bucket" "{name}" {{
  service_uuid = {service_ref}
  name         = "{container}"
}}
"#,
        name = res.name,
        service_ref = service_ref,
        container = container_name,
    );

    MigrationResult {
        resource_type: res.resource_type.clone(),
        resource_name: res.name.clone(),
        source_file: res.source_file.display().to_string(),
        status: MigrationStatus::Compatible,
        upcloud_type: "upcloud_managed_object_storage_bucket".into(),
        upcloud_hcl: Some(hcl),
        snippet: None,
        parent_resource: storage_account_name,
        notes: vec!["Azure Blob Container → UpCloud Object Storage Bucket.".into()],
        source_hcl: None,
    }
}

pub fn map_storage_share(res: &TerraformResource) -> MigrationResult {
    let quota = res
        .attributes
        .get("quota")
        .and_then(|v| v.trim_matches('"').parse::<u32>().ok())
        .unwrap_or(250);
    let size = quota.max(250); // UpCloud minimum 250 GiB

    let hcl = format!(
        r#"resource "upcloud_file_storage" "{name}" {{
  name              = "{name}"
  size              = {size}
  zone              = "__ZONE__"
  configured_status = "started"
}}
"#,
        name = res.name,
        size = size,
    );

    MigrationResult {
        resource_type: res.resource_type.clone(),
        resource_name: res.name.clone(),
        source_file: res.source_file.display().to_string(),
        status: MigrationStatus::Compatible,
        upcloud_type: "upcloud_file_storage".into(),
        upcloud_hcl: Some(hcl),
        snippet: None,
        parent_resource: None,
        notes: vec![
            "Azure File Share → UpCloud File Storage (NFS-based). Manual mount config needed."
                .into(),
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

    // ── map_managed_disk ──────────────────────────────────────────────────────

    #[test]
    fn premium_lrs_maps_to_maxiops() {
        let res = make_res(
            "azurerm_managed_disk",
            "data",
            &[
                ("storage_account_type", "Premium_LRS"),
                ("disk_size_gb", "100"),
            ],
        );
        let r = map_managed_disk(&res);
        assert_eq!(r.upcloud_type, "upcloud_storage");
        let hcl = r.upcloud_hcl.unwrap();
        assert!(
            hcl.contains("resource \"upcloud_storage\" \"data\""),
            "{hcl}"
        );
        assert!(hcl.contains("tier  = \"maxiops\""), "{hcl}");
    }

    #[test]
    fn standard_lrs_maps_to_hdd() {
        let res = make_res(
            "azurerm_managed_disk",
            "archive",
            &[
                ("storage_account_type", "Standard_LRS"),
                ("disk_size_gb", "500"),
            ],
        );
        let hcl = map_managed_disk(&res).upcloud_hcl.unwrap();
        assert!(hcl.contains("tier  = \"hdd\""), "{hcl}");
    }

    #[test]
    fn premium_zrs_maps_to_maxiops() {
        let res = make_res(
            "azurerm_managed_disk",
            "v",
            &[
                ("storage_account_type", "Premium_ZRS"),
                ("disk_size_gb", "50"),
            ],
        );
        assert!(
            map_managed_disk(&res)
                .upcloud_hcl
                .unwrap()
                .contains("maxiops")
        );
    }

    #[test]
    fn managed_disk_size_propagated() {
        let res = make_res(
            "azurerm_managed_disk",
            "big",
            &[
                ("storage_account_type", "Premium_LRS"),
                ("disk_size_gb", "200"),
            ],
        );
        let hcl = map_managed_disk(&res).upcloud_hcl.unwrap();
        assert!(hcl.contains("size  = 200"), "{hcl}");
    }

    #[test]
    fn managed_disk_defaults_to_20gb_maxiops_when_no_attrs() {
        let res = make_res("azurerm_managed_disk", "v", &[]);
        let hcl = map_managed_disk(&res).upcloud_hcl.unwrap();
        assert!(hcl.contains("size  = 20"), "{hcl}");
        assert!(hcl.contains("maxiops"), "{hcl}");
    }

    // ── map_data_disk_attachment ──────────────────────────────────────────────

    #[test]
    fn disk_attachment_generates_storage_devices_snippet() {
        let res = make_res(
            "azurerm_virtual_machine_data_disk_attachment",
            "attach",
            &[
                ("managed_disk_id", "azurerm_managed_disk.data.id"),
                ("virtual_machine_id", "azurerm_linux_virtual_machine.web.id"),
            ],
        );
        let r = map_data_disk_attachment(&res);
        assert_eq!(r.status, crate::migration::types::MigrationStatus::Partial);
        let snippet = r.snippet.unwrap();
        assert!(snippet.contains("storage_devices {"), "{snippet}");
        assert!(snippet.contains("upcloud_storage.data.id"), "{snippet}");
    }

    #[test]
    fn disk_attachment_no_upcloud_hcl() {
        let res = make_res("azurerm_virtual_machine_data_disk_attachment", "a", &[]);
        assert!(map_data_disk_attachment(&res).upcloud_hcl.is_none());
    }

    // ── map_storage_account ───────────────────────────────────────────────────

    #[test]
    fn storage_account_generates_object_storage_and_bucket() {
        let res = make_res("azurerm_storage_account", "assets", &[("name", "myassets")]);
        let r = map_storage_account(&res);
        let hcl = r.upcloud_hcl.unwrap();
        assert!(hcl.contains("upcloud_managed_object_storage"), "{hcl}");
        assert!(
            hcl.contains("upcloud_managed_object_storage_bucket"),
            "{hcl}"
        );
    }

    // ── map_storage_container ─────────────────────────────────────────────────

    #[test]
    fn storage_container_generates_bucket_resource() {
        let res = make_res(
            "azurerm_storage_container",
            "blobs",
            &[
                ("name", "my-container"),
                (
                    "storage_account_name",
                    "azurerm_storage_account.assets.name",
                ),
            ],
        );
        let r = map_storage_container(&res);
        assert_eq!(r.upcloud_type, "upcloud_managed_object_storage_bucket");
        let hcl = r.upcloud_hcl.unwrap();
        assert!(
            hcl.contains("upcloud_managed_object_storage.assets.id"),
            "{hcl}"
        );
    }

    #[test]
    fn storage_container_without_account_has_todo_ref() {
        let res = make_res("azurerm_storage_container", "blobs", &[]);
        let hcl = map_storage_container(&res).upcloud_hcl.unwrap();
        assert!(
            hcl.contains("upcloud_managed_object_storage.<TODO>.id"),
            "{hcl}"
        );
    }

    // ── map_storage_share ─────────────────────────────────────────────────────

    #[test]
    fn storage_share_generates_file_storage() {
        let res = make_res("azurerm_storage_share", "share", &[("quota", "1000")]);
        let r = map_storage_share(&res);
        assert_eq!(r.upcloud_type, "upcloud_file_storage");
        let hcl = r.upcloud_hcl.unwrap();
        assert!(
            hcl.contains("resource \"upcloud_file_storage\" \"share\""),
            "{hcl}"
        );
        assert!(hcl.contains("size              = 1000"), "{hcl}");
    }

    #[test]
    fn storage_share_minimum_size_is_250() {
        let res = make_res("azurerm_storage_share", "small", &[("quota", "10")]);
        let hcl = map_storage_share(&res).upcloud_hcl.unwrap();
        assert!(hcl.contains("size              = 250"), "{hcl}");
    }
}
