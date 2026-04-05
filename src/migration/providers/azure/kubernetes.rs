use super::super::shared;
use super::compute::azure_vm_size_to_upcloud_plan;
use crate::migration::types::{MigrationResult, MigrationStatus};
use crate::terraform::types::TerraformResource;

pub fn map_kubernetes_cluster(res: &TerraformResource) -> MigrationResult {
    let k8s_version_raw = res
        .attributes
        .get("kubernetes_version")
        .map(|v| v.trim_matches('"').to_string())
        .unwrap_or_else(|| "1.31".to_string());
    let version_hcl = shared::hcl_value(&k8s_version_raw);

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
            format!("AKS → UpCloud Managed Kubernetes (k8s {})", k8s_version_raw),
            "network is auto-resolved from the AKS cluster's VNet subnets when a matching upcloud_network exists.".into(),
            "The upcloud_network used here must have lifecycle { ignore_changes = [router] } — generated automatically.".into(),
            "Azure RBAC and AAD integration → use UpCloud API credentials instead.".into(),
        ],
        source_hcl: None,
    }
}

pub fn map_kubernetes_cluster_node_pool(res: &TerraformResource) -> MigrationResult {
    let vm_size = res
        .attributes
        .get("vm_size")
        .map(|t| t.trim_matches('"').to_string())
        .unwrap_or_else(|| "Standard_D2s_v3".into());

    let plan = azure_vm_size_to_upcloud_plan(&vm_size).unwrap_or("2xCPU-4GB");

    let node_count = res
        .attributes
        .get("node_count")
        .map(|s| s.trim_matches('"'))
        .unwrap_or("2");
    let min_count = res
        .attributes
        .get("min_count")
        .map(|s| s.trim_matches('"'))
        .unwrap_or("1");
    let max_count = res
        .attributes
        .get("max_count")
        .map(|s| s.trim_matches('"'))
        .unwrap_or("3");

    let ng_name_raw = res
        .attributes
        .get("name")
        .map(|v| v.trim_matches('"').to_string())
        .unwrap_or_else(|| res.name.clone());
    let ng_name_hcl = shared::hcl_value(&ng_name_raw);

    // Try to resolve the parent cluster from kubernetes_cluster_id
    let cluster_ref = res
        .attributes
        .get("kubernetes_cluster_id")
        .and_then(|v| {
            let v = v.trim_matches('"');
            if v.starts_with("azurerm_kubernetes_cluster.") {
                v.split('.')
                    .nth(1)
                    .map(|n| format!("upcloud_kubernetes_cluster.{}.id", n))
            } else {
                None
            }
        })
        .unwrap_or_else(|| "upcloud_kubernetes_cluster.<TODO>.id".into());

    let hcl = format!(
        r#"resource "upcloud_kubernetes_node_group" "{id}" {{
  cluster    = {cluster_ref}
  name       = {ng_name_hcl}
  plan       = "{plan}"  # from vm_size: {vm_size}
  node_count = {node_count}  # node_count={node_count} (min={min}, max={max}; UpCloud node groups don't auto-scale)
}}
"#,
        id = res.name,
        cluster_ref = cluster_ref,
        ng_name_hcl = ng_name_hcl,
        plan = plan,
        vm_size = vm_size,
        node_count = node_count,
        min = min_count,
        max = max_count,
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
            format!(
                "AKS Node Pool (vm_size: {}) → UpCloud k8s Node Group (plan: {})",
                vm_size, plan
            ),
            "UpCloud node groups don't auto-scale — set node_count explicitly.".into(),
            "cluster reference auto-resolved from the AKS cluster.".into(),
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

    // ── map_kubernetes_cluster ────────────────────────────────────────────────

    #[test]
    fn aks_cluster_generates_upcloud_kubernetes_cluster() {
        let res = make_res(
            "azurerm_kubernetes_cluster",
            "prod",
            &[("kubernetes_version", "1.30")],
        );
        let r = map_kubernetes_cluster(&res);
        assert_eq!(r.upcloud_type, "upcloud_kubernetes_cluster");
        let hcl = r.upcloud_hcl.unwrap();
        assert!(
            hcl.contains("resource \"upcloud_kubernetes_cluster\" \"prod\""),
            "{hcl}"
        );
        assert!(hcl.contains("version                 = \"1.30\""), "{hcl}");
    }

    #[test]
    fn aks_cluster_has_network_todo() {
        let res = make_res("azurerm_kubernetes_cluster", "c", &[]);
        let hcl = map_kubernetes_cluster(&res).upcloud_hcl.unwrap();
        assert!(hcl.contains("<TODO: upcloud_network reference>"), "{hcl}");
    }

    #[test]
    fn aks_cluster_has_control_plane_ip_filter() {
        let res = make_res("azurerm_kubernetes_cluster", "c", &[]);
        let hcl = map_kubernetes_cluster(&res).upcloud_hcl.unwrap();
        assert!(hcl.contains("control_plane_ip_filter"), "{hcl}");
    }

    #[test]
    fn aks_cluster_defaults_version_when_absent() {
        let res = make_res("azurerm_kubernetes_cluster", "c", &[]);
        let hcl = map_kubernetes_cluster(&res).upcloud_hcl.unwrap();
        assert!(hcl.contains("version                 = \"1.31\""), "{hcl}");
    }

    #[test]
    fn aks_cluster_variable_version_emitted_unquoted() {
        let res = make_res(
            "azurerm_kubernetes_cluster",
            "c",
            &[("kubernetes_version", "var.k8s_version")],
        );
        let hcl = map_kubernetes_cluster(&res).upcloud_hcl.unwrap();
        assert!(
            hcl.contains("version                 = var.k8s_version"),
            "{hcl}"
        );
        assert!(!hcl.contains("\"var.k8s_version\""), "{hcl}");
    }

    #[test]
    fn aks_cluster_has_commented_private_node_groups() {
        let res = make_res("azurerm_kubernetes_cluster", "c", &[]);
        let hcl = map_kubernetes_cluster(&res).upcloud_hcl.unwrap();
        assert!(hcl.contains("# private_node_groups = true"), "{hcl}");
        assert!(!hcl.contains("\n  private_node_groups     = true"), "{hcl}");
    }

    // ── map_kubernetes_cluster_node_pool ──────────────────────────────────────

    #[test]
    fn node_pool_generates_upcloud_node_group() {
        let res = make_res(
            "azurerm_kubernetes_cluster_node_pool",
            "workers",
            &[("vm_size", "Standard_D4s_v3"), ("node_count", "3")],
        );
        let r = map_kubernetes_cluster_node_pool(&res);
        assert_eq!(r.upcloud_type, "upcloud_kubernetes_node_group");
        let hcl = r.upcloud_hcl.unwrap();
        assert!(
            hcl.contains("resource \"upcloud_kubernetes_node_group\" \"workers\""),
            "{hcl}"
        );
        assert!(hcl.contains("plan       = \"4xCPU-8GB\""), "{hcl}");
        assert!(hcl.contains("node_count = 3"), "{hcl}");
    }

    #[test]
    fn node_pool_with_cluster_id_ref_links_cluster() {
        let res = make_res(
            "azurerm_kubernetes_cluster_node_pool",
            "np",
            &[
                ("vm_size", "Standard_D2s_v3"),
                (
                    "kubernetes_cluster_id",
                    "azurerm_kubernetes_cluster.prod.id",
                ),
            ],
        );
        let hcl = map_kubernetes_cluster_node_pool(&res).upcloud_hcl.unwrap();
        assert!(hcl.contains("upcloud_kubernetes_cluster.prod.id"), "{hcl}");
    }

    #[test]
    fn node_pool_without_cluster_ref_has_todo() {
        let res = make_res("azurerm_kubernetes_cluster_node_pool", "np", &[]);
        let hcl = map_kubernetes_cluster_node_pool(&res).upcloud_hcl.unwrap();
        assert!(
            hcl.contains("upcloud_kubernetes_cluster.<TODO>.id"),
            "{hcl}"
        );
    }

    #[test]
    fn node_pool_unknown_vm_size_falls_back_to_default_plan() {
        let res = make_res(
            "azurerm_kubernetes_cluster_node_pool",
            "np",
            &[("vm_size", "Standard_UNKNOWN_X9")],
        );
        let hcl = map_kubernetes_cluster_node_pool(&res).upcloud_hcl.unwrap();
        assert!(hcl.contains("2xCPU-4GB"), "{hcl}");
    }
}
