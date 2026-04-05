//! Azure variable detection — recognises Azure VM sizes and Azure regions.

use crate::migration::providers::azure::compute::azure_vm_size_to_upcloud_plan;
use crate::migration::var_detector::{VarConversion, VarDetector, VarKind};

const AZURE_REGIONS: &[&str] = &[
    "eastus",
    "eastus2",
    "centralus",
    "northcentralus",
    "southcentralus",
    "westus",
    "westus2",
    "westus3",
    "canadacentral",
    "canadaeast",
    "northeurope",
    "westeurope",
    "uksouth",
    "ukwest",
    "francecentral",
    "francesouth",
    "germanywestcentral",
    "swedencentral",
    "norwayeast",
    "southeastasia",
    "eastasia",
    "japaneast",
    "japanwest",
    "koreacentral",
    "australiaeast",
    "australiasoutheast",
    "brazilsouth",
    "centralindia",
    "southindia",
    "westindia",
    "uaenorth",
    "southafricanorth",
];

/// Map an Azure region to the closest UpCloud zone.
pub fn azure_region_to_upcloud_zone(region: &str) -> Option<&'static str> {
    match region {
        "eastus" | "eastus2" | "centralus" | "northcentralus" | "southcentralus" => Some("us-nyc1"),
        "westus" | "westus2" | "westus3" => Some("us-chi1"),
        "canadacentral" | "canadaeast" => Some("us-nyc1"),
        "northeurope" | "uksouth" | "ukwest" => Some("de-fra1"),
        "westeurope" | "germanywestcentral" | "francecentral" | "francesouth" => Some("de-fra1"),
        "swedencentral" | "norwayeast" => Some("fi-hel1"),
        "southeastasia" | "eastasia" => Some("sg-sin1"),
        "japaneast" | "japanwest" | "koreacentral" => Some("sg-sin1"),
        "australiaeast" | "australiasoutheast" => Some("au-syd1"),
        "brazilsouth" => Some("us-nyc1"),
        "centralindia" | "southindia" | "westindia" => Some("sg-sin1"),
        "uaenorth" => Some("sg-sin1"),
        "southafricanorth" => Some("de-fra1"),
        _ => None,
    }
}

fn is_azure_region(s: &str) -> bool {
    AZURE_REGIONS.contains(&s)
}

pub struct AzureVarDetector;

impl VarDetector for AzureVarDetector {
    fn detect(
        &self,
        name: &str,
        default_val: Option<&str>,
        description: Option<&str>,
        usage_attrs: &[String],
    ) -> Vec<VarConversion> {
        let mut results = Vec::new();
        if let Some(c) = score_vm_size(name, default_val, description, usage_attrs) {
            results.push(c);
        }
        if let Some(c) = score_region(name, default_val, description, usage_attrs) {
            results.push(c);
        }
        results
    }
}

fn score_vm_size(
    name: &str,
    default_val: Option<&str>,
    description: Option<&str>,
    usage_attrs: &[String],
) -> Option<VarConversion> {
    let mut score = 0u8;
    let mut signals = Vec::new();
    let mut converted_value = None;
    let original_default = default_val.map(str::to_string);

    // Signal 1: default matches Azure VM size (e.g. Standard_B2s)
    if let Some(dv) = default_val
        && let Some(plan) = azure_vm_size_to_upcloud_plan(dv)
    {
        score += 5;
        signals.push(format!("default '{}' is an Azure VM size", dv));
        converted_value = Some(plan.to_string());
    }

    // Signal 2: used as size / vm_size attribute
    if usage_attrs.iter().any(|a| a == "size" || a == "vm_size") {
        score += 5;
        signals.push("referenced as 'size' or 'vm_size' attribute in a resource".to_string());
    }

    // Signal 3: description keywords
    if let Some(desc) = description {
        let dl = desc.to_lowercase();
        let kw = [
            "vm size",
            "virtual machine size",
            "instance size",
            "machine size",
            "sku",
        ];
        if kw.iter().any(|k| dl.contains(k)) {
            score += 2;
            signals.push("description mentions VM size".to_string());
        }
    }

    // Signal 4: variable name keywords
    let nl = name.to_lowercase();
    if nl.contains("vm_size")
        || nl.contains("instance_size")
        || nl.contains("machine_size")
        || nl.contains("sku_name")
    {
        score += 1;
        signals.push("variable name suggests VM size".to_string());
    }

    if score == 0 {
        return None;
    }
    Some(VarConversion {
        kind: VarKind::InstanceType,
        confidence: score.min(10),
        converted_value,
        original_default,
        signals,
    })
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

    // Signal 1: default is a known Azure region
    if let Some(dv) = default_val
        && is_azure_region(dv)
    {
        score += 5;
        signals.push(format!("default '{}' is an Azure region", dv));
        if let Some(zone) = azure_region_to_upcloud_zone(dv) {
            converted_value = Some(zone.to_string());
        }
    }

    // Signal 2: used as location attribute
    if usage_attrs.iter().any(|a| a == "location") {
        score += 5;
        signals.push("referenced as 'location' attribute in a resource".to_string());
    }

    // Signal 3: description keywords
    if let Some(desc) = description {
        let dl = desc.to_lowercase();
        let kw = ["region", "location", "azure region", "deployment region"];
        if kw.iter().any(|k| dl.contains(k)) {
            score += 2;
            signals.push("description mentions region/location".to_string());
        }
    }

    // Signal 4: variable name keywords
    let nl = name.to_lowercase();
    if nl.contains("region") || nl.contains("location") || nl == "azure_region" {
        score += 1;
        signals.push("variable name suggests region".to_string());
    }

    if score == 0 {
        return None;
    }
    Some(VarConversion {
        kind: VarKind::Region,
        confidence: score.min(10),
        converted_value,
        original_default,
        signals,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── azure_region_to_upcloud_zone ──────────────────────────────────────────

    #[test]
    fn eastus_maps_to_us_nyc1() {
        assert_eq!(azure_region_to_upcloud_zone("eastus"), Some("us-nyc1"));
    }

    #[test]
    fn eastus2_maps_to_us_nyc1() {
        assert_eq!(azure_region_to_upcloud_zone("eastus2"), Some("us-nyc1"));
    }

    #[test]
    fn westus_maps_to_us_chi1() {
        assert_eq!(azure_region_to_upcloud_zone("westus"), Some("us-chi1"));
    }

    #[test]
    fn swedencentral_maps_to_fi_hel1() {
        assert_eq!(
            azure_region_to_upcloud_zone("swedencentral"),
            Some("fi-hel1")
        );
    }

    #[test]
    fn norwayeast_maps_to_fi_hel1() {
        assert_eq!(azure_region_to_upcloud_zone("norwayeast"), Some("fi-hel1"));
    }

    #[test]
    fn southeastasia_maps_to_sg_sin1() {
        assert_eq!(
            azure_region_to_upcloud_zone("southeastasia"),
            Some("sg-sin1")
        );
    }

    #[test]
    fn australiaeast_maps_to_au_syd1() {
        assert_eq!(
            azure_region_to_upcloud_zone("australiaeast"),
            Some("au-syd1")
        );
    }

    #[test]
    fn unknown_region_returns_none() {
        assert_eq!(azure_region_to_upcloud_zone("middleofnowhere"), None);
    }

    // ── AzureVarDetector — VM size detection ──────────────────────────────────

    #[test]
    fn detects_vm_size_from_default_value() {
        let det = AzureVarDetector;
        let results = det.detect("instance_size", Some("Standard_B2s"), None, &[]);
        assert!(!results.is_empty(), "should detect a VM size variable");
        let conv = &results[0];
        assert_eq!(conv.kind, VarKind::InstanceType);
        assert_eq!(conv.converted_value.as_deref(), Some("2xCPU-4GB"));
    }

    #[test]
    fn detects_vm_size_from_usage_attr() {
        let det = AzureVarDetector;
        let results = det.detect("my_var", None, None, &["size".to_string()]);
        assert!(!results.is_empty(), "should detect from 'size' usage attr");
        assert_eq!(results[0].kind, VarKind::InstanceType);
    }

    #[test]
    fn detects_vm_size_from_vm_size_usage_attr() {
        let det = AzureVarDetector;
        let results = det.detect("my_var", None, None, &["vm_size".to_string()]);
        assert!(
            !results.is_empty(),
            "should detect from 'vm_size' usage attr"
        );
        assert_eq!(results[0].kind, VarKind::InstanceType);
    }

    #[test]
    fn detects_vm_size_from_variable_name() {
        let det = AzureVarDetector;
        let results = det.detect("vm_size", None, None, &[]);
        assert!(
            !results.is_empty(),
            "variable name 'vm_size' should trigger detection"
        );
        assert_eq!(results[0].kind, VarKind::InstanceType);
    }

    #[test]
    fn d4s_v5_default_converts_to_correct_plan() {
        let det = AzureVarDetector;
        let results = det.detect("size", Some("Standard_D4s_v5"), None, &[]);
        assert!(!results.is_empty());
        assert_eq!(results[0].converted_value.as_deref(), Some("4xCPU-8GB"));
    }

    // ── AzureVarDetector — region detection ───────────────────────────────────

    #[test]
    fn detects_region_from_default_value() {
        let det = AzureVarDetector;
        let results = det.detect("deploy_region", Some("eastus"), None, &[]);
        assert!(!results.is_empty(), "should detect an Azure region");
        let conv = &results[0];
        assert_eq!(conv.kind, VarKind::Region);
        assert_eq!(conv.converted_value.as_deref(), Some("us-nyc1"));
    }

    #[test]
    fn detects_region_from_location_usage_attr() {
        let det = AzureVarDetector;
        let results = det.detect("my_var", None, None, &["location".to_string()]);
        assert!(
            !results.is_empty(),
            "should detect from 'location' usage attr"
        );
        assert_eq!(results[0].kind, VarKind::Region);
    }

    #[test]
    fn detects_region_from_variable_name_containing_location() {
        let det = AzureVarDetector;
        let results = det.detect("azure_location", None, None, &[]);
        assert!(
            !results.is_empty(),
            "variable name 'azure_location' should trigger detection"
        );
        assert_eq!(results[0].kind, VarKind::Region);
    }

    #[test]
    fn unknown_default_value_returns_no_results() {
        let det = AzureVarDetector;
        let results = det.detect("my_flag", Some("true"), None, &[]);
        assert!(
            results.is_empty(),
            "unrecognised default should return nothing"
        );
    }
}
