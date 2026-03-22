use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct TerraformResource {
    pub resource_type: String,
    pub name: String,
    pub attributes: HashMap<String, String>,
    pub source_file: PathBuf,
    /// Raw source text of the resource block, for diff display.
    pub raw_hcl: String,
}

/// Distinguishes the kind of passthrough block so the generator can decide
/// whether to rewrite embedded AWS resource references.
#[derive(Debug, Clone, PartialEq)]
pub enum PassthroughKind {
    Variable,
    Output,
    Locals,
    Provider,
    Data,
}

/// A non-resource block (`variable`, `output`, `locals`) that is passed through
/// to the generated output unchanged, with a comment added when the name looks
/// AWS-specific so the user knows to review it.
#[derive(Debug, Clone)]
pub struct PassthroughBlock {
    /// Block label (variable/output name), or `None` for unlabelled blocks like `locals`.
    pub name: Option<String>,
    /// Raw HCL text of the block exactly as it appeared in the source file.
    pub raw_hcl: String,
    pub source_file: PathBuf,
    pub kind: PassthroughKind,
}

#[derive(Debug, Clone)]
pub struct TerraformFile {
    pub _path: PathBuf,
    pub resources: Vec<TerraformResource>,
    pub passthroughs: Vec<PassthroughBlock>,
}
