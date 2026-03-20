#[derive(Debug, Clone, PartialEq)]
pub enum MigrationStatus {
    Native,      // direct 1:1 mapping, full auto-convert
    Compatible,  // mapping exists, minor manual tweaks
    Partial,     // partial mapping, significant manual work
    Unsupported, // no equivalent, full manual migration
    Unknown,     // not recognized (other provider or custom)
}

impl MigrationStatus {
    pub fn label(&self) -> &'static str {
        match self {
            MigrationStatus::Native => "NATIVE",
            MigrationStatus::Compatible => "COMPATIBLE",
            MigrationStatus::Partial => "PARTIAL",
            MigrationStatus::Unsupported => "UNSUPPORTED",
            MigrationStatus::Unknown => "UNKNOWN",
        }
    }

}

#[derive(Debug, Clone)]
pub struct MigrationResult {
    pub resource_type: String,
    pub resource_name: String,
    pub source_file: String,
    pub status: MigrationStatus,
    pub upcloud_type: String,
    /// Fully renderable HCL written to the output .tf file.
    pub upcloud_hcl: Option<String>,
    /// HCL snippet that must be manually merged into another resource
    /// (e.g. ip_network block for a subnet). Written to MIGRATION_NOTES.md.
    pub snippet: Option<String>,
    /// Name of the parent resource this should be merged into (e.g. VPC name for a subnet).
    #[allow(dead_code)]
    pub parent_resource: Option<String>,
    pub notes: Vec<String>,
    /// Raw source HCL of the original resource block, for diff display.
    pub source_hcl: Option<String>,
}
