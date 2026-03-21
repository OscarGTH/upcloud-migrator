//! AWS variable detection — recognises EC2/RDS/ElastiCache types and AWS regions.

use crate::migration::providers::aws::compute::aws_instance_type_to_upcloud_plan;
use crate::migration::var_detector::{VarConversion, VarDetector, VarKind};

const AWS_REGIONS: &[&str] = &[
    "us-east-1",
    "us-east-2",
    "us-west-1",
    "us-west-2",
    "ca-central-1",
    "ca-west-1",
    "eu-west-1",
    "eu-west-2",
    "eu-west-3",
    "eu-central-1",
    "eu-central-2",
    "eu-north-1",
    "eu-south-1",
    "eu-south-2",
    "ap-east-1",
    "ap-southeast-1",
    "ap-southeast-2",
    "ap-southeast-3",
    "ap-southeast-4",
    "ap-northeast-1",
    "ap-northeast-2",
    "ap-northeast-3",
    "ap-south-1",
    "ap-south-2",
    "sa-east-1",
    "me-south-1",
    "me-central-1",
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
        "ap-southeast-1" | "ap-southeast-2" | "ap-southeast-3" | "ap-southeast-4" => {
            Some("sg-sin1")
        }
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

pub struct AwsVarDetector;

impl VarDetector for AwsVarDetector {
    fn detect(
        &self,
        name: &str,
        default_val: Option<&str>,
        description: Option<&str>,
        usage_attrs: &[String],
    ) -> Vec<VarConversion> {
        let mut results = Vec::new();
        if let Some(c) = score_instance_type(name, default_val, description, usage_attrs) {
            results.push(c);
        }
        if let Some(c) = score_region(name, default_val, description, usage_attrs) {
            results.push(c);
        }
        results
    }
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
        signals.push(format!(
            "default '{}' is an AWS RDS/ElastiCache instance class",
            dv
        ));
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
        let kw = [
            "instance type",
            "machine type",
            "server size",
            "ec2 instance",
            "instance class",
            "compute type",
            "node type",
        ];
        if kw.iter().any(|k| dl.contains(k)) {
            score += 2;
            signals.push("description mentions instance/machine type".to_string());
        }
    }

    // Signal 4: variable name keywords
    let nl = name.to_lowercase();
    if nl.contains("instance_type")
        || nl.contains("machine_type")
        || nl.contains("server_size")
        || nl.contains("instance_class")
        || nl.contains("node_type")
    {
        score += 1;
        signals.push("variable name suggests instance type".to_string());
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
            if is_rds {
                Some("1x1xCPU-2GB-25GB")
            } else {
                Some("1x1xCPU-2GB")
            }
        }
        "medium" => {
            if is_rds {
                Some("1x2xCPU-4GB-50GB")
            } else {
                Some("1x2xCPU-4GB")
            }
        }
        "large" => {
            if is_rds {
                Some("2x4xCPU-8GB-100GB")
            } else {
                Some("1x2xCPU-8GB")
            }
        }
        "xlarge" => {
            if is_rds {
                Some("2x6xCPU-16GB-100GB")
            } else {
                Some("1x4xCPU-28GB")
            }
        }
        s if s.ends_with("xlarge") => {
            if is_rds {
                Some("2x8xCPU-32GB-100GB")
            } else {
                Some("1x8xCPU-56GB")
            }
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
                signals
                    .push("referenced as 'region' attribute in a resource or provider".to_string());
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
        let kw = [
            "region",
            "location",
            "datacenter",
            "data center",
            "geography",
            "area",
        ];
        if kw.iter().any(|k| dl.contains(k)) {
            score += 2;
            signals.push("description mentions region/location".to_string());
        }
    }

    // Signal 4: variable name keywords
    let nl = name.to_lowercase();
    if nl.contains("region")
        || nl.contains("location")
        || nl == "zone"
        || nl.contains("datacenter")
        || nl.contains("data_center")
    {
        score += 1;
        signals.push("variable name suggests region/zone".to_string());
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
