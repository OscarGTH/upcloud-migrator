//! Provider-agnostic variable detection framework.
//! Delegates to provider-specific [`VarDetector`] implementations.

use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq)]
pub enum VarKind {
    InstanceType,
    Region,
}

impl VarKind {
    pub fn label(&self) -> &'static str {
        match self {
            VarKind::InstanceType => "instance type → UpCloud plan",
            VarKind::Region => "region → UpCloud zone",
        }
    }
}

#[derive(Debug, Clone)]
pub struct VarConversion {
    pub kind: VarKind,
    /// Confidence score 0–10.
    pub confidence: u8,
    /// Converted default value (e.g. "1xCPU-1GB" or "fi-hel1"). None if no default to convert.
    pub converted_value: Option<String>,
    /// Original default value before conversion.
    pub original_default: Option<String>,
    /// Human-readable list of signals that contributed to the confidence.
    pub signals: Vec<String>,
}

impl VarConversion {
    pub fn confidence_label(&self) -> &'static str {
        match self.confidence {
            8..=u8::MAX => "HIGH",
            5..=7 => "MEDIUM",
            _ => "LOW",
        }
    }
}

/// Per-provider variable detection plug-in.
///
/// Implement this trait in each provider module (e.g. `providers::aws::var_detector`)
/// to teach the framework how to recognise and convert provider-specific variable
/// patterns such as region codes and instance/machine types.
pub trait VarDetector: Send + Sync {
    /// Return all candidate [`VarConversion`]s for the given variable.
    ///
    /// The framework picks the highest-confidence result that crosses the
    /// minimum threshold (≥ 3). Return an empty `Vec` if no signals fire.
    fn detect(
        &self,
        name: &str,
        default_val: Option<&str>,
        description: Option<&str>,
        usage_attrs: &[String],
    ) -> Vec<VarConversion>;
}

/// Analyse a variable using a specific provider detector and return the best
/// [`VarConversion`] if confidence reaches the minimum threshold (≥ 3).
pub fn analyze_variable_with(
    detector: &dyn VarDetector,
    name: &str,
    default_val: Option<&str>,
    description: Option<&str>,
    usage_attrs: &[String],
) -> Option<VarConversion> {
    detector
        .detect(name, default_val, description, usage_attrs)
        .into_iter()
        .filter(|c| c.confidence >= 3)
        .max_by_key(|c| c.confidence)
}

/// Extract `(default_value, description)` from a `variable "..." { ... }` HCL block.
/// Returns unquoted string values; non-string or absent fields return `None`.
pub fn extract_variable_info(raw_hcl: &str) -> (Option<String>, Option<String>) {
    let mut default_val = None;
    let mut description = None;
    let mut depth = 0usize;
    let mut in_validation = false;

    for line in raw_hcl.lines() {
        let trimmed = line.trim_start();

        // Track brace depth so we skip nested validation { } blocks.
        let opens = trimmed.chars().filter(|&c| c == '{').count();
        let closes = trimmed.chars().filter(|&c| c == '}').count();
        if trimmed.starts_with("validation") {
            in_validation = depth == 1;
        }
        depth = depth.saturating_add(opens).saturating_sub(closes);
        if in_validation && depth > 1 {
            continue;
        }
        if depth == 0 {
            in_validation = false;
        }

        if trimmed.starts_with("default") && !in_validation {
            if let Some(rest) = trimmed.strip_prefix("default").map(|s| s.trim_start())
                && let Some(rest) = rest.strip_prefix('=')
            {
                let val = rest.trim().trim_end_matches(',');
                if val.starts_with('"') && val.ends_with('"') && val.len() >= 2 {
                    default_val = Some(val[1..val.len() - 1].to_string());
                }
            }
        } else if trimmed.starts_with("description")
            && !in_validation
            && let Some(rest) = trimmed.strip_prefix("description").map(|s| s.trim_start())
            && let Some(rest) = rest.strip_prefix('=')
        {
            let val = rest.trim().trim_end_matches(',');
            if val.starts_with('"') && val.ends_with('"') && val.len() >= 2 {
                description = Some(val[1..val.len() - 1].to_string());
            }
        }
    }

    (default_val, description)
}

// ---------------------------------------------------------------------------
// Usage-map builder
// ---------------------------------------------------------------------------

/// Build a map `variable_name → Vec<attribute_name>` by scanning raw source HCL blocks
/// for lines of the form `attr = ... var.NAME ...`.
pub fn build_var_usage_map(source_hcl_blocks: &[&str]) -> HashMap<String, Vec<String>> {
    let mut map: HashMap<String, Vec<String>> = HashMap::new();

    for hcl in source_hcl_blocks {
        for line in hcl.lines() {
            let trimmed = line.trim_start();
            // Skip comments and pure block lines.
            if trimmed.starts_with('#') || trimmed.is_empty() || trimmed == "{" || trimmed == "}" {
                continue;
            }
            let Some(eq_pos) = trimmed.find('=') else {
                continue;
            };
            let attr = trimmed[..eq_pos].trim();
            // Skip block headers (contain spaces/quotes) and comparison operators.
            if attr.contains('"') || attr.contains(' ') || attr.is_empty() {
                continue;
            }
            let rhs = &trimmed[eq_pos + 1..];
            // Scan for all `var.NAME` references on this RHS.
            let mut search = rhs;
            while let Some(var_pos) = search.find("var.") {
                let var_start = var_pos + 4;
                let var_end = search[var_start..]
                    .find(|c: char| !c.is_alphanumeric() && c != '_')
                    .map(|off| var_start + off)
                    .unwrap_or(search.len());
                let var_name = &search[var_start..var_end];
                if !var_name.is_empty() {
                    map.entry(var_name.to_string())
                        .or_default()
                        .push(attr.to_string());
                }
                if var_start >= search.len() {
                    break;
                }
                search = &search[var_start..];
            }
        }
    }
    map
}

/// Return annotation comment lines to prepend to the variable block.
pub fn build_var_annotation(name: &str, conversion: &VarConversion) -> String {
    let label = conversion.confidence_label();
    let kind = conversion.kind.label();

    let mut out = String::new();

    match conversion.confidence {
        8..=u8::MAX => {
            // HIGH: brief one-liner
            if let (Some(orig), Some(cv)) =
                (&conversion.original_default, &conversion.converted_value)
            {
                out.push_str(&format!(
                    "# AUTO-CONVERTED [{}]: '{}' → '{}' ({})\n",
                    label, orig, cv, kind
                ));
            } else {
                out.push_str(&format!(
                    "# REVIEW [{}]: '{}' is likely a {} variable — update manually\n",
                    label, name, kind
                ));
                for sig in &conversion.signals {
                    out.push_str(&format!("#   · {}\n", sig));
                }
            }
        }
        5..=7 => {
            // MEDIUM: conversion note + verify prompt
            if let (Some(orig), Some(cv)) =
                (&conversion.original_default, &conversion.converted_value)
            {
                out.push_str(&format!(
                    "# AUTO-CONVERTED [{}]: '{}' → '{}' ({})\n",
                    label, orig, cv, kind
                ));
                out.push_str("#   Verify this mapping suits your use case\n");
            } else {
                out.push_str(&format!(
                    "# REVIEW [{}]: '{}' may be a {} variable\n",
                    label, name, kind
                ));
                for sig in &conversion.signals {
                    out.push_str(&format!("#   · {}\n", sig));
                }
            }
        }
        _ => {
            // LOW: flag only; convert the default if it's unambiguous, but warn clearly
            if let (Some(orig), Some(cv)) =
                (&conversion.original_default, &conversion.converted_value)
            {
                out.push_str(&format!(
                    "# AUTO-CONVERTED [{}confidence]: '{}' → '{}' ({})\n",
                    label.to_lowercase(),
                    orig,
                    cv,
                    kind
                ));
                out.push_str("#   Low confidence — verify this is correct before applying\n");
            } else {
                // No default to convert — only flag if there's actual usage context
                if conversion.signals.len() >= 2 {
                    out.push_str(&format!(
                        "# POSSIBLE REVIEW: '{}' might be a {} variable (low confidence)\n",
                        name, kind
                    ));
                    for sig in &conversion.signals {
                        out.push_str(&format!("#   · {}\n", sig));
                    }
                }
            }
        }
    }
    out
}

/// Rewrite the `default = "..."` line in a variable block to use the converted value.
/// All other lines are preserved verbatim.
pub fn apply_conversion_to_hcl(raw_hcl: &str, conversion: &VarConversion) -> String {
    let Some(orig) = &conversion.original_default else {
        return raw_hcl.to_string();
    };
    let Some(cv) = &conversion.converted_value else {
        return raw_hcl.to_string();
    };

    let target = format!("\"{}\"", orig);
    let replacement = format!("\"{}\"", cv);

    let mut out = String::with_capacity(raw_hcl.len());
    for line in raw_hcl.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("default") && line.contains(&target) {
            out.push_str(&line.replacen(&target, &replacement, 1));
        } else {
            out.push_str(line);
        }
        out.push('\n');
    }
    if !raw_hcl.ends_with('\n') && out.ends_with('\n') {
        out.pop();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::migration::providers::aws::var_detector::AwsVarDetector;

    #[test]
    fn extracts_default_and_description() {
        let hcl = "variable \"region\" {\n  description = \"AWS region\"\n  type = string\n  default = \"us-east-1\"\n}";
        let (d, desc) = extract_variable_info(hcl);
        assert_eq!(d.as_deref(), Some("us-east-1"));
        assert_eq!(desc.as_deref(), Some("AWS region"));
    }

    #[test]
    fn returns_none_for_no_default() {
        let hcl = "variable \"tok\" {\n  type = string\n  sensitive = true\n}";
        let (d, _) = extract_variable_info(hcl);
        assert!(d.is_none());
    }

    #[test]
    fn skips_default_inside_validation_block() {
        let hcl = concat!(
            "variable \"x\" {\n",
            "  default = \"us-east-1\"\n",
            "  validation {\n",
            "    condition = contains([\"us-east-1\"], var.x)\n",
            "    error_message = \"must be us-east-1\"\n",
            "  }\n",
            "}\n"
        );
        let (d, _) = extract_variable_info(hcl);
        assert_eq!(
            d.as_deref(),
            Some("us-east-1"),
            "real default must be found"
        );
    }

    #[test]
    fn detects_instance_type_usage() {
        let hcl = "resource \"aws_instance\" \"web\" {\n  instance_type = var.instance_type\n}";
        let map = build_var_usage_map(&[hcl]);
        assert!(map["instance_type"].contains(&"instance_type".to_string()));
    }

    #[test]
    fn detects_region_usage() {
        let hcl = "provider \"aws\" {\n  region = var.aws_region\n}";
        let map = build_var_usage_map(&[hcl]);
        assert!(map["aws_region"].contains(&"region".to_string()));
    }

    #[test]
    fn high_confidence_instance_type_from_default_and_usage() {
        let conv = analyze_variable_with(
            &AwsVarDetector,
            "instance_type",
            Some("t3.micro"),
            Some("EC2 instance type"),
            &[String::from("instance_type")],
        )
        .unwrap();
        assert_eq!(conv.kind, VarKind::InstanceType);
        assert!(conv.confidence >= 8, "should be HIGH: {}", conv.confidence);
        assert_eq!(conv.converted_value.as_deref(), Some("1xCPU-1GB"));
    }

    #[test]
    fn high_confidence_region_from_default_and_usage() {
        let conv = analyze_variable_with(
            &AwsVarDetector,
            "aws_region",
            Some("us-east-1"),
            Some("AWS region to deploy resources"),
            &[String::from("region")],
        )
        .unwrap();
        assert_eq!(conv.kind, VarKind::Region);
        assert!(conv.confidence >= 8, "should be HIGH: {}", conv.confidence);
        assert_eq!(conv.converted_value.as_deref(), Some("us-nyc1"));
    }

    #[test]
    fn medium_confidence_region_from_name_and_default_only() {
        let conv = analyze_variable_with(&AwsVarDetector, "deploy_region", Some("eu-west-1"), None, &[]).unwrap();
        assert_eq!(conv.kind, VarKind::Region);
        // default match (+5) + name (+1) = 6 → MEDIUM
        assert!(
            conv.confidence >= 5,
            "should be at least MEDIUM: {}",
            conv.confidence
        );
        assert_eq!(conv.converted_value.as_deref(), Some("de-fra1"));
    }

    #[test]
    fn low_confidence_instance_type_from_name_only() {
        let conv = analyze_variable_with(&AwsVarDetector, "instance_type", None, None, &[]);
        // Only name signal (+1) — below threshold of 3, should be None
        assert!(
            conv.is_none(),
            "name-only should not reach confidence threshold"
        );
    }

    #[test]
    fn no_signals_returns_none() {
        let result = analyze_variable_with(&AwsVarDetector, "my_bucket", Some("my-bucket-name"), None, &[]);
        assert!(result.is_none());
    }

    #[test]
    fn picks_higher_confidence_kind() {
        // "region" name + region default + region usage: clearly region, not instance type
        let conv = analyze_variable_with(
            &AwsVarDetector,
            "region",
            Some("ap-southeast-1"),
            None,
            &[String::from("region")],
        )
        .unwrap();
        assert_eq!(conv.kind, VarKind::Region);
    }

    // ── apply_conversion_to_hcl ───────────────────────────────────────────────

    #[test]
    fn rewrites_default_line_only() {
        let hcl = "variable \"r\" {\n  default = \"us-east-1\"\n}";
        let conv = VarConversion {
            kind: VarKind::Region,
            confidence: 9,
            converted_value: Some("us-nyc1".to_string()),
            original_default: Some("us-east-1".to_string()),
            signals: vec![],
        };
        let out = apply_conversion_to_hcl(hcl, &conv);
        assert!(out.contains("default = \"us-nyc1\""), "{out}");
        assert!(
            !out.contains("us-east-1"),
            "original value should be replaced\n{out}"
        );
    }

    #[test]
    fn leaves_description_containing_original_value_unchanged() {
        let hcl = concat!(
            "variable \"r\" {\n",
            "  description = \"default is us-east-1\"\n",
            "  default = \"us-east-1\"\n",
            "}"
        );
        let conv = VarConversion {
            kind: VarKind::Region,
            confidence: 9,
            converted_value: Some("us-nyc1".to_string()),
            original_default: Some("us-east-1".to_string()),
            signals: vec![],
        };
        let out = apply_conversion_to_hcl(hcl, &conv);
        // Description must be untouched; only the default line is rewritten.
        assert!(
            out.contains("description = \"default is us-east-1\""),
            "description unchanged\n{out}"
        );
        assert!(
            out.contains("default = \"us-nyc1\""),
            "default rewritten\n{out}"
        );
    }

    // ── RDS instance class / ElastiCache node type detection ─────────────────

    #[test]
    fn rds_instance_class_detected_and_converted() {
        let conv = analyze_variable_with(
            &AwsVarDetector,
            "db_instance_class",
            Some("db.t3.medium"),
            Some("RDS instance class for PostgreSQL"),
            &[String::from("instance_class")],
        )
        .unwrap();
        assert_eq!(conv.kind, VarKind::InstanceType);
        assert!(
            conv.confidence >= 8,
            "confidence too low: {}",
            conv.confidence
        );
        assert_eq!(conv.converted_value.as_deref(), Some("1x2xCPU-4GB-50GB"));
    }

    #[test]
    fn cache_node_type_detected_and_converted() {
        let conv = analyze_variable_with(
            &AwsVarDetector,
            "cache_node_type",
            Some("cache.t3.micro"),
            Some("ElastiCache node type"),
            &[String::from("node_type")],
        )
        .unwrap();
        assert_eq!(conv.kind, VarKind::InstanceType);
        assert!(
            conv.confidence >= 8,
            "confidence too low: {}",
            conv.confidence
        );
        assert_eq!(conv.converted_value.as_deref(), Some("1x1xCPU-2GB"));
    }

    #[test]
    fn rds_xlarge_maps_to_correct_plan() {
        let conv = analyze_variable_with(
            &AwsVarDetector,
            "db_class",
            Some("db.m5.xlarge"),
            None,
            &[String::from("instance_class")],
        )
        .unwrap();
        assert_eq!(conv.converted_value.as_deref(), Some("2x6xCPU-16GB-100GB"));
    }

    #[test]
    fn rds_2xlarge_maps_to_correct_plan() {
        let conv = analyze_variable_with(
            &AwsVarDetector,
            "db_class",
            Some("db.r5.2xlarge"),
            None,
            &[String::from("instance_class")],
        )
        .unwrap();
        assert_eq!(conv.converted_value.as_deref(), Some("2x8xCPU-32GB-100GB"));
    }

    #[test]
    fn cache_xlarge_maps_to_correct_plan() {
        let conv = analyze_variable_with(
            &AwsVarDetector,
            "redis_node_type",
            Some("cache.r6g.xlarge"),
            None,
            &[String::from("node_type")],
        )
        .unwrap();
        assert_eq!(conv.converted_value.as_deref(), Some("1x4xCPU-28GB"));
    }

    #[test]
    fn rds_class_name_only_below_threshold() {
        // Variable named "db_class" with no default and no usage signals
        // should not reach the threshold
        let result = analyze_variable_with(&AwsVarDetector, "db_class", None, None, &[]);
        assert!(
            result.is_none(),
            "name-only RDS variable should not convert: {:?}",
            result
        );
    }
}
