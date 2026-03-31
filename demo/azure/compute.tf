# ──────────────────────────────────────────────
# Web Server Scale Set
# ──────────────────────────────────────────────
resource "azurerm_linux_virtual_machine_scale_set" "web" {
  name                            = "${local.name_prefix}-web-vmss"
  resource_group_name             = azurerm_resource_group.main.name
  location                        = azurerm_resource_group.main.location
  sku                             = var.vm_size
  instances                       = var.web_instance_count
  admin_username                  = "azureadmin"
  disable_password_authentication = true
  upgrade_mode                    = "Rolling"
  health_probe_id                 = azurerm_application_gateway.main.probe[0].id
  tags                            = local.common_tags

  admin_ssh_key {
    username   = "azureadmin"
    public_key = file("~/.ssh/id_rsa.pub")
  }

  source_image_reference {
    publisher = "Canonical"
    offer     = "0001-com-ubuntu-server-jammy"
    sku       = "22_04-lts-gen2"
    version   = "latest"
  }

  os_disk {
    caching              = "ReadWrite"
    storage_account_type = "Premium_LRS"
    disk_size_gb         = 30
  }

  network_interface {
    name    = "web-nic"
    primary = true

    ip_configuration {
      name                                         = "web-ip-config"
      primary                                      = true
      subnet_id                                    = azurerm_subnet.web.id
      application_gateway_backend_address_pool_ids = [for pool in azurerm_application_gateway.main.backend_address_pool : pool.id if pool.name == "web-backend-pool"]
    }
  }

  rolling_upgrade_policy {
    max_batch_instance_percent              = 50
    max_unhealthy_instance_percent          = 50
    max_unhealthy_upgraded_instance_percent = 50
    pause_time_between_batches              = "PT5S"
  }

  custom_data = base64encode(<<-CLOUDINIT
    #!/bin/bash
    apt-get update -y
    apt-get install -y nginx nfs-common
    # Mount Azure Files share
    mkdir -p /mnt/shared
    mount -t nfs ${azurerm_storage_account.files.name}.file.core.windows.net:/${azurerm_storage_account.files.name}/${azurerm_storage_share.shared.name} /mnt/shared -o vers=4,minorversion=1,sec=sys
    systemctl enable --now nginx
  CLOUDINIT
  )
}

# ──────────────────────────────────────────────
# API Server Scale Set
# ──────────────────────────────────────────────
resource "azurerm_linux_virtual_machine_scale_set" "api" {
  name                            = "${local.name_prefix}-api-vmss"
  resource_group_name             = azurerm_resource_group.main.name
  location                        = azurerm_resource_group.main.location
  sku                             = var.vm_size
  instances                       = var.api_instance_count
  admin_username                  = "azureadmin"
  disable_password_authentication = true
  upgrade_mode                    = "Rolling"
  tags                            = local.common_tags

  admin_ssh_key {
    username   = "azureadmin"
    public_key = file("~/.ssh/id_rsa.pub")
  }

  source_image_reference {
    publisher = "Canonical"
    offer     = "0001-com-ubuntu-server-jammy"
    sku       = "22_04-lts-gen2"
    version   = "latest"
  }

  os_disk {
    caching              = "ReadWrite"
    storage_account_type = "Premium_LRS"
    disk_size_gb         = 30
  }

  network_interface {
    name    = "api-nic"
    primary = true

    ip_configuration {
      name                                         = "api-ip-config"
      primary                                      = true
      subnet_id                                    = azurerm_subnet.api.id
      application_gateway_backend_address_pool_ids = [for pool in azurerm_application_gateway.main.backend_address_pool : pool.id if pool.name == "api-backend-pool"]
    }
  }

  rolling_upgrade_policy {
    max_batch_instance_percent              = 50
    max_unhealthy_instance_percent          = 50
    max_unhealthy_upgraded_instance_percent = 50
    pause_time_between_batches              = "PT5S"
  }

  custom_data = base64encode(<<-CLOUDINIT
    #!/bin/bash
    apt-get update -y
    apt-get install -y nfs-common
    mkdir -p /mnt/shared
    mount -t nfs ${azurerm_storage_account.files.name}.file.core.windows.net:/${azurerm_storage_account.files.name}/${azurerm_storage_share.shared.name} /mnt/shared -o vers=4,minorversion=1,sec=sys
  CLOUDINIT
  )
}