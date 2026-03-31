# ──────────────────────────────────────────────
# Blob Storage Account (S3-equivalent buckets)
# ──────────────────────────────────────────────
resource "azurerm_storage_account" "blobs" {
  name                            = "${replace(local.name_prefix, "-", "")}blobs"
  resource_group_name             = azurerm_resource_group.main.name
  location                        = azurerm_resource_group.main.location
  account_tier                    = "Standard"
  account_replication_type        = "GRS"
  account_kind                    = "StorageV2"
  min_tls_version                 = "TLS1_2"
  allow_nested_items_to_be_public = false
  tags                            = local.common_tags

  blob_properties {
    versioning_enabled = true

    delete_retention_policy {
      days = 30
    }

    container_delete_retention_policy {
      days = 30
    }
  }

  network_rules {
    default_action             = "Deny"
    virtual_network_subnet_ids = [azurerm_subnet.web.id, azurerm_subnet.api.id]
    bypass                     = ["AzureServices"]
  }
}

resource "azurerm_storage_container" "uploads" {
  name                  = "uploads"
  storage_account_id    = azurerm_storage_account.blobs.id
  container_access_type = "private"
}

resource "azurerm_storage_container" "assets" {
  name                  = "assets"
  storage_account_id    = azurerm_storage_account.blobs.id
  container_access_type = "private"
}

resource "azurerm_storage_container" "backups" {
  name                  = "backups"
  storage_account_id    = azurerm_storage_account.blobs.id
  container_access_type = "private"
}

# Lifecycle management (auto-archive old backups)
resource "azurerm_storage_management_policy" "lifecycle" {
  storage_account_id = azurerm_storage_account.blobs.id

  rule {
    name    = "archive-old-backups"
    enabled = true

    filters {
      prefix_match = ["backups/"]
      blob_types   = ["blockBlob"]
    }

    actions {
      base_blob {
        tier_to_cool_after_days_since_modification_greater_than    = 30
        tier_to_archive_after_days_since_modification_greater_than = 90
        delete_after_days_since_modification_greater_than          = 365
      }
    }
  }
}

# ──────────────────────────────────────────────
# Azure Files — Shared NFS Filesystem (EFS equivalent)
# ──────────────────────────────────────────────
resource "azurerm_storage_account" "files" {
  name                     = "${replace(local.name_prefix, "-", "")}files"
  resource_group_name      = azurerm_resource_group.main.name
  location                 = azurerm_resource_group.main.location
  account_tier             = "Premium"
  account_replication_type = "ZRS"
  account_kind             = "FileStorage"
  min_tls_version          = "TLS1_2"
  tags                     = local.common_tags

  network_rules {
    default_action             = "Deny"
    virtual_network_subnet_ids = [azurerm_subnet.web.id, azurerm_subnet.api.id, azurerm_subnet.storage.id]
    bypass                     = ["AzureServices"]
  }
}

resource "azurerm_storage_share" "shared" {
  name                 = "shared-data"
  storage_account_id = azurerm_storage_account.files.id
  quota                = 100   # GiB
  enabled_protocol     = "NFS"
}