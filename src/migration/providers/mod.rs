pub mod aws;

use crate::migration::types::MigrationResult;

/// Roles that source resources play in cross-reference building.
/// The generator uses these to filter results by semantic role rather than
/// hardcoded provider-specific type strings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResourceRole {
    /// Compute instance (e.g. aws_instance, azurerm_virtual_machine)
    ComputeInstance,
    /// SSH key pair (e.g. aws_key_pair)
    KeyPair,
    /// Database/cache parameter group (e.g. aws_db_parameter_group)
    ParameterGroup,
    /// Database/cache subnet group (e.g. aws_db_subnet_group)
    SubnetGroup,
    /// Volume attachment (e.g. aws_volume_attachment)
    VolumeAttachment,
    /// Load balancer target group attachment
    LbTargetGroupAttachment,
    /// Load balancer listener
    LbListener,
    /// No special cross-reference role
    Other,
}

/// Abstraction over source cloud provider specifics.
///
/// The generator uses this trait to avoid hardcoding provider-specific type names,
/// HCL parsing logic, and reference patterns. Each supported source cloud
/// provider (AWS, Azure, GCP, …) implements this trait.
pub trait SourceProvider {
    /// Display name for comments and logs (e.g. "AWS", "Azure", "GCP")
    fn display_name(&self) -> &str;

    /// Resource type prefix (e.g. "aws_", "azurerm_", "google_")
    fn resource_type_prefix(&self) -> &str;

    /// Classify a source resource type by its cross-reference role.
    fn resource_role(&self, resource_type: &str) -> ResourceRole;

    /// The source resource type string for block-storage volumes.
    /// Used as key in the resolved HCL map when promoting storage count.
    fn volume_resource_type(&self) -> &str;

    // --- Source HCL extraction (for cross-reference table building) ---

    /// Extract security group / firewall references from a compute instance's source HCL.
    fn extract_security_refs_from_instance(&self, hcl: &str) -> Vec<String>;

    /// Extract the subnet/network resource name from a compute instance's source HCL.
    fn extract_subnet_from_instance(&self, hcl: &str) -> Option<String>;

    /// Extract parameter key-value pairs from a parameter group's source HCL.
    fn extract_parameter_blocks(&self, hcl: &str) -> Vec<(String, String)>;

    /// Whether a parameter name is valid for the target managed database properties.
    fn is_valid_db_property(&self, name: &str) -> bool;

    /// Extract subnet resource names from a subnet group's source HCL.
    fn extract_subnet_names_from_subnet_group(&self, hcl: &str) -> Vec<String>;

    /// Extract (target_group_name, server_name) from a LB target group attachment's source HCL.
    fn extract_tg_server_from_attachment(&self, hcl: &str) -> Option<(String, String)>;

    /// Extract target group name from a LB listener's source HCL.
    fn extract_tg_from_listener(&self, hcl: &str) -> Option<String>;

    /// Extract LB resource name from a listener's source HCL.
    /// Used as fallback when `parent_resource` is not set.
    fn extract_lb_name_from_listener(&self, hcl: &str) -> Option<String>;

    // --- Placeholder text builders ---

    /// Build the SSH key placeholder string for a given key pair name.
    fn ssh_key_placeholder(&self, key_name: &str) -> String;

    /// Build TODO text for an unresolved parameter group reference.
    fn parameter_group_todo_text(&self, group_name: &str) -> String;

    // --- Output processing ---

    /// Sanitize/remove leftover source provider references in generated HCL.
    fn sanitize_source_refs(&self, hcl: String) -> String;

    /// Rewrite source provider references in output/locals blocks to UpCloud equivalents.
    fn rewrite_output_refs(&self, hcl: &str) -> String;
}

/// Detect the source cloud provider from migration results.
/// Falls back to AWS if no provider-specific prefix is recognized.
pub fn detect_provider(results: &[MigrationResult]) -> Box<dyn SourceProvider> {
    let _is_aws = results.iter().any(|r| r.resource_type.starts_with("aws_"));
    // Future: add Azure/GCP detection here
    // let is_azure = results.iter().any(|r| r.resource_type.starts_with("azurerm_"));
    // let is_gcp = results.iter().any(|r| r.resource_type.starts_with("google_"));

    // Default to AWS (currently the only supported provider)
    Box::new(aws::AwsSourceProvider)
}
