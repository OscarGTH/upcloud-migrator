use crate::migration::types::{MigrationResult, MigrationStatus};
use crate::migration::providers::aws;
use crate::terraform::types::TerraformResource;

pub fn map_resource(res: &TerraformResource) -> MigrationResult {
    let rt = res.resource_type.as_str();

    let mut result = if rt.starts_with("aws_") {
        map_aws(res)
    } else {
        MigrationResult {
            resource_type: res.resource_type.clone(),
            resource_name: res.name.clone(),
            source_file: res.source_file.display().to_string(),
            status: MigrationStatus::Unknown,
            score: 0,
            upcloud_type: "(unknown provider)".into(),
            upcloud_hcl: None,
            snippet: None,
            parent_resource: None,
            notes: vec![format!("Provider not recognized for resource type '{}'", rt)],
            source_hcl: None,
        }
    };

    result.source_hcl = Some(res.raw_hcl.clone());
    result
}

fn map_aws(res: &TerraformResource) -> MigrationResult {
    match res.resource_type.as_str() {
        // Compute
        "aws_instance" => aws::compute::map_instance(res),
        "aws_key_pair" => aws::compute::map_key_pair(res),
        "aws_autoscaling_group" => aws::compute::map_autoscaling_group(res),

        // Storage
        "aws_ebs_volume" => aws::storage::map_ebs_volume(res),
        "aws_volume_attachment" => aws::storage::map_volume_attachment(res),
        "aws_s3_bucket" => aws::storage::map_s3_bucket(res),
        "aws_s3_bucket_policy" | "aws_s3_bucket_acl" => aws::storage::map_s3_bucket_policy(res),
        "aws_efs_file_system" => aws::storage::map_efs_file_system(res),

        // Network
        "aws_vpc" => aws::network::map_vpc(res),
        "aws_subnet" => aws::network::map_subnet(res),
        "aws_security_group" => aws::network::map_security_group(res),
        "aws_internet_gateway" => aws::network::map_internet_gateway(res),
        "aws_nat_gateway" => aws::network::map_nat_gateway(res),
        "aws_route_table" | "aws_route_table_association" => aws::network::map_route_table(res),
        "aws_eip" => aws::network::map_eip(res),
        "aws_eip_association" => aws::network::map_eip_association(res),

        // Load Balancers
        "aws_lb" | "aws_alb" => aws::loadbalancer::map_lb(res),
        "aws_lb_target_group" | "aws_alb_target_group" => aws::loadbalancer::map_lb_target_group(res),
        "aws_lb_listener" | "aws_alb_listener" => aws::loadbalancer::map_lb_listener(res),
        "aws_lb_target_group_attachment" | "aws_alb_target_group_attachment" => {
            aws::loadbalancer::map_lb_target_group_attachment(res)
        }
        "aws_acm_certificate" => aws::loadbalancer::map_acm_certificate(res),

        // Databases
        "aws_db_instance" | "aws_rds_instance" => aws::database::map_rds_instance(res),
        "aws_rds_cluster" => aws::database::map_rds_cluster(res),
        "aws_db_parameter_group" => aws::database::map_db_parameter_group(res),
        "aws_db_subnet_group" => aws::database::map_db_subnet_group(res),
        "aws_elasticache_cluster" | "aws_elasticache_replication_group" => {
            aws::database::map_elasticache_cluster(res)
        }
        "aws_elasticache_subnet_group" => aws::database::map_elasticache_subnet_group(res),
        "aws_elasticache_parameter_group" => aws::database::map_elasticache_parameter_group(res),

        // Kubernetes (not supported in this MVP)
        "aws_eks_cluster" | "aws_eks_node_group" | "aws_eks_fargate_profile"
        | "aws_eks_addon" => unsupported(res, "(EKS not supported — use upcloud_kubernetes_cluster manually)"),

        // Unsupported
        "aws_iam_role"
        | "aws_iam_policy"
        | "aws_iam_role_policy"
        | "aws_iam_role_policy_attachment"
        | "aws_iam_user"
        | "aws_iam_group" => unsupported(res, "(no IAM equivalent)"),

        "aws_lambda_function" | "aws_lambda_permission" | "aws_lambda_event_source_mapping" => {
            unsupported(res, "(no serverless/FaaS)")
        }

        "aws_cloudfront_distribution" | "aws_cloudfront_origin_access_identity" => {
            unsupported(res, "(no CDN)")
        }

        "aws_sqs_queue" | "aws_sqs_queue_policy" => unsupported(res, "(no queue service)"),

        "aws_sns_topic" | "aws_sns_topic_subscription" => unsupported(res, "(no pub/sub)"),

        "aws_api_gateway_rest_api"
        | "aws_api_gateway_resource"
        | "aws_api_gateway_method"
        | "aws_apigatewayv2_api" => unsupported(res, "(no API gateway)"),

        "aws_cognito_user_pool" | "aws_cognito_user_pool_client" => {
            unsupported(res, "(no auth/identity service)")
        }

        "aws_route53_record" | "aws_route53_zone" => partial_dns(res),

        "aws_cloudwatch_metric_alarm" | "aws_cloudwatch_log_group" => {
            unsupported(res, "(no equivalent monitoring resource)")
        }

        _ => MigrationResult {
            resource_type: res.resource_type.clone(),
            resource_name: res.name.clone(),
            source_file: res.source_file.display().to_string(),
            status: MigrationStatus::Unknown,
            score: 0,
            upcloud_type: "(not mapped)".into(),
            upcloud_hcl: None,
            snippet: None,
            parent_resource: None,
            notes: vec![format!("AWS resource '{}' has no defined mapping yet", res.resource_type)],
            source_hcl: None,
        },
    }
}

fn unsupported(res: &TerraformResource, reason: &str) -> MigrationResult {
    MigrationResult {
        resource_type: res.resource_type.clone(),
        resource_name: res.name.clone(),
        source_file: res.source_file.display().to_string(),
        status: MigrationStatus::Unsupported,
        score: 0,
        upcloud_type: reason.into(),
        upcloud_hcl: None,
        snippet: None,
        parent_resource: None,
        notes: vec![format!(
            "'{}' has no UpCloud equivalent. Manual migration required.",
            res.resource_type
        )],
            source_hcl: None,
    }
}

fn partial_dns(res: &TerraformResource) -> MigrationResult {
    MigrationResult {
        resource_type: res.resource_type.clone(),
        resource_name: res.name.clone(),
        source_file: res.source_file.display().to_string(),
        status: MigrationStatus::Partial,
        score: 30,
        upcloud_type: "(no DNS resource in UpCloud provider)".into(),
        upcloud_hcl: None,
        snippet: None,
        parent_resource: None,
        notes: vec!["DNS must be managed outside Terraform or via a separate DNS provider".into()],
            source_hcl: None,
    }
}
