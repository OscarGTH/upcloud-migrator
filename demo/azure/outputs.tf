output "resource_group_name" {
  value = azurerm_resource_group.main.name
}

output "appgw_public_ip" {
  description = "Public IP address of the Application Gateway"
  value       = azurerm_public_ip.appgw.ip_address
}

output "postgresql_fqdn" {
  description = "PostgreSQL server FQDN (private)"
  value       = azurerm_postgresql_flexible_server.main.fqdn
  sensitive   = true
}

output "postgresql_database" {
  value = azurerm_postgresql_flexible_server_database.app.name
}

output "redis_hostname" {
  description = "Redis cache hostname (private)"
  value       = azurerm_redis_cache.main.hostname
  sensitive   = true
}

output "redis_primary_key" {
  description = "Redis primary access key"
  value       = azurerm_redis_cache.main.primary_access_key
  sensitive   = true
}

output "blob_storage_account" {
  value = azurerm_storage_account.blobs.name
}

output "shared_filesystem_endpoint" {
  description = "NFS mount endpoint for shared filesystem"
  value       = "${azurerm_storage_account.files.name}.file.core.windows.net:/${azurerm_storage_account.files.name}/${azurerm_storage_share.shared.name}"
}