use crate::migration::generator::{ResolvedHclMap, SKIPPED_SENTINEL};
use crate::migration::types::{MigrationResult, MigrationStatus};
use crate::terraform::types::{PassthroughBlock, PassthroughKind};

static PRICING_JSON: &str = include_str!("../pricing.json");

#[derive(Debug, Clone)]
pub struct CostEntry {
    pub resource_name: String,
    pub upcloud_type: String,
    pub plan: String,
    pub monthly_eur: f64,
}

/// Extract a simple `key = "value"` or `key = expr` attribute from HCL text.
/// Handles any amount of whitespace around `=` and strips trailing `# comments`.
fn extract_attr(hcl: &str, attr: &str) -> Option<String> {
    for line in hcl.lines() {
        let trimmed = line.trim();
        // Key must match exactly (guard against e.g. "size_gb" when looking for "size")
        let Some(after_key) = trimmed.strip_prefix(attr) else {
            continue;
        };
        if !after_key.starts_with(|c: char| c == '=' || c.is_whitespace()) {
            continue;
        }
        let after_eq = after_key.trim_start();
        let Some(rhs) = after_eq.strip_prefix('=') else {
            continue;
        };
        // Strip trailing comment — the first ` #` that is not inside quotes
        let rhs = rhs.trim();
        let rhs = rhs.find(" #").map(|p| &rhs[..p]).unwrap_or(rhs).trim();
        let val = rhs.trim_matches('"');
        // Exclude TODO placeholders and interpolation blocks, but allow variable refs (var.X)
        if !val.is_empty() && !val.contains('<') && !val.contains('$') && !val.contains('{') {
            return Some(val.to_string());
        }
    }
    None
}

fn lookup_server_plan(plan: &str, pricing: &serde_json::Value) -> Option<f64> {
    for (_, list) in pricing["upcloud_server"]["plans"].as_object()? {
        if let Some(arr) = list.as_array() {
            for entry in arr {
                if entry["plan"].as_str() == Some(plan) {
                    return entry["eur_monthly"].as_f64();
                }
            }
        }
    }
    None
}

fn lookup_plan_price(upcloud_type: &str, plan: &str, pricing: &serde_json::Value) -> Option<f64> {
    match upcloud_type {
        "upcloud_server" => lookup_server_plan(plan, pricing),
        "upcloud_managed_database_postgresql" | "upcloud_managed_database_mysql" => {
            pricing["upcloud_managed_database_postgresql"]["plans"]
                .as_array()?
                .iter()
                .find(|e| e["plan"].as_str() == Some(plan))?["eur_monthly"]
                .as_f64()
        }
        "upcloud_managed_database_valkey" => pricing["upcloud_managed_database_valkey"]["plans"]
            .as_array()?
            .iter()
            .find(|e| e["plan"].as_str() == Some(plan))?["eur_monthly"]
            .as_f64(),
        "upcloud_managed_database_opensearch" => {
            pricing["upcloud_managed_database_opensearch"]["plans"]
                .as_array()?
                .iter()
                .find(|e| e["plan"].as_str() == Some(plan))?["eur_monthly"]
                .as_f64()
        }
        "upcloud_loadbalancer" => pricing["upcloud_loadbalancer"]["plans"]
            .as_array()?
            .iter()
            .find(|e| e["plan"].as_str() == Some(plan))?["eur_monthly"]
            .as_f64(),
        "upcloud_kubernetes_cluster" => pricing["upcloud_kubernetes_cluster"]["plans"]
            .as_array()?
            .iter()
            .find(|e| e["plan"].as_str() == Some(plan))?["eur_monthly"]
            .as_f64(),
        "upcloud_gateway" => pricing["upcloud_gateway"]["plans"]
            .as_array()?
            .iter()
            .find(|e| e["plan"].as_str() == Some(plan))?["eur_monthly"]
            .as_f64(),
        _ => None,
    }
}

/// Resolve the `count` attribute from HCL text, falling back to variable defaults.
/// Returns 1 if count is absent, non-numeric, or unresolvable.
fn resolve_count(hcl: &str, var_defaults: &std::collections::HashMap<String, String>) -> u32 {
    let raw = match extract_attr(hcl, "count") {
        Some(v) => v,
        None => return 1,
    };
    // Literal integer
    if let Ok(n) = raw.parse::<u32>() {
        return n.max(1);
    }
    // Variable reference
    if let Some(var_name) = raw.strip_prefix("var.")
        && let Some(default) = var_defaults.get(var_name)
        && let Ok(n) = default.parse::<u32>()
    {
        return n.max(1);
    }
    1
}

fn storage_monthly(size_gb: u64, tier: &str, pricing: &serde_json::Value) -> f64 {
    let Some(tiers) = pricing["upcloud_storage"]["tiers"].as_array() else {
        return 0.0;
    };
    for entry in tiers {
        if entry["tier"].as_str() == Some(tier)
            && let Some(m) = entry["per_gb"]["eur_monthly"].as_f64()
        {
            return size_gb as f64 * m;
        }
    }
    0.0
}

pub fn compute_costs(
    migration_results: &[MigrationResult],
    resolved_hcl_map: &ResolvedHclMap,
    passthroughs: &[PassthroughBlock],
) -> Vec<CostEntry> {
    let pricing: serde_json::Value =
        serde_json::from_str(PRICING_JSON).unwrap_or(serde_json::Value::Null);
    let mut entries: Vec<CostEntry> = Vec::new();

    // Build variable-name → default-value map from Variable passthroughs.
    // Used to resolve plan attributes that reference a variable (e.g. `plan = var.web_instance_type`).
    let mut var_defaults: std::collections::HashMap<String, String> =
        std::collections::HashMap::new();
    for pt in passthroughs {
        if matches!(pt.kind, PassthroughKind::Variable)
            && let (Some(name), Some(default)) = (&pt.name, extract_attr(&pt.raw_hcl, "default"))
        {
            var_defaults.insert(name.clone(), default);
        }
    }

    for result in migration_results {
        if matches!(
            result.status,
            MigrationStatus::Unsupported | MigrationStatus::Unknown
        ) {
            continue;
        }
        let upcloud_type = result.upcloud_type.as_str();
        if upcloud_type.is_empty() || upcloud_type.starts_with("unsupported") {
            continue;
        }

        // Prefer resolved HCL, fall back to upcloud_hcl from the mapper.
        // The generator stores resolved HCL under (aws_resource_type, resource_name),
        // matching the key used by the diff view.
        let hcl_opt = resolved_hcl_map
            .get(&(result.resource_type.clone(), result.resource_name.clone()))
            .cloned()
            .or_else(|| result.upcloud_hcl.clone());

        let Some(hcl) = hcl_opt else { continue };
        if hcl == SKIPPED_SENTINEL {
            continue;
        }

        let (plan, monthly_eur) = if upcloud_type == "upcloud_storage" {
            let size = extract_attr(&hcl, "size")
                .and_then(|s| s.parse::<u64>().ok())
                .unwrap_or(50);
            let tier = extract_attr(&hcl, "tier").unwrap_or_else(|| "maxiops".to_string());
            let monthly = storage_monthly(size, &tier, &pricing);
            (format!("{} GB {}", size, tier), monthly)
        } else if upcloud_type.starts_with("upcloud_managed_object_storage") {
            let monthly = pricing["upcloud_managed_object_storage"]["base_monthly"]
                .as_f64()
                .unwrap_or(5.0);
            ("≥250 GB (usage)".to_string(), monthly)
        } else if upcloud_type == "upcloud_file_storage" {
            let size = extract_attr(&hcl, "size")
                .and_then(|s| s.parse::<u64>().ok())
                .unwrap_or(100);
            let rate = pricing["upcloud_file_storage"]["per_gb"]["eur_monthly"]
                .as_f64()
                .unwrap_or(0.15);
            let monthly = size as f64 * rate;
            (format!("{} GB", size), monthly)
        } else {
            let raw_plan = extract_attr(&hcl, "plan").unwrap_or_default();
            // Resolve variable references (e.g. `var.web_instance_type`) to their defaults.
            let plan = if let Some(var_name) = raw_plan.strip_prefix("var.") {
                let default = var_defaults.get(var_name).cloned().unwrap_or(raw_plan);
                // If the default is still an AWS instance type, convert it to UpCloud plan.
                crate::migration::providers::aws::compute::aws_instance_type_to_upcloud_plan(
                    &default,
                )
                .map(|s| s.to_string())
                .unwrap_or(default)
            } else {
                raw_plan
            };
            let monthly = if !plan.is_empty() {
                lookup_plan_price(upcloud_type, &plan, &pricing).unwrap_or(0.0)
            } else {
                0.0
            };
            (plan, monthly)
        };

        let count = resolve_count(&hcl, &var_defaults);
        let (plan, monthly_eur) = if count > 1 {
            (format!("{} ×{}", plan, count), monthly_eur * count as f64)
        } else {
            (plan, monthly_eur)
        };

        entries.push(CostEntry {
            resource_name: result.resource_name.clone(),
            upcloud_type: upcloud_type.to_string(),
            plan,
            monthly_eur,
        });
    }

    // Sort: expensive first, then alphabetical
    entries.sort_by(|a, b| {
        b.monthly_eur
            .partial_cmp(&a.monthly_eur)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.resource_name.cmp(&b.resource_name))
    });

    entries
}

/// Short display name for an UpCloud resource type (strips the `upcloud_` prefix).
pub fn short_upcloud_type(t: &str) -> &str {
    t.strip_prefix("upcloud_").unwrap_or(t)
}
