//! Provider-agnostic resource mapping framework.
//! Delegates to provider-specific [`ResourceMapper`] implementations.

use crate::migration::types::MigrationResult;
use crate::terraform::types::TerraformResource;

/// Per-provider resource mapping plug-in.
///
/// Implement this trait in each provider module (e.g. `providers::aws::mapper`)
/// to teach the framework how to map source cloud provider resources
/// to UpCloud equivalents.
pub trait ResourceMapper: Send + Sync {
    /// Map a single source resource to an UpCloud [`MigrationResult`].
    ///
    /// Return `Unsupported` or `Partial` status for resources with no direct equivalent.
    fn map(&self, res: &TerraformResource) -> MigrationResult;
}

/// Dispatch resource mapping via the appropriate provider.
///
/// # Arguments
/// - `provider_display_name`: Display name of the provider (e.g. "AWS")
/// - `mapper`: The provider's resource mapper
/// - `res`: The Terraform resource to map
pub fn map_resource_with(mapper: &dyn ResourceMapper, res: &TerraformResource) -> MigrationResult {
    let mut result = mapper.map(res);
    result.source_hcl = Some(res.raw_hcl.clone());
    result
}

/// Route a resource to the appropriate provider mapper.
///
/// Detects provider from resource type prefix (e.g. "aws_" for AWS),
/// uses the detected provider's mapper, and attaches source HCL.
pub fn map_resource(res: &TerraformResource) -> MigrationResult {
    use crate::migration::providers::detect_provider;
    use crate::migration::types::MigrationResult;
    use crate::migration::types::MigrationStatus;

    let rt = res.resource_type.as_str();

    // Known cloud provider prefixes that have (or will have) mapping support
    let is_cloud_provider =
        rt.starts_with("aws_") || rt.starts_with("azurerm_") || rt.starts_with("google_");

    // Try to detect provider from resource type prefix
    let mut result = if rt.starts_with("aws_") {
        let provider = detect_provider(&[]);
        map_resource_with(provider.mapper().as_ref(), res)
    } else if rt.starts_with("azurerm_") {
        let provider = detect_provider(&[crate::migration::types::MigrationResult {
            resource_type: rt.to_string(),
            resource_name: String::new(),
            source_file: String::new(),
            status: crate::migration::types::MigrationStatus::Unknown,
            upcloud_type: String::new(),
            upcloud_hcl: None,
            snippet: None,
            parent_resource: None,
            notes: vec![],
            source_hcl: None,
        }]);
        map_resource_with(provider.mapper().as_ref(), res)
    } else if is_cloud_provider {
        // Known cloud provider but not yet supported — mark as Unknown
        MigrationResult {
            resource_type: res.resource_type.clone(),
            resource_name: res.name.clone(),
            source_file: res.source_file.display().to_string(),
            status: MigrationStatus::Unknown,
            upcloud_type: "(unknown provider)".into(),
            upcloud_hcl: None,
            snippet: None,
            parent_resource: None,
            notes: vec![format!(
                "Provider not recognized for resource type '{}'",
                rt
            )],
            source_hcl: Some(res.raw_hcl.clone()),
        }
    } else {
        // Non-cloud-provider resource (e.g. kubernetes, helm, null, random) — keep as-is
        MigrationResult {
            resource_type: res.resource_type.clone(),
            resource_name: res.name.clone(),
            source_file: res.source_file.display().to_string(),
            status: MigrationStatus::Passthrough,
            upcloud_type: res.resource_type.clone(),
            upcloud_hcl: Some(res.raw_hcl.clone()),
            snippet: None,
            parent_resource: None,
            notes: vec!["Non-cloud-provider resource, kept as is.".to_string()],
            source_hcl: Some(res.raw_hcl.clone()),
        }
    };

    result.source_hcl = Some(res.raw_hcl.clone());
    result
}
