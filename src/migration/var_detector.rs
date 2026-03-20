//! Multi-signal variable purpose detection for AWS → UpCloud migration.
//!
//! Each variable is scored across up to five signals:
//!   1. Default value matches a known AWS pattern (+5)
//!   2. Usage context — attribute name where the var is referenced (+3–5)
//!   3. Description text keywords (+2)
//!   4. Variable name keywords (+1)
//!
//! Confidence categories:  ≥8 HIGH · 5–7 MEDIUM · 3–4 LOW  (< 3 → ignore)
//! HIGH and MEDIUM auto-convert the default value and annotate.
//! LOW flags the variable with a review comment but still converts if the default
//! value is an unambiguous match.

use std::collections::HashMap;

use crate::migration::providers::aws::compute::aws_instance_type_to_upcloud_plan;

// ---------------------------------------------------------------------------
// Region data
// ---------------------------------------------------------------------------

const AWS_REGIONS: &[&str] = &[
    "us-east-1", "us-east-2", "us-west-1", "us-west-2",
    "ca-central-1", "ca-west-1",
    "eu-west-1", "eu-west-2", "eu-west-3",
    "eu-central-1", "eu-central-2",
    "eu-north-1", "eu-south-1", "eu-south-2",
    "ap-east-1",
    "ap-southeast-1", "ap-southeast-2", "ap-southeast-3", "ap-southeast-4",
    "ap-northeast-1", "ap-northeast-2", "ap-northeast-3",
    "ap-south-1", "ap-south-2",
    "sa-east-1",
    "me-south-1", "me-central-1",
    "af-south-1",
    "il-central-1",
];

/// Map an AWS region to the closest UpCloud zone.
pub fn aws_region_to_upcloud_zone(region: &str) -> Option<&'static str> {
    match region {
        "us-east-1" | "us-east-2" => Some("us-nyc1"),
        "us-west-1" | "us-west-2" => Some("us-chi1"),
        "ca-central-1" | "ca-west-1" => Some("us-nyc1"),
        "eu-west-1" | "eu-west-2" | "eu-west-3" => Some("de-fra1"),
        "eu-central-1" | "eu-central-2" => Some("de-fra1"),
        "eu-north-1" => Some("fi-hel1"),
        "eu-south-1" | "eu-south-2" => Some("pl-waw1"),
        "ap-east-1" => Some("sg-sin1"),
        "ap-southeast-1" | "ap-southeast-2" | "ap-southeast-3" | "ap-southeast-4" => Some("sg-sin1"),
        "ap-northeast-1" | "ap-northeast-2" | "ap-northeast-3" => Some("sg-sin1"),
        "ap-south-1" | "ap-south-2" => Some("sg-sin1"),
        "sa-east-1" => Some("us-nyc1"),
        "me-south-1" | "me-central-1" => Some("sg-sin1"),
        "af-south-1" => Some("de-fra1"),
        "il-central-1" => Some("de-fra1"),
        _ => None,
    }
}

fn is_aws_region(s: &str) -> bool {
    AWS_REGIONS.contains(&s)
}

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub enum VarKind {
    InstanceType,
    Region,
}

impl VarKind {
    pub fn label(&self) -> &'static str {
        match self {
            VarKind::InstanceType => "EC2 instance type → UpCloud plan",
            VarKind::Region       => "AWS region → UpCloud zone",
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
            5..=7       => "MEDIUM",
            _           => "LOW",
        }
    }
}

// ---------------------------------------------------------------------------
// Analysis entry point
// ---------------------------------------------------------------------------

/// Analyse a single variable and return a `VarConversion` if any signals fire.
///
/// * `name`         – variable name (e.g. `"aws_region"`)
/// * `default_val`  – unquoted default value, if present (e.g. `"us-east-1"`)
/// * `description`  – variable description string, if present
/// * `usage_attrs`  – attribute names where `var.<name>` appears in resource HCL
pub fn analyze_variable(
    name: &str,
    default_val: Option<&str>,
    description: Option<&str>,
    usage_attrs: &[String],
) -> Option<VarConversion> {
    let instance = score_instance_type(name, default_val, description, usage_attrs);
    let region   = score_region(name, default_val, description, usage_attrs);

    let best = match (instance, region) {
        (Some(i), Some(r)) => if i.confidence >= r.confidence { i } else { r },
        (Some(i), None)    => i,
        (None, Some(r))    => r,
        (None, None)       => return None,
    };

    // Only report if there's at least some evidence.
    if best.confidence >= 3 { Some(best) } else { None }
}

// ---------------------------------------------------------------------------
// Scoring functions
// ---------------------------------------------------------------------------

fn score_instance_type(
    name: &str,
    default_val: Option<&str>,
    description: Option<&str>,
    usage_attrs: &[String],
) -> Option<VarConversion> {
    let mut score = 0u8;
    let mut signals = Vec::new();
    let mut converted_value = None;
    let original_default = default_val.map(str::to_string);

    // Signal 1: default matches AWS EC2 instance type (e.g. t3.small)
    if let Some(dv) = default_val
        && let Some(plan) = aws_instance_type_to_upcloud_plan(dv)
    {
        score += 5;
        signals.push(format!("default '{}' is an AWS EC2 instance type", dv));
        converted_value = Some(plan.to_string());
    }

    // Signal 1b: default matches an RDS instance class (db.t3.medium) or
    // ElastiCache node type (cache.t3.micro) — these use different UpCloud plan
    // formats than EC2 server plans.
    if converted_value.is_none()
        && let Some(dv) = default_val
        && let Some(plan) = rds_or_cache_class_to_upcloud_plan(dv)
    {
        score += 5;
        signals.push(format!("default '{}' is an AWS RDS/ElastiCache instance class", dv));
        converted_value = Some(plan.to_string());
    }

    // Signal 2: used as instance_type / instance_types / instance_class / node_type attribute
    if usage_attrs.iter().any(|a| a == "instance_type") {
        score += 5;
        signals.push("referenced as 'instance_type' attribute in a resource".to_string());
    } else if usage_attrs.iter().any(|a| a == "instance_types") {
        score += 3;
        signals.push("referenced as 'instance_types' attribute in a resource".to_string());
    } else if usage_attrs.iter().any(|a| a == "instance_class") {
        score += 5;
        signals.push("referenced as 'instance_class' attribute in a resource (RDS)".to_string());
    } else if usage_attrs.iter().any(|a| a == "node_type") {
        score += 5;
        signals.push("referenced as 'node_type' attribute in a resource (ElastiCache)".to_string());
    }

    // Signal 3: description keywords
    if let Some(desc) = description {
        let dl = desc.to_lowercase();
        let kw = ["instance type", "machine type", "server size", "ec2 instance", "instance class", "compute type", "node type"];
        if kw.iter().any(|k| dl.contains(k)) {
            score += 2;
            signals.push("description mentions instance/machine type".to_string());
        }
    }

    // Signal 4: variable name keywords
    let nl = name.to_lowercase();
    if nl.contains("instance_type") || nl.contains("machine_type") || nl.contains("server_size")
        || nl.contains("instance_class") || nl.contains("node_type")
    {
        score += 1;
        signals.push("variable name suggests instance type".to_string());
    }

    if score == 0 { return None; }
    Some(VarConversion { kind: VarKind::InstanceType, confidence: score.min(10), converted_value, original_default, signals })
}

/// Map an AWS RDS instance class or ElastiCache node type to the corresponding UpCloud
/// managed database plan. Returns `None` for unrecognised patterns.
///
/// RDS classes use the `db.` prefix and map to plans with a storage component
/// (e.g. `1x2xCPU-4GB-50GB`). ElastiCache node types use the `cache.` prefix
/// and map to plans without storage (e.g. `1x2xCPU-4GB`).
fn rds_or_cache_class_to_upcloud_plan(class: &str) -> Option<&'static str> {
    let (is_rds, stripped) = if let Some(s) = class.strip_prefix("db.") {
        (true, s)
    } else if let Some(s) = class.strip_prefix("cache.") {
        (false, s)
    } else {
        return None;
    };

    // Match on the size portion (after stripping vendor-specific prefix like "t3.", "r6g.", etc.)
    let size = stripped.rsplit('.').next().unwrap_or(stripped);
    match size {
        "nano" | "micro" | "small" => {
            if is_rds { Some("1x1xCPU-2GB-25GB") } else { Some("1x1xCPU-2GB") }
        }
        "medium" => {
            if is_rds { Some("1x2xCPU-4GB-50GB") } else { Some("1x2xCPU-4GB") }
        }
        "large" => {
            if is_rds { Some("2x4xCPU-8GB-100GB") } else { Some("1x2xCPU-8GB") }
        }
        "xlarge" => {
            if is_rds { Some("2x6xCPU-16GB-100GB") } else { Some("1x4xCPU-28GB") }
        }
        s if s.ends_with("xlarge") => {
            if is_rds { Some("2x8xCPU-32GB-100GB") } else { Some("1x8xCPU-56GB") }
        }
        _ => None,
    }
}

fn score_region(
    name: &str,
    default_val: Option<&str>,
    description: Option<&str>,
    usage_attrs: &[String],
) -> Option<VarConversion> {
    let mut score = 0u8;
    let mut signals = Vec::new();
    let mut converted_value = None;
    let original_default = default_val.map(str::to_string);

    // Signal 1: default matches AWS region
    if let Some(dv) = default_val
        && is_aws_region(dv)
    {
        score += 5;
        signals.push(format!("default '{}' is an AWS region code", dv));
        converted_value = aws_region_to_upcloud_zone(dv).map(str::to_string);
    }

    // Signal 2: used as region-related attribute
    for attr in usage_attrs {
        match attr.as_str() {
            "region" => {
                score += 5;
                signals.push("referenced as 'region' attribute in a resource or provider".to_string());
                break;
            }
            "availability_zone" | "az" => {
                score += 3;
                signals.push("referenced as 'availability_zone' attribute".to_string());
                break;
            }
            _ => {}
        }
    }

    // Signal 3: description keywords
    if let Some(desc) = description {
        let dl = desc.to_lowercase();
        let kw = ["region", "location", "datacenter", "data center", "geography", "area"];
        if kw.iter().any(|k| dl.contains(k)) {
            score += 2;
            signals.push("description mentions region/location".to_string());
        }
    }

    // Signal 4: variable name keywords
    let nl = name.to_lowercase();
    if nl.contains("region") || nl.contains("location") || nl == "zone"
        || nl.contains("datacenter") || nl.contains("data_center")
    {
        score += 1;
        signals.push("variable name suggests region/zone".to_string());
    }

    if score == 0 { return None; }
    Some(VarConversion { kind: VarKind::Region, confidence: score.min(10), converted_value, original_default, signals })
}

// ---------------------------------------------------------------------------
// HCL parsing helpers
// ---------------------------------------------------------------------------

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
        let opens  = trimmed.chars().filter(|&c| c == '{').count();
        let closes = trimmed.chars().filter(|&c| c == '}').count();
        if trimmed.starts_with("validation") { in_validation = depth == 1; }
        depth = depth.saturating_add(opens).saturating_sub(closes);
        if in_validation && depth > 1 { continue; }
        if depth == 0 { in_validation = false; }

        if trimmed.starts_with("default") && !in_validation {
            if let Some(rest) = trimmed.strip_prefix("default").map(|s| s.trim_start())
                && let Some(rest) = rest.strip_prefix('=')
            {
                let val = rest.trim().trim_end_matches(',');
                if val.starts_with('"') && val.ends_with('"') && val.len() >= 2 {
                    default_val = Some(val[1..val.len() - 1].to_string());
                }
            }
        } else if trimmed.starts_with("description") && !in_validation
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
            if trimmed.starts_with('#')
                || trimmed.is_empty()
                || trimmed == "{"
                || trimmed == "}"
            {
                continue;
            }
            let Some(eq_pos) = trimmed.find('=') else { continue };
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
                    map.entry(var_name.to_string()).or_default().push(attr.to_string());
                }
                if var_start >= search.len() { break; }
                search = &search[var_start..];
            }
        }
    }
    map
}

// ---------------------------------------------------------------------------
// HCL rewriter + annotation builder
// ---------------------------------------------------------------------------

/// Return annotation comment lines to prepend to the variable block.
pub fn build_var_annotation(name: &str, conversion: &VarConversion) -> String {
    let label = conversion.confidence_label();
    let kind  = conversion.kind.label();

    let mut out = String::new();

    match conversion.confidence {
        8..=u8::MAX => {
            // HIGH: brief one-liner
            if let (Some(orig), Some(cv)) = (&conversion.original_default, &conversion.converted_value) {
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
            if let (Some(orig), Some(cv)) = (&conversion.original_default, &conversion.converted_value) {
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
            if let (Some(orig), Some(cv)) = (&conversion.original_default, &conversion.converted_value) {
                out.push_str(&format!(
                    "# AUTO-CONVERTED [{}confidence]: '{}' → '{}' ({})\n",
                    label.to_lowercase(), orig, cv, kind
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
    let Some(orig) = &conversion.original_default else { return raw_hcl.to_string(); };
    let Some(cv)   = &conversion.converted_value   else { return raw_hcl.to_string(); };

    let target      = format!("\"{}\"", orig);
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

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // ── extract_variable_info ─────────────────────────────────────────────────

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
        assert_eq!(d.as_deref(), Some("us-east-1"), "real default must be found");
    }

    // ── build_var_usage_map ───────────────────────────────────────────────────

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

    // ── analyze_variable ─────────────────────────────────────────────────────

    #[test]
    fn high_confidence_instance_type_from_default_and_usage() {
        let conv = analyze_variable(
            "instance_type",
            Some("t3.micro"),
            Some("EC2 instance type"),
            &[String::from("instance_type")],
        ).unwrap();
        assert_eq!(conv.kind, VarKind::InstanceType);
        assert!(conv.confidence >= 8, "should be HIGH: {}", conv.confidence);
        assert_eq!(conv.converted_value.as_deref(), Some("1xCPU-1GB"));
    }

    #[test]
    fn high_confidence_region_from_default_and_usage() {
        let conv = analyze_variable(
            "aws_region",
            Some("us-east-1"),
            Some("AWS region to deploy resources"),
            &[String::from("region")],
        ).unwrap();
        assert_eq!(conv.kind, VarKind::Region);
        assert!(conv.confidence >= 8, "should be HIGH: {}", conv.confidence);
        assert_eq!(conv.converted_value.as_deref(), Some("us-nyc1"));
    }

    #[test]
    fn medium_confidence_region_from_name_and_default_only() {
        let conv = analyze_variable(
            "deploy_region",
            Some("eu-west-1"),
            None,
            &[],
        ).unwrap();
        assert_eq!(conv.kind, VarKind::Region);
        // default match (+5) + name (+1) = 6 → MEDIUM
        assert!(conv.confidence >= 5, "should be at least MEDIUM: {}", conv.confidence);
        assert_eq!(conv.converted_value.as_deref(), Some("de-fra1"));
    }

    #[test]
    fn low_confidence_instance_type_from_name_only() {
        let conv = analyze_variable("instance_type", None, None, &[]);
        // Only name signal (+1) — below threshold of 3, should be None
        assert!(conv.is_none(), "name-only should not reach confidence threshold");
    }

    #[test]
    fn no_signals_returns_none() {
        let result = analyze_variable("my_bucket", Some("my-bucket-name"), None, &[]);
        assert!(result.is_none());
    }

    #[test]
    fn picks_higher_confidence_kind() {
        // "region" name + region default + region usage: clearly region, not instance type
        let conv = analyze_variable(
            "region",
            Some("ap-southeast-1"),
            None,
            &[String::from("region")],
        ).unwrap();
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
        assert!(!out.contains("us-east-1"), "original value should be replaced\n{out}");
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
        assert!(out.contains("description = \"default is us-east-1\""), "description unchanged\n{out}");
        assert!(out.contains("default = \"us-nyc1\""), "default rewritten\n{out}");
    }

    // ── RDS instance class / ElastiCache node type detection ─────────────────

    #[test]
    fn rds_instance_class_detected_and_converted() {
        let conv = analyze_variable(
            "db_instance_class",
            Some("db.t3.medium"),
            Some("RDS instance class for PostgreSQL"),
            &[String::from("instance_class")],
        ).unwrap();
        assert_eq!(conv.kind, VarKind::InstanceType);
        assert!(conv.confidence >= 8, "confidence too low: {}", conv.confidence);
        assert_eq!(conv.converted_value.as_deref(), Some("1x2xCPU-4GB-50GB"));
    }

    #[test]
    fn cache_node_type_detected_and_converted() {
        let conv = analyze_variable(
            "cache_node_type",
            Some("cache.t3.micro"),
            Some("ElastiCache node type"),
            &[String::from("node_type")],
        ).unwrap();
        assert_eq!(conv.kind, VarKind::InstanceType);
        assert!(conv.confidence >= 8, "confidence too low: {}", conv.confidence);
        assert_eq!(conv.converted_value.as_deref(), Some("1x1xCPU-2GB"));
    }

    #[test]
    fn rds_xlarge_maps_to_correct_plan() {
        let conv = analyze_variable(
            "db_class",
            Some("db.m5.xlarge"),
            None,
            &[String::from("instance_class")],
        ).unwrap();
        assert_eq!(conv.converted_value.as_deref(), Some("2x6xCPU-16GB-100GB"));
    }

    #[test]
    fn rds_2xlarge_maps_to_correct_plan() {
        let conv = analyze_variable(
            "db_class",
            Some("db.r5.2xlarge"),
            None,
            &[String::from("instance_class")],
        ).unwrap();
        assert_eq!(conv.converted_value.as_deref(), Some("2x8xCPU-32GB-100GB"));
    }

    #[test]
    fn cache_xlarge_maps_to_correct_plan() {
        let conv = analyze_variable(
            "redis_node_type",
            Some("cache.r6g.xlarge"),
            None,
            &[String::from("node_type")],
        ).unwrap();
        assert_eq!(conv.converted_value.as_deref(), Some("1x4xCPU-28GB"));
    }

    #[test]
    fn rds_class_name_only_below_threshold() {
        // Variable named "db_class" with no default and no usage signals
        // should not reach the threshold
        let result = analyze_variable("db_class", None, None, &[]);
        assert!(result.is_none(), "name-only RDS variable should not convert: {:?}", result);
    }
}
