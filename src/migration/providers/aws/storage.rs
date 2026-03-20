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

    // ── map_ebs_volume ────────────────────────────────────────────────────────

    #[test]
    fn ebs_gp2_maps_to_maxiops() {
        let res = make_res("aws_ebs_volume", "data", &[("type", "gp2"), ("size", "50")]);
        let r = map_ebs_volume(&res);
        assert_eq!(r.upcloud_type, "upcloud_storage");
        let hcl = r.upcloud_hcl.unwrap();
        assert!(hcl.contains("resource \"upcloud_storage\" \"data\""), "{hcl}");
        assert!(hcl.contains("tier  = \"maxiops\""), "{hcl}");
    }

    #[test]
    fn ebs_gp3_maps_to_maxiops() {
        let res = make_res("aws_ebs_volume", "v", &[("type", "gp3"), ("size", "100")]);
        assert!(map_ebs_volume(&res).upcloud_hcl.unwrap().contains("maxiops"));
    }

    #[test]
    fn ebs_io1_maps_to_maxiops() {
        let res = make_res("aws_ebs_volume", "v", &[("type", "io1"), ("size", "20")]);
        assert!(map_ebs_volume(&res).upcloud_hcl.unwrap().contains("maxiops"));
    }

    #[test]
    fn ebs_st1_maps_to_hdd() {
        let res = make_res("aws_ebs_volume", "cold", &[("type", "st1"), ("size", "500")]);
        let hcl = map_ebs_volume(&res).upcloud_hcl.unwrap();
        assert!(hcl.contains("tier  = \"hdd\""), "{hcl}");
    }

    #[test]
    fn ebs_sc1_maps_to_hdd() {
        let res = make_res("aws_ebs_volume", "archive", &[("type", "sc1"), ("size", "1000")]);
        assert!(map_ebs_volume(&res).upcloud_hcl.unwrap().contains("hdd"));
    }

    #[test]
    fn ebs_size_propagated() {
        let res = make_res("aws_ebs_volume", "big", &[("type", "gp2"), ("size", "200")]);
        let hcl = map_ebs_volume(&res).upcloud_hcl.unwrap();
        assert!(hcl.contains("size  = 200"), "{hcl}");
    }

    #[test]
    fn ebs_defaults_to_20gb_maxiops_when_no_attrs() {
        let res = make_res("aws_ebs_volume", "v", &[]);
        let hcl = map_ebs_volume(&res).upcloud_hcl.unwrap();
        assert!(hcl.contains("size  = 20"), "{hcl}");
        assert!(hcl.contains("maxiops"), "{hcl}");
    }

    #[test]
    fn ebs_with_count_propagates_count_and_indexed_title() {
        let res = make_res("aws_ebs_volume", "web_data", &[
            ("type", "gp3"),
            ("size", "50"),
            ("count", "var.web_server_count"),
        ]);
        let r = map_ebs_volume(&res);
        let hcl = r.upcloud_hcl.unwrap();
        assert!(hcl.contains("count = var.web_server_count"), "count should be propagated\n{hcl}");
        assert!(hcl.contains("web_data-${count.index + 1}"), "title should use count.index\n{hcl}");
        assert!(r.notes.iter().any(|n| n.contains("count")), "should note count propagation\n{:?}", r.notes);
    }

    // ── map_s3_bucket ─────────────────────────────────────────────────────────

    #[test]
    fn s3_bucket_generates_object_storage_and_bucket() {
        let res = make_res("aws_s3_bucket", "assets", &[("bucket", "my-assets-bucket")]);
        let r = map_s3_bucket(&res);
        assert!(r.upcloud_type.contains("upcloud_managed_object_storage"));
        let hcl = r.upcloud_hcl.unwrap();
        assert!(hcl.contains("resource \"upcloud_managed_object_storage\" \"assets\""), "{hcl}");
        assert!(hcl.contains("resource \"upcloud_managed_object_storage_bucket\" \"assets_bucket\""), "{hcl}");
        assert!(hcl.contains("name         = \"my-assets-bucket\""), "{hcl}");
    }

    #[test]
    fn s3_bucket_name_falls_back_to_resource_name() {
        let res = make_res("aws_s3_bucket", "my_bucket", &[]);
        let hcl = map_s3_bucket(&res).upcloud_hcl.unwrap();
        // When no bucket attribute, falls back to name with _ replaced by -
        assert!(hcl.contains("my-bucket"), "{hcl}");
    }

    #[test]
    fn s3_bucket_references_object_storage_id() {
        let res = make_res("aws_s3_bucket", "store", &[("bucket", "data")]);
        let hcl = map_s3_bucket(&res).upcloud_hcl.unwrap();
        assert!(
            hcl.contains("service_uuid = upcloud_managed_object_storage.store.id"),
            "{hcl}"
        );
    }

    // ── map_s3_bucket_policy ──────────────────────────────────────────────────

    #[test]
    fn s3_bucket_policy_is_partial_no_hcl() {
        let res = make_res("aws_s3_bucket_policy", "pol", &[]);
        let r = map_s3_bucket_policy(&res);
        assert_eq!(r.status, crate::migration::types::MigrationStatus::Partial);
        assert!(r.upcloud_hcl.is_none());
    }

    // ── map_volume_attachment ─────────────────────────────────────────────────

    #[test]
    fn volume_attachment_generates_storage_devices_snippet() {
        let res = make_res("aws_volume_attachment", "attach", &[
            ("volume_id", "aws_ebs_volume.data.id"),
            ("instance_id", "aws_instance.web.id"),
        ]);
        let r = map_volume_attachment(&res);
        assert_eq!(r.upcloud_type, "storage_devices block in upcloud_server");
        assert!(r.upcloud_hcl.is_none(), "no standalone HCL resource");
        let snippet = r.snippet.unwrap();
        assert!(snippet.contains("storage_devices"), "{snippet}");
        assert!(snippet.contains("upcloud_storage.data.id"), "{snippet}");
        assert!(snippet.contains("upcloud_server\" \"web\""), "{snippet}");
    }

    #[test]
    fn volume_attachment_without_refs_has_todo_snippet() {
        let res = make_res("aws_volume_attachment", "a", &[]);
        let snippet = map_volume_attachment(&res).snippet.unwrap();
        assert!(snippet.contains("storage_devices"), "{snippet}");
        assert!(snippet.contains("<TODO: storage>"), "{snippet}");
        assert!(snippet.contains("<TODO: server>"), "{snippet}");
    }

    // ── map_efs_file_system ───────────────────────────────────────────────────

    #[test]
    fn efs_generates_file_storage() {
        let res = make_res("aws_efs_file_system", "shared", &[]);
        let r = map_efs_file_system(&res);
        assert_eq!(r.upcloud_type, "upcloud_file_storage");
        let hcl = r.upcloud_hcl.unwrap();
        assert!(hcl.contains("resource \"upcloud_file_storage\" \"shared\""), "{hcl}");
        assert!(hcl.contains("zone              = \"__ZONE__\""), "{hcl}");
        assert!(hcl.contains("name              = \"shared\""), "{hcl}");
        assert!(hcl.contains("configured_status = \"started\""), "{hcl}");
    }
}

pub fn map_volume_attachment(res: &TerraformResource) -> MigrationResult {
    // aws_volume_attachment → storage_devices block inside upcloud_server.
    // There is no standalone UpCloud resource for volume attachment.
    // Strip any index expressions (e.g. `web_data[count.index]` → `web_data`) from references.
    let volume_name = res.attributes.get("volume_id").and_then(|v| {
        let v = v.trim_matches('"');
        if !v.starts_with("aws_ebs_volume.") { return None; }
        let rest = &v["aws_ebs_volume.".len()..];
        let base = rest.split(|c: char| c == '.' || c == '[').next().unwrap_or(rest);
        if base.is_empty() { None } else { Some(base.to_string()) }
    });
    let instance_name = res.attributes.get("instance_id").and_then(|v| {
        let v = v.trim_matches('"');
        if !v.starts_with("aws_instance.") { return None; }
        let rest = &v["aws_instance.".len()..];
        let base = rest.split(|c: char| c == '.' || c == '[').next().unwrap_or(rest);
        if base.is_empty() { None } else { Some(base.to_string()) }
    });

    let storage_ref = volume_name.as_deref().unwrap_or("<TODO: storage>").to_string();
    let server_ref  = instance_name.as_deref().unwrap_or("<TODO: server>").to_string();

    let snippet = format!(
        "# Add inside resource \"upcloud_server\" \"{server_ref}\" {{\n  storage_devices {{\n    storage = upcloud_storage.{storage_ref}.id\n    type    = \"disk\"\n  }}"
    );

    MigrationResult {
        resource_type: res.resource_type.clone(),
        resource_name: res.name.clone(),
        source_file: res.source_file.display().to_string(),
        status: MigrationStatus::Partial,
        upcloud_type: "storage_devices block in upcloud_server".into(),
        upcloud_hcl: None,
        snippet: Some(snippet),
        parent_resource: instance_name,
        notes: vec![
            "EBS volume attachments → add a storage_devices block inside the upcloud_server resource.".into(),
            format!("Add the snippet to upcloud_server.{server_ref} to attach upcloud_storage.{storage_ref}."),
        ],
        source_hcl: None,
    }
}

pub fn map_ebs_volume(res: &TerraformResource) -> MigrationResult {
    let size = res.attributes.get("size").and_then(|s| s.trim_matches('"').parse::<u32>().ok()).unwrap_or(20);
    let tier = res.attributes.get("type").map(|t| t.trim_matches('"')).unwrap_or("gp2");
    let upcloud_tier = match tier {
        "gp2" | "gp3" => "maxiops",
        "io1" | "io2" => "maxiops",
        "st1" | "sc1" => "hdd",
        _ => "maxiops",
    };

    // Propagate count if the source EBS volume used it (e.g. count = var.web_server_count)
    let count_attr = res.attributes.get("count").map(|v| v.trim_matches('"').to_string());
    let count_line = match &count_attr {
        Some(n) => format!("  count = {}\n", n),
        None    => String::new(),
    };
    // When count is set the title must be unique per instance
    let title_val = if count_attr.is_some() {
        format!("{}-${{count.index + 1}}", res.name)
    } else {
        res.name.to_string()
    };

    let hcl = format!(
        r#"resource "upcloud_storage" "{name}" {{
{count_line}  title = "{title}"
  size  = {size}
  tier  = "{tier}"
  zone  = "__ZONE__"
}}
"#,
        name = res.name,
        count_line = count_line,
        title = title_val,
        size = size,
        tier = upcloud_tier,
    );

    let mut notes = vec![format!("EBS type '{}' → UpCloud tier '{}'", tier, upcloud_tier)];
    if let Some(ref n) = count_attr {
        notes.push(format!("count = {} propagated.", n));
    }

    MigrationResult {
        resource_type: res.resource_type.clone(),
        resource_name: res.name.clone(),
        source_file: res.source_file.display().to_string(),
        status: MigrationStatus::Native,
        upcloud_type: "upcloud_storage".into(),
        upcloud_hcl: Some(hcl),
        snippet: None,
        parent_resource: None,
        notes,
        source_hcl: None,
    }
}

pub fn map_s3_bucket(res: &TerraformResource) -> MigrationResult {
    let bucket_name = res.attributes.get("bucket")
        .map(|b| b.trim_matches('"').to_string())
        .unwrap_or_else(|| res.name.replace('_', "-"));

    let hcl = format!(
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
        name = res.name,
        bucket = bucket_name,
    );

    MigrationResult {
        resource_type: res.resource_type.clone(),
        resource_name: res.name.clone(),
        source_file: res.source_file.display().to_string(),
        status: MigrationStatus::Native,
        upcloud_type: "upcloud_managed_object_storage + _bucket".into(),
        upcloud_hcl: Some(hcl),
        snippet: None,
        parent_resource: None,
        notes: vec!["S3 → UpCloud Managed Object Storage (S3-compatible)".into()],
            source_hcl: None,
    }
}

pub fn map_s3_bucket_policy(res: &TerraformResource) -> MigrationResult {
    MigrationResult {
        resource_type: res.resource_type.clone(),
        resource_name: res.name.clone(),
        source_file: res.source_file.display().to_string(),
        status: MigrationStatus::Partial,
        upcloud_type: "upcloud_managed_object_storage_user_access_key".into(),
        upcloud_hcl: None,
        snippet: None,
        parent_resource: None,
        notes: vec!["S3 bucket policies → UpCloud uses access keys and policies via the UI/API".into()],
            source_hcl: None,
    }
}

pub fn map_efs_file_system(res: &TerraformResource) -> MigrationResult {
    let hcl = format!(
        r#"resource "upcloud_file_storage" "{name}" {{
  name              = "{name}"
  size              = 250  # minimum 250 GiB; set appropriate size
  zone              = "__ZONE__"  # NOTE: File Storage is not available in all zones (e.g. fi-hel1); use fi-hel2 if your zone is unsupported
  configured_status = "started"
}}
"#,
        name = res.name,
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
        notes: vec!["EFS → UpCloud File Storage (NFS-based). Manual mount target config needed.".into()],
            source_hcl: None,
    }
}
