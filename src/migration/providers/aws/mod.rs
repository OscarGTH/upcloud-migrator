pub mod compute;
pub mod database;
pub mod generator_support;
pub mod kubernetes;
pub mod loadbalancer;
pub mod network;
pub mod storage;

use super::{ResourceRole, SourceProvider};

/// AWS source provider implementation.
pub struct AwsSourceProvider;

impl SourceProvider for AwsSourceProvider {
    fn display_name(&self) -> &str {
        "AWS"
    }

    fn resource_type_prefix(&self) -> &str {
        "aws_"
    }

    fn resource_role(&self, resource_type: &str) -> ResourceRole {
        match resource_type {
            "aws_instance" => ResourceRole::ComputeInstance,
            "aws_key_pair" => ResourceRole::KeyPair,
            "aws_db_parameter_group" | "aws_elasticache_parameter_group" => {
                ResourceRole::ParameterGroup
            }
            "aws_db_subnet_group" | "aws_elasticache_subnet_group" => ResourceRole::SubnetGroup,
            "aws_volume_attachment" => ResourceRole::VolumeAttachment,
            "aws_lb_target_group_attachment" | "aws_alb_target_group_attachment" => {
                ResourceRole::LbTargetGroupAttachment
            }
            "aws_lb_listener" | "aws_alb_listener" => ResourceRole::LbListener,
            _ => ResourceRole::Other,
        }
    }

    fn volume_resource_type(&self) -> &str {
        "aws_ebs_volume"
    }

    fn extract_security_refs_from_instance(&self, hcl: &str) -> Vec<String> {
        generator_support::extract_sg_refs_from_instance_hcl(hcl)
    }

    fn extract_subnet_from_instance(&self, hcl: &str) -> Option<String> {
        generator_support::extract_subnet_id_from_instance_hcl(hcl)
    }

    fn extract_parameter_blocks(&self, hcl: &str) -> Vec<(String, String)> {
        database::extract_parameter_blocks(hcl)
    }

    fn is_valid_db_property(&self, name: &str) -> bool {
        database::is_valid_pg_property(name)
    }

    fn extract_subnet_names_from_subnet_group(&self, hcl: &str) -> Vec<String> {
        generator_support::extract_subnet_names_from_subnet_group(hcl)
    }

    fn extract_tg_server_from_attachment(&self, hcl: &str) -> Option<(String, String)> {
        generator_support::extract_tg_server_from_attachment_source_hcl(hcl)
    }

    fn extract_tg_from_listener(&self, hcl: &str) -> Option<String> {
        generator_support::extract_tg_from_listener_source_hcl(hcl)
    }

    fn extract_lb_name_from_listener(&self, hcl: &str) -> Option<String> {
        generator_support::extract_lb_name_from_listener_hcl(hcl)
    }

    fn ssh_key_placeholder(&self, key_name: &str) -> String {
        format!("<TODO: SSH public key for aws_key_pair.{}>", key_name)
    }

    fn parameter_group_todo_text(&self, group_name: &str) -> String {
        format!(
            "    # <TODO: migrate parameters from aws_db_parameter_group.{}>\n",
            group_name
        )
    }

    fn sanitize_source_refs(&self, hcl: String) -> String {
        generator_support::sanitize_aws_refs(hcl)
    }

    fn rewrite_output_refs(&self, hcl: &str) -> String {
        generator_support::rewrite_output_refs(hcl)
    }
}
