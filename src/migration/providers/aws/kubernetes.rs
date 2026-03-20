#[cfg(test)]
use crate::migration::types::{MigrationResult, MigrationStatus};
#[cfg(test)]
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

    // ── map_eks_cluster ───────────────────────────────────────────────────────

    #[test]
    fn eks_cluster_generates_kubernetes_cluster() {
        let res = make_res("aws_eks_cluster", "prod", &[("version", "1.29")]);
        let r = map_eks_cluster(&res);
        assert_eq!(r.upcloud_type, "upcloud_kubernetes_cluster");
        let hcl = r.upcloud_hcl.unwrap();
        assert!(hcl.contains("resource \"upcloud_kubernetes_cluster\" \"prod\""), "{hcl}");
        assert!(hcl.contains("version             = \"1.29\""), "{hcl}");
    }

    #[test]
    fn eks_cluster_defaults_version_when_absent() {
        let res = make_res("aws_eks_cluster", "c", &[]);
        let hcl = map_eks_cluster(&res).upcloud_hcl.unwrap();
        // defaults to some version string
        assert!(hcl.contains("version             = \""), "{hcl}");
    }

    #[test]
    fn eks_cluster_has_network_uuid_todo() {
        // The network UUID TODO is cross-resolved by the generator when a subnet exists
        let res = make_res("aws_eks_cluster", "c", &[]);
        let hcl = map_eks_cluster(&res).upcloud_hcl.unwrap();
        assert!(
            hcl.contains("<TODO: upcloud_network UUID>"),
            "cluster must have network TODO before generator cross-resolve\n{hcl}"
        );
    }

    #[test]
    fn eks_cluster_has_control_plane_ip_filter() {
        let res = make_res("aws_eks_cluster", "c", &[]);
        let hcl = map_eks_cluster(&res).upcloud_hcl.unwrap();
        assert!(hcl.contains("control_plane_ip_filter"), "{hcl}");
    }

    // ── map_eks_node_group ────────────────────────────────────────────────────

    #[test]
    fn eks_node_group_generates_kubernetes_node_group() {
        let res = make_res("aws_eks_node_group", "workers", &[
            ("scaling_config.desired_size", "3"),
            ("scaling_config.min_size", "1"),
            ("scaling_config.max_size", "5"),
        ]);
        let r = map_eks_node_group(&res);
        assert_eq!(r.upcloud_type, "upcloud_kubernetes_node_group");
        let hcl = r.upcloud_hcl.unwrap();
        assert!(hcl.contains("resource \"upcloud_kubernetes_node_group\" \"workers\""), "{hcl}");
        // desired_size drives the node_count
        assert!(hcl.contains("node_count = 3"), "{hcl}");
    }

    #[test]
    fn eks_node_group_has_cluster_cross_ref_todo() {
        let res = make_res("aws_eks_node_group", "ng", &[]);
        let hcl = map_eks_node_group(&res).upcloud_hcl.unwrap();
        assert!(
            hcl.contains("upcloud_kubernetes_cluster.<TODO>.id"),
            "node group must reference cluster via TODO before generator cross-resolve\n{hcl}"
        );
    }

    #[test]
    fn eks_node_group_defaults_desired_size_to_2() {
        let res = make_res("aws_eks_node_group", "ng", &[]);
        let hcl = map_eks_node_group(&res).upcloud_hcl.unwrap();
        assert!(hcl.contains("node_count = 2"), "{hcl}");
    }

    /// hcl-rs formats list expressions with newlines, e.g. `[\n  var.eks_instance_type\n]`.
    /// Embedding that raw string into a HCL comment breaks the file (comments are single-line).
    /// The mapper must normalise the value to a single line before using it.
    #[test]
    fn eks_node_group_multiline_instance_types_produces_valid_hcl() {
        // Simulate the value hcl-rs produces for `instance_types = [var.eks_instance_type]`
        let res = make_res("aws_eks_node_group", "ng", &[
            ("instance_types", "[\n  var.eks_instance_type\n]"),
        ]);
        let hcl = map_eks_node_group(&res).upcloud_hcl.unwrap();

        // No line may start with whitespace-only content that looks like a continuation
        // of a broken comment — the simplest check is that no line is purely `]`.
        for line in hcl.lines() {
            assert!(
                !line.trim() .eq("]"),
                "stray `]` on its own line — comment was broken:\n{hcl}"
            );
        }

        // The note must also be a single line (no embedded newlines).
        let r = map_eks_node_group(&res);
        for note in &r.notes {
            assert!(
                !note.contains('\n'),
                "note contains newline — would break `# NOTE:` comment:\n{note:?}"
            );
        }
    }
}

#[cfg(test)]
pub fn map_eks_cluster(res: &TerraformResource) -> MigrationResult {
    let k8s_version = res.attributes.get("version").map(|v| v.trim_matches('"')).unwrap_or("1.28");

    let hcl = format!(
        r#"resource "upcloud_kubernetes_cluster" "{name}" {{
  name                = "{name}"
  zone                = "__ZONE__"
  network             = "<TODO: upcloud_network UUID>"
  version             = "{k8s_version}"

  control_plane_ip_filter = ["0.0.0.0/0"]  # restrict to known CIDRs in production
}}
"#,
        name = res.name,
        k8s_version = k8s_version,
    );

    MigrationResult {
        resource_type: res.resource_type.clone(),
        resource_name: res.name.clone(),
        source_file: res.source_file.display().to_string(),
        status: MigrationStatus::Compatible,
        upcloud_type: "upcloud_kubernetes_cluster".into(),
        upcloud_hcl: Some(hcl),
        snippet: None,
        parent_resource: None,
        notes: vec![
            format!("EKS → UpCloud Managed Kubernetes (k8s {})", k8s_version),
            "Update kubeconfig after cluster creation".into(),
            "IAM roles for service accounts → UpCloud API credentials".into(),
        ],
            source_hcl: None,
    }
}

#[cfg(test)]
pub fn map_eks_node_group(res: &TerraformResource) -> MigrationResult {
    let instance_types = res.attributes.get("instance_types")
        .map(|t| {
            // hcl-rs formats list expressions with newlines (e.g. "[\n  var.x\n]").
            // Collapse to a single line so the value can be safely embedded in
            // HCL comment lines (which must not contain newlines).
            t.split_whitespace().collect::<Vec<_>>().join(" ")
        })
        .unwrap_or_else(|| "t3.medium".into());
    let min_size = res.attributes.get("scaling_config.min_size").map(|s| s.trim_matches('"')).unwrap_or("1");
    let desired = res.attributes.get("scaling_config.desired_size").map(|s| s.trim_matches('"')).unwrap_or("2");
    let max_size = res.attributes.get("scaling_config.max_size").map(|s| s.trim_matches('"')).unwrap_or("3");

    let hcl = format!(
        r#"resource "upcloud_kubernetes_node_group" "{name}" {{
  cluster    = upcloud_kubernetes_cluster.<TODO>.id
  name       = "{name}"
  plan       = "2xCPU-4GB"  # TODO: map from instance_types: {instance_types}
  node_count = {desired}    # desired_size={desired} (min={min}, max={max})
}}
"#,
        name = res.name,
        instance_types = instance_types,
        desired = desired,
        min = min_size,
        max = max_size,
    );

    MigrationResult {
        resource_type: res.resource_type.clone(),
        resource_name: res.name.clone(),
        source_file: res.source_file.display().to_string(),
        status: MigrationStatus::Compatible,
        upcloud_type: "upcloud_kubernetes_node_group".into(),
        upcloud_hcl: Some(hcl),
        snippet: None,
        parent_resource: None,
        notes: vec![
            format!("Node group ({}) → UpCloud k8s Node Group", instance_types),
            "UpCloud node groups don't auto-scale; set count explicitly".into(),
        ],
            source_hcl: None,
    }
}
