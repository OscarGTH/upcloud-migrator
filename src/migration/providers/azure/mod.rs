pub mod compute;
pub mod database;
pub mod generator_support;
pub mod kubernetes;
pub mod loadbalancer;
pub mod mapper;
pub mod network;
pub mod storage;
pub mod var_detector;

use super::{ResourceRole, SourceProvider};

/// Azure source provider implementation.
pub struct AzureSourceProvider;

impl SourceProvider for AzureSourceProvider {
    fn display_name(&self) -> &str {
        "Azure"
    }

    fn resource_type_prefix(&self) -> &str {
        "azurerm_"
    }

    fn resource_role(&self, resource_type: &str) -> ResourceRole {
        match resource_type {
            "azurerm_linux_virtual_machine" | "azurerm_windows_virtual_machine"
            | "azurerm_virtual_machine"
            | "azurerm_linux_virtual_machine_scale_set" => ResourceRole::ComputeInstance,
            "azurerm_ssh_public_key" => ResourceRole::KeyPair,
            "azurerm_managed_disk" => ResourceRole::Other,
            "azurerm_virtual_machine_data_disk_attachment" => ResourceRole::VolumeAttachment,
            "azurerm_lb_rule" => ResourceRole::LbListener,
            "azurerm_lb_backend_address_pool_association" => ResourceRole::LbTargetGroupAttachment,
            _ => ResourceRole::Other,
        }
    }

    fn volume_resource_type(&self) -> &str {
        "azurerm_managed_disk"
    }

    fn extract_security_refs_from_instance(&self, hcl: &str) -> Vec<String> {
        generator_support::extract_nsg_refs_from_instance_hcl(hcl)
    }

    fn extract_subnet_from_instance(&self, hcl: &str) -> Option<String> {
        generator_support::extract_subnet_from_instance_hcl(hcl)
    }

    fn nic_resource_type(&self) -> Option<&str> {
        Some("azurerm_network_interface")
    }

    fn extract_nic_refs_from_instance(&self, hcl: &str) -> Vec<String> {
        generator_support::extract_nic_refs_from_instance_hcl(hcl)
    }

    fn extract_subnet_from_nic(&self, hcl: &str) -> Option<String> {
        generator_support::extract_subnet_from_instance_hcl(hcl)
    }

    fn subnet_nsg_association_type(&self) -> Option<&str> {
        Some("azurerm_subnet_network_security_group_association")
    }

    fn extract_nsg_from_subnet_association(&self, hcl: &str) -> Option<(String, String)> {
        generator_support::extract_nsg_subnet_association_hcl(hcl)
    }

    fn extract_parameter_blocks(&self, _hcl: &str) -> Vec<(String, String)> {
        // Azure databases don't use parameter groups like AWS
        Vec::new()
    }

    fn is_valid_db_property(&self, name: &str) -> bool {
        database::is_valid_pg_property(name)
    }

    fn extract_subnet_names_from_subnet_group(&self, _hcl: &str) -> Vec<String> {
        // Azure doesn't have subnet groups
        Vec::new()
    }

    fn extract_tg_server_from_attachment(&self, hcl: &str) -> Option<(String, String)> {
        generator_support::extract_backend_pool_server_from_association_hcl(hcl)
    }

    fn extract_tg_from_listener(&self, hcl: &str) -> Option<String> {
        generator_support::extract_backend_pool_from_lb_rule_hcl(hcl)
    }

    fn extract_lb_name_from_listener(&self, hcl: &str) -> Option<String> {
        generator_support::extract_lb_name_from_rule_hcl(hcl)
    }

    fn lb_probe_resource_type(&self) -> Option<&str> {
        Some("azurerm_lb_probe")
    }

    fn extract_probe_from_lb_rule(&self, hcl: &str) -> Option<(String, String)> {
        generator_support::extract_probe_and_backend_from_lb_rule_hcl(hcl)
    }

    fn extract_probe_health_check_props(&self, hcl: &str) -> String {
        generator_support::extract_probe_health_check_props_hcl(hcl)
    }

    fn ssh_key_placeholder(&self, key_name: &str) -> String {
        format!("<TODO: SSH public key for azurerm_ssh_public_key.{}>", key_name)
    }

    fn parameter_group_todo_text(&self, group_name: &str) -> String {
        format!(
            "    # <TODO: migrate parameters from Azure configuration {}>\n",
            group_name
        )
    }

    fn sanitize_source_refs(&self, hcl: String) -> String {
        generator_support::sanitize_azure_refs(hcl)
    }

    fn rewrite_output_refs(&self, hcl: &str) -> String {
        generator_support::rewrite_output_refs(hcl)
    }

    fn var_detector(&self) -> Box<dyn crate::migration::var_detector::VarDetector> {
        Box::new(var_detector::AzureVarDetector)
    }

    fn mapper(&self) -> Box<dyn crate::migration::mapper::ResourceMapper> {
        Box::new(mapper::AzureResourceMapper)
    }
}
