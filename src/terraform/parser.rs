use anyhow::Result;
use std::collections::HashMap;
use std::path::PathBuf;

use super::types::{PassthroughBlock, PassthroughKind, TerraformFile, TerraformResource};

pub fn parse_tf_file(path: &PathBuf) -> Result<TerraformFile> {
    let content = std::fs::read_to_string(path)?;
    let body: hcl::Body = hcl::from_str(&content).map_err(|e| anyhow::anyhow!("{}", e))?;

    let mut resources = Vec::new();
    let mut passthroughs = Vec::new();

    for block in body.blocks() {
        match block.identifier() {
            "resource" => {
                let labels: Vec<&str> = block.labels().iter().map(|l| l.as_str()).collect();
                if labels.len() < 2 {
                    continue;
                }
                let resource_type = labels[0].to_string();
                let resource_name = labels[1].to_string();

                let mut attributes: HashMap<String, String> = HashMap::new();
                extract_attributes(block.body(), &mut attributes, "");

                let raw_hcl = extract_raw_block(&content, &resource_type, &resource_name)
                    .unwrap_or_else(|| {
                        format!(
                            "resource \"{}\" \"{}\" {{ ... }}",
                            resource_type, resource_name
                        )
                    });

                resources.push(TerraformResource {
                    resource_type,
                    name: resource_name,
                    attributes,
                    source_file: path.clone(),
                    raw_hcl,
                });
            }
            kind @ ("variable" | "output" | "locals") => {
                let name = block.labels().first().map(|l| l.as_str().to_string());
                let raw_hcl =
                    extract_passthrough_block(&content, kind, name.as_deref()).unwrap_or_default();
                if !raw_hcl.is_empty() {
                    let pt_kind = match kind {
                        "output" => PassthroughKind::Output,
                        "locals" => PassthroughKind::Locals,
                        _ => PassthroughKind::Variable,
                    };
                    passthroughs.push(PassthroughBlock {
                        name,
                        raw_hcl,
                        source_file: path.clone(),
                        kind: pt_kind,
                    });
                }
            }
            _ => {} // terraform, provider, data, module — ignored for now
        }
    }

    Ok(TerraformFile {
        _path: path.clone(),
        resources,
        passthroughs,
    })
}

/// Extract the raw text of a `variable "name" { ... }`, `output "name" { ... }`,
/// or `locals { ... }` block from the source.  Works for any block kind.
fn extract_passthrough_block(content: &str, kind: &str, name: Option<&str>) -> Option<String> {
    let header = match name {
        Some(n) => format!("{} \"{}\"", kind, n),
        None => kind.to_string(),
    };
    extract_block_by_header(content, &header)
}

/// Extract the raw text of a `resource "type" "name" { ... }` block from source.
fn extract_raw_block(content: &str, resource_type: &str, name: &str) -> Option<String> {
    let header = format!("resource \"{}\" \"{}\"", resource_type, name);
    extract_block_by_header(content, &header)
}

/// Core brace-counting block extractor.  Finds the first block whose opening
/// line starts with `header` (after leading whitespace) and returns the full
/// raw text up to and including the matching closing `}`.
fn extract_block_by_header(content: &str, header: &str) -> Option<String> {
    let mut collecting = false;
    let mut depth: i32 = 0;
    let mut block_lines: Vec<&str> = Vec::new();

    for line in content.lines() {
        if !collecting && line.trim_start().starts_with(header) {
            collecting = true;
        }
        if collecting {
            block_lines.push(line);
            depth += line.chars().filter(|&c| c == '{').count() as i32;
            depth -= line.chars().filter(|&c| c == '}').count() as i32;
            if depth == 0 && block_lines.len() > 1 {
                return Some(block_lines.join("\n"));
            }
        }
    }
    None
}

fn extract_attributes(body: &hcl::Body, attrs: &mut HashMap<String, String>, prefix: &str) {
    for attr in body.attributes() {
        let key = if prefix.is_empty() {
            attr.key().to_string()
        } else {
            format!("{}.{}", prefix, attr.key())
        };
        attrs.insert(key, format!("{}", attr.expr()));
    }
    for block in body.blocks() {
        let block_key = if prefix.is_empty() {
            block.identifier().to_string()
        } else {
            format!("{}.{}", prefix, block.identifier())
        };
        extract_attributes(block.body(), attrs, &block_key);
    }
}
