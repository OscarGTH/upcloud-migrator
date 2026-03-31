use super::super::shared;
use super::compute::aws_instance_type_to_upcloud_plan;
use crate::migration::types::{MigrationResult, MigrationStatus};
use crate::terraform::types::TerraformResource;

pub fn map_eks_cluster(res: &TerraformResource) -> MigrationResult {
    let k8s_version_raw = res
        .attributes
        .get("version")
        .map(|v| v.trim_matches('"').to_string())
        .unwrap_or_else(|| "1.31".to_string());
    let version_hcl = shared::hcl_value(&k8s_version_raw);

    // Use the `name` attribute from the source (may be var.cluster_name etc.),
    // falling back to the resource identifier if absent.
    let name_raw = res
        .attributes
        .get("name")
        .map(|v| v.trim_matches('"').to_string())
        .unwrap_or_else(|| res.name.clone());
    let name_hcl = shared::hcl_value(&name_raw);

    let hcl = shared::upcloud_kubernetes_cluster_hcl(&res.name, &name_hcl, &version_hcl);

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
            format!("EKS → UpCloud Managed Kubernetes (k8s {})", k8s_version_raw),
            "network is auto-resolved from the EKS cluster's VPC subnets when a matching upcloud_network exists.".into(),
            "The upcloud_network used here must have lifecycle { ignore_changes = [router] } — generated automatically.".into(),
            "IAM roles for service accounts → use UpCloud API credentials instead.".into(),
            "Uncomment private_node_groups = true if all node-group subnets are private (nodes have no public IPs, outbound via NAT gateway).".into(),
        ],
        source_hcl: None,
    }
}

pub fn map_eks_node_group(res: &TerraformResource) -> MigrationResult {
    let instance_types_raw = res
        .attributes
        .get("instance_types")
        .map(|t| {
            // hcl-rs formats list expressions with newlines (e.g. "[\n  var.x\n]").
            // Collapse to a single line so the value can be safely embedded in
            // HCL comment lines (which must not contain newlines).
            t.split_whitespace().collect::<Vec<_>>().join(" ")
        })
        .unwrap_or_else(|| "[\"t3.medium\"]".into());

    // Extract the first concrete instance type for plan mapping (skip variable refs)
    let first_type = instance_types_raw
        .trim_matches(|c: char| c == '[' || c == ']')
        .split(',')
        .next()
        .unwrap_or("")
        .trim()
        .trim_matches('"');

    let plan = aws_instance_type_to_upcloud_plan(first_type).unwrap_or("2xCPU-4GB");

    let desired = res
        .attributes
        .get("scaling_config.desired_size")
        .map(|s| s.trim_matches('"'))
        .unwrap_or("2");
    let min_size = res
        .attributes
        .get("scaling_config.min_size")
        .map(|s| s.trim_matches('"'))
        .unwrap_or("1");
    let max_size = res
        .attributes
        .get("scaling_config.max_size")
        .map(|s| s.trim_matches('"'))
        .unwrap_or("3");

    // Use the `node_group_name` attribute from the source (may be "${var.cluster_name}-node-group"
    // etc.), falling back to the resource identifier if absent.
    let ng_name_raw = res
        .attributes
        .get("node_group_name")
        .or_else(|| res.attributes.get("name"))
        .map(|v| v.trim_matches('"').to_string())
        .unwrap_or_else(|| res.name.clone());
    let ng_name_hcl = shared::hcl_value(&ng_name_raw);

    let hcl = format!(
        r#"resource "upcloud_kubernetes_node_group" "{id}" {{
  cluster    = upcloud_kubernetes_cluster.<TODO>.id
  name       = {ng_name_hcl}
  plan       = "{plan}"  # from instance_types: {instance_types}
  node_count = {desired}  # desired_size={desired} (min={min}, max={max}; UpCloud node groups don't auto-scale)
}}
"#,
        id = res.name,
        ng_name_hcl = ng_name_hcl,
        plan = plan,
        instance_types = instance_types_raw,
        desired = desired,
        min = min_size,
        max = max_size,
    );

    let mut notes = vec![
        format!(
            "Node group ({}) → UpCloud k8s Node Group (plan: {})",
            instance_types_raw, plan
        ),
        "UpCloud node groups don't auto-scale — set node_count explicitly.".into(),
        "cluster reference auto-resolved from the EKS cluster in the same config.".into(),
    ];
    if first_type.starts_with("var.") || first_type.starts_with("${") {
        notes.push(
            "instance_types is a variable reference — update plan manually after resolving the variable.".into(),
        );
    }

    MigrationResult {
        resource_type: res.resource_type.clone(),
        resource_name: res.name.clone(),
        source_file: res.source_file.display().to_string(),
        status: MigrationStatus::Compatible,
        upcloud_type: "upcloud_kubernetes_node_group".into(),
        upcloud_hcl: Some(hcl),
        snippet: None,
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

    // ── map_eks_cluster ───────────────────────────────────────────────────────

    #[test]
    fn eks_cluster_generates_kubernetes_cluster() {
        let res = make_res("aws_eks_cluster", "prod", &[("version", "1.29")]);
        let r = map_eks_cluster(&res);
        assert_eq!(r.upcloud_type, "upcloud_kubernetes_cluster");
        let hcl = r.upcloud_hcl.unwrap();
        assert!(
            hcl.contains("resource \"upcloud_kubernetes_cluster\" \"prod\""),
            "{hcl}"
        );
        assert!(hcl.contains("version                 = \"1.29\""), "{hcl}");
        assert!(hcl.contains("control_plane_ip_filter"), "{hcl}");
    }

    #[test]
    fn eks_cluster_variable_name_emitted_unquoted() {
        let res = make_res("aws_eks_cluster", "main", &[("name", "var.cluster_name")]);
        let hcl = map_eks_cluster(&res).upcloud_hcl.unwrap();
        assert!(
            hcl.contains("name                    = var.cluster_name"),
            "variable name attr must be unquoted\n{hcl}"
        );
        assert!(
            !hcl.contains("\"var.cluster_name\""),
            "variable name attr must not be double-quoted\n{hcl}"
        );
    }

    #[test]
    fn eks_cluster_variable_version_emitted_unquoted() {
        let res = make_res(
            "aws_eks_cluster",
            "c",
            &[("version", "var.kubernetes_version")],
        );
        let hcl = map_eks_cluster(&res).upcloud_hcl.unwrap();
        assert!(
            hcl.contains("version                 = var.kubernetes_version"),
            "variable ref must be unquoted\n{hcl}"
        );
        assert!(
            !hcl.contains("\"var.kubernetes_version\""),
            "variable ref must not be double-quoted\n{hcl}"
        );
    }

    #[test]
    fn eks_cluster_defaults_version_when_absent() {
        let res = make_res("aws_eks_cluster", "c", &[]);
        let hcl = map_eks_cluster(&res).upcloud_hcl.unwrap();
        assert!(hcl.contains("version                 = \""), "{hcl}");
    }

    #[test]
    fn eks_cluster_has_network_ref_todo() {
        let res = make_res("aws_eks_cluster", "c", &[]);
        let hcl = map_eks_cluster(&res).upcloud_hcl.unwrap();
        assert!(
            hcl.contains("<TODO: upcloud_network reference>"),
            "cluster must have network TODO before generator cross-resolve\n{hcl}"
        );
    }

    #[test]
    fn eks_cluster_has_control_plane_ip_filter() {
        let res = make_res("aws_eks_cluster", "c", &[]);
        let hcl = map_eks_cluster(&res).upcloud_hcl.unwrap();
        assert!(hcl.contains("control_plane_ip_filter"), "{hcl}");
    }

    #[test]
    fn eks_cluster_notes_lifecycle_ignore_changes() {
        let res = make_res("aws_eks_cluster", "c", &[]);
        let r = map_eks_cluster(&res);
        assert!(
            r.notes.iter().any(|n| n.contains("ignore_changes")),
            "should note the lifecycle block on the network\n{:?}",
            r.notes
        );
    }

    #[test]
    fn eks_cluster_has_commented_private_node_groups() {
        // private_node_groups cannot be auto-detected from EKS vpc_config fields;
        // it depends on whether _node-group_ subnets are private-only (NAT-only outbound).
        // The generated HCL always includes it as a comment for the user to opt in.
        let res = make_res("aws_eks_cluster", "c", &[]);
        let hcl = map_eks_cluster(&res).upcloud_hcl.unwrap();
        assert!(
            hcl.contains("# private_node_groups = true"),
            "HCL must include a commented-out private_node_groups line\n{hcl}"
        );
        assert!(
            !hcl.contains("\n  private_node_groups     = true"),
            "private_node_groups must not be uncommented automatically\n{hcl}"
        );
    }

    #[test]
    fn eks_cluster_private_node_groups_note_mentions_nat() {
        let res = make_res("aws_eks_cluster", "c", &[]);
        let r = map_eks_cluster(&res);
        assert!(
            r.notes.iter().any(|n| n.contains("private_node_groups")),
            "should include a note explaining when to enable private_node_groups\n{:?}",
            r.notes
        );
    }

    #[test]
    fn eks_cluster_has_commented_plan() {
        let res = make_res("aws_eks_cluster", "c", &[]);
        let hcl = map_eks_cluster(&res).upcloud_hcl.unwrap();
        assert!(
            hcl.contains("# plan ="),
            "HCL must include a commented-out plan line\n{hcl}"
        );
    }

    // ── map_eks_node_group ────────────────────────────────────────────────────

    #[test]
    fn eks_node_group_generates_kubernetes_node_group() {
        let res = make_res(
            "aws_eks_node_group",
            "workers",
            &[
                ("scaling_config.desired_size", "3"),
                ("scaling_config.min_size", "1"),
                ("scaling_config.max_size", "5"),
            ],
        );
        let r = map_eks_node_group(&res);
        assert_eq!(r.upcloud_type, "upcloud_kubernetes_node_group");
        let hcl = r.upcloud_hcl.unwrap();
        assert!(
            hcl.contains("resource \"upcloud_kubernetes_node_group\" \"workers\""),
            "{hcl}"
        );
        assert!(hcl.contains("node_count = 3"), "{hcl}");
    }

    #[test]
    fn eks_node_group_variable_name_emitted_unquoted() {
        let res = make_res(
            "aws_eks_node_group",
            "main",
            &[("node_group_name", "${var.cluster_name}-node-group")],
        );
        let hcl = map_eks_node_group(&res).upcloud_hcl.unwrap();
        assert!(
            hcl.contains("name       = \"${var.cluster_name}-node-group\""),
            "template string should be quoted\n{hcl}"
        );
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

    #[test]
    fn eks_node_group_maps_instance_type_to_plan() {
        let res = make_res(
            "aws_eks_node_group",
            "ng",
            &[("instance_types", "[\"t3.large\"]")],
        );
        let hcl = map_eks_node_group(&res).upcloud_hcl.unwrap();
        assert!(hcl.contains("2xCPU-8GB"), "{hcl}");
    }

    #[test]
    fn eks_node_group_unknown_type_falls_back_to_default_plan() {
        let res = make_res(
            "aws_eks_node_group",
            "ng",
            &[("instance_types", "[\"x9.unknown\"]")],
        );
        let hcl = map_eks_node_group(&res).upcloud_hcl.unwrap();
        assert!(hcl.contains("2xCPU-4GB"), "{hcl}");
    }

    /// hcl-rs formats list expressions with newlines, e.g. `[\n  var.eks_instance_type\n]`.
    /// Embedding that raw string into a HCL comment breaks the file (comments are single-line).
    /// The mapper must normalise the value to a single line before using it.
    #[test]
    fn eks_node_group_multiline_instance_types_produces_valid_hcl() {
        let res = make_res(
            "aws_eks_node_group",
            "ng",
            &[("instance_types", "[\n  var.eks_instance_type\n]")],
        );
        let hcl = map_eks_node_group(&res).upcloud_hcl.unwrap();

        for line in hcl.lines() {
            assert!(
                !line.trim().eq("]"),
                "stray `]` on its own line — comment was broken:\n{hcl}"
            );
        }

        let r = map_eks_node_group(&res);
        for note in &r.notes {
            assert!(
                !note.contains('\n'),
                "note contains newline — would break `# NOTE:` comment:\n{note:?}"
            );
        }
    }

    #[test]
    fn eks_node_group_variable_instance_type_adds_note() {
        let res = make_res(
            "aws_eks_node_group",
            "ng",
            &[("instance_types", "[var.instance_type]")],
        );
        let r = map_eks_node_group(&res);
        assert!(
            r.notes.iter().any(|n| n.contains("variable reference")),
            "{:?}",
            r.notes
        );
    }
}
