//! Azure resource mapper — routes Azure resource types to their UpCloud equivalents.

use super::{compute, database, kubernetes, loadbalancer, network, storage};
use crate::migration::mapper::ResourceMapper;
use crate::migration::types::{MigrationResult, MigrationStatus};
use crate::terraform::types::TerraformResource;

pub struct AzureResourceMapper;

impl ResourceMapper for AzureResourceMapper {
    fn map(&self, res: &TerraformResource) -> MigrationResult {
        match res.resource_type.as_str() {
            // Compute
            "azurerm_linux_virtual_machine" => compute::map_linux_virtual_machine(res),
            "azurerm_windows_virtual_machine" => compute::map_windows_virtual_machine(res),
            "azurerm_virtual_machine" => compute::map_virtual_machine(res),
            "azurerm_ssh_public_key" => compute::map_ssh_public_key(res),
            "azurerm_availability_set" => compute::map_availability_set(res),
            "azurerm_virtual_machine_scale_set"
            | "azurerm_linux_virtual_machine_scale_set"
            | "azurerm_windows_virtual_machine_scale_set" => {
                compute::map_virtual_machine_scale_set(res)
            }

            // Storage
            "azurerm_managed_disk" => storage::map_managed_disk(res),
            "azurerm_virtual_machine_data_disk_attachment" => {
                storage::map_data_disk_attachment(res)
            }
            "azurerm_storage_account" => storage::map_storage_account(res),
            "azurerm_storage_container" => storage::map_storage_container(res),
            "azurerm_storage_share" => storage::map_storage_share(res),

            // Network
            "azurerm_virtual_network" => network::map_virtual_network(res),
            "azurerm_subnet" => network::map_subnet(res),
            "azurerm_network_security_group" => network::map_network_security_group(res),
            "azurerm_network_security_rule" => network::map_network_security_rule(res),
            "azurerm_public_ip" => network::map_public_ip(res),
            "azurerm_network_interface" => network::map_network_interface(res),

            // Load Balancers
            "azurerm_lb" => loadbalancer::map_lb(res),
            "azurerm_lb_backend_address_pool" => loadbalancer::map_lb_backend_address_pool(res),
            "azurerm_lb_rule" => loadbalancer::map_lb_rule(res),
            "azurerm_lb_backend_address_pool_association" => {
                loadbalancer::map_lb_backend_address_pool_association(res)
            }
            "azurerm_application_gateway" => loadbalancer::map_application_gateway(res),
            "azurerm_lb_probe" => loadbalancer::map_lb_probe(res),

            // Databases
            "azurerm_postgresql_server" => database::map_postgresql_server(res),
            "azurerm_postgresql_flexible_server" => database::map_postgresql_flexible_server(res),
            "azurerm_mysql_server" => database::map_mysql_server(res),
            "azurerm_mysql_flexible_server" => database::map_mysql_flexible_server(res),
            "azurerm_redis_cache" => database::map_redis_cache(res),
            "azurerm_cosmosdb_account" => database::map_cosmosdb_account(res),
            "azurerm_mssql_server" => database::map_mssql_server(res),
            "azurerm_mssql_database" => database::map_mssql_database(res),

            // Kubernetes
            "azurerm_kubernetes_cluster" => kubernetes::map_kubernetes_cluster(res),
            "azurerm_kubernetes_cluster_node_pool" => {
                kubernetes::map_kubernetes_cluster_node_pool(res)
            }

            // Resource Group — no UpCloud equivalent (informational)
            "azurerm_resource_group" => unsupported(
                res,
                "(no resource group equivalent — UpCloud uses flat structure)",
            ),

            // DNS
            "azurerm_dns_zone" | "azurerm_dns_a_record" | "azurerm_dns_cname_record"
            | "azurerm_private_dns_zone" => partial_dns(res),

            // Identity / IAM
            "azurerm_user_assigned_identity" | "azurerm_role_assignment"
            | "azurerm_role_definition" => {
                unsupported(res, "(no IAM equivalent)")
            }

            // Serverless
            "azurerm_function_app" | "azurerm_linux_function_app"
            | "azurerm_windows_function_app" | "azurerm_service_plan" => {
                unsupported(res, "(no serverless/FaaS)")
            }

            // CDN
            "azurerm_cdn_profile" | "azurerm_cdn_endpoint" | "azurerm_frontdoor" => {
                unsupported(res, "(no CDN)")
            }

            // Messaging
            "azurerm_servicebus_namespace" | "azurerm_servicebus_queue"
            | "azurerm_servicebus_topic" | "azurerm_eventgrid_topic"
            | "azurerm_eventhub_namespace" | "azurerm_eventhub" => {
                unsupported(res, "(no messaging service)")
            }

            // Monitoring
            "azurerm_monitor_metric_alert" | "azurerm_monitor_action_group"
            | "azurerm_log_analytics_workspace" | "azurerm_application_insights" => {
                unsupported(res, "(no equivalent monitoring resource)")
            }

            // Key Vault
            "azurerm_key_vault" | "azurerm_key_vault_secret" | "azurerm_key_vault_key" => {
                unsupported(res, "(no key vault equivalent)")
            }

            _ => MigrationResult {
                resource_type: res.resource_type.clone(),
                resource_name: res.name.clone(),
                source_file: res.source_file.display().to_string(),
                status: MigrationStatus::Unknown,
                upcloud_type: "(not mapped)".into(),
                upcloud_hcl: None,
                snippet: None,
                parent_resource: None,
                notes: vec![format!(
                    "Azure resource '{}' has no defined mapping yet",
                    res.resource_type
                )],
                source_hcl: None,
            },
        }
    }
}

fn unsupported(res: &TerraformResource, reason: &str) -> MigrationResult {
    MigrationResult {
        resource_type: res.resource_type.clone(),
        resource_name: res.name.clone(),
        source_file: res.source_file.display().to_string(),
        status: MigrationStatus::Unsupported,
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
        upcloud_type: "(no DNS resource in UpCloud provider)".into(),
        upcloud_hcl: None,
        snippet: None,
        parent_resource: None,
        notes: vec!["DNS must be managed outside Terraform or via a separate DNS provider.".into()],
        source_hcl: None,
    }
}
