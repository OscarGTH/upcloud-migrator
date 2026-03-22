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
            "provider" => {
                // Keep non-cloud-provider provider blocks (e.g. kubernetes, helm, null, random)
                let labels: Vec<&str> = block.labels().iter().map(|l| l.as_str()).collect();
                if let Some(&provider_name) = labels.first()
                    && !is_cloud_provider_name(provider_name)
                {
                    let raw_hcl =
                        extract_passthrough_block(&content, "provider", Some(provider_name))
                            .unwrap_or_default();
                    if !raw_hcl.is_empty() {
                        passthroughs.push(PassthroughBlock {
                            name: Some(provider_name.to_string()),
                            raw_hcl,
                            source_file: path.clone(),
                            kind: PassthroughKind::Provider,
                        });
                    }
                }
            }
            "data" => {
                // Keep non-cloud-provider data sources (e.g. external, http, local)
                let labels: Vec<&str> = block.labels().iter().map(|l| l.as_str()).collect();
                if labels.len() >= 2 {
                    let data_type = labels[0];
                    if !is_cloud_provider_type(data_type) {
                        let data_name = labels[1];
                        let raw_hcl = extract_block_by_header(
                            &content,
                            &format!("data \"{}\" \"{}\"", data_type, data_name),
                        )
                        .unwrap_or_default();
                        if !raw_hcl.is_empty() {
                            passthroughs.push(PassthroughBlock {
                                name: Some(format!("{}.{}", data_type, data_name)),
                                raw_hcl,
                                source_file: path.clone(),
                                kind: PassthroughKind::Data,
                            });
                        }
                    }
                }
            }
            "terraform" => {
                // Extract required_providers entries for non-cloud providers
                for inner_block in block.body().blocks() {
                    if inner_block.identifier() == "required_providers" {
                        for attr in inner_block.body().attributes() {
                            let provider_name = attr.key();
                            if !is_cloud_provider_name(provider_name) {
                                // Extract the raw text for this provider requirement
                                let raw_hcl =
                                    extract_required_provider_entry(&content, provider_name)
                                        .unwrap_or_else(|| {
                                            format_required_provider_from_expr(
                                                provider_name,
                                                attr.expr(),
                                            )
                                        });
                                if !raw_hcl.is_empty() {
                                    passthroughs.push(PassthroughBlock {
                                        name: Some(format!("required_provider:{}", provider_name)),
                                        raw_hcl,
                                        source_file: path.clone(),
                                        kind: PassthroughKind::Provider,
                                    });
                                }
                            }
                        }
                    }
                }
            }
            _ => {} // module blocks and others — ignored for now
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

/// Check whether a provider name belongs to a cloud provider that has (or will
/// have) migration support.
fn is_cloud_provider_name(name: &str) -> bool {
    matches!(name, "aws" | "azurerm" | "google" | "alicloud")
}

/// Check whether a resource/data source type belongs to a cloud provider.
fn is_cloud_provider_type(type_name: &str) -> bool {
    type_name.starts_with("aws_")
        || type_name.starts_with("azurerm_")
        || type_name.starts_with("google_")
        || type_name.starts_with("alicloud_")
}

/// Extract a single `provider_name = { ... }` entry from the `required_providers`
/// block within a `terraform { ... }` block in the source text.
fn extract_required_provider_entry(content: &str, provider_name: &str) -> Option<String> {
    // Find terraform { ... } block first
    let terraform_block = extract_block_by_header(content, "terraform")?;
    // Find required_providers { ... } within it
    let rp_block = extract_block_by_header(&terraform_block, "required_providers")?;
    // Find the specific provider entry: `provider_name = {`
    // The entry looks like: `    kubernetes = {\n      source = ...\n      version = ...\n    }`
    let pattern = format!("{} = {{", provider_name);
    let alt_pattern = format!("{}=", provider_name);
    let start = rp_block
        .find(&pattern)
        .or_else(|| rp_block.find(&alt_pattern))?;

    // Brace-count to find the end of the entry (wait until we've seen at least one `{`)
    let entry_start = rp_block[..start]
        .rfind('\n')
        .map(|p| p + 1)
        .unwrap_or(start);
    let mut depth = 0i32;
    let mut found_open = false;
    let mut end = start;
    for (i, ch) in rp_block[start..].char_indices() {
        if ch == '{' {
            depth += 1;
            found_open = true;
        }
        if ch == '}' {
            depth -= 1;
        }
        if found_open && depth == 0 {
            end = start + i + 1;
            break;
        }
    }
    if !found_open {
        return None;
    }
    Some(rp_block[entry_start..end].trim().to_string())
}

/// Fallback: reconstruct a required_provider entry from the parsed expression.
fn format_required_provider_from_expr(name: &str, expr: &hcl::Expression) -> String {
    format!("    {} = {}", name, expr)
}
