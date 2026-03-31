# ──────────────────────────────────────────────
# Public IP for Application Gateway
# ──────────────────────────────────────────────
resource "azurerm_public_ip" "appgw" {
  name                = "${local.name_prefix}-appgw-pip"
  resource_group_name = azurerm_resource_group.main.name
  location            = azurerm_resource_group.main.location
  allocation_method   = "Static"
  sku                 = "Standard"
  tags                = local.common_tags
}

# ──────────────────────────────────────────────
# Application Gateway (Layer 7 Load Balancer)
# ──────────────────────────────────────────────
resource "azurerm_application_gateway" "main" {
  name                = "${local.name_prefix}-appgw"
  resource_group_name = azurerm_resource_group.main.name
  location            = azurerm_resource_group.main.location
  tags                = local.common_tags

  sku {
    name     = "Standard_v2"
    tier     = "Standard_v2"
    capacity = 2
  }

  gateway_ip_configuration {
    name      = "gateway-ip-config"
    subnet_id = azurerm_subnet.appgw.id
  }

  # ── Frontend ──
  frontend_ip_configuration {
    name                 = "frontend-ip"
    public_ip_address_id = azurerm_public_ip.appgw.id
  }

  frontend_port {
    name = "https-port"
    port = 443
  }

  frontend_port {
    name = "http-port"
    port = 80
  }

  # ── TLS Certificate ──
  ssl_certificate {
    name     = "appgw-tls-cert"
    data     = filebase64("certs/appgw.pfx")
    password = var.tls_certificate_password
  }

  ssl_policy {
    policy_type = "Predefined"
    policy_name = "AppGwSslPolicy20220101"
  }

  # ── Backend Pools ──
  backend_address_pool {
    name = "web-backend-pool"
  }

  backend_address_pool {
    name = "api-backend-pool"
  }

  # ── Backend HTTP Settings ──
  backend_http_settings {
    name                  = "web-http-settings"
    cookie_based_affinity = "Disabled"
    port                  = 80
    protocol              = "Http"
    request_timeout       = 30
    probe_name            = "web-health-probe"
  }

  backend_http_settings {
    name                  = "api-http-settings"
    cookie_based_affinity = "Disabled"
    port                  = 8080
    protocol              = "Http"
    request_timeout       = 30
    probe_name            = "api-health-probe"
  }

  # ── Health Probes ──
  probe {
    name                = "web-health-probe"
    protocol            = "Http"
    path                = "/health"
    host                = "127.0.0.1"
    interval            = 15
    timeout             = 10
    unhealthy_threshold = 3
  }

  probe {
    name                = "api-health-probe"
    protocol            = "Http"
    path                = "/api/health"
    host                = "127.0.0.1"
    interval            = 15
    timeout             = 10
    unhealthy_threshold = 3
  }

  # ── HTTPS Listener (web) ──
  http_listener {
    name                           = "web-https-listener"
    frontend_ip_configuration_name = "frontend-ip"
    frontend_port_name             = "https-port"
    protocol                       = "Https"
    ssl_certificate_name           = "appgw-tls-cert"
    host_name                      = "www.example.com"
  }

  # ── HTTPS Listener (api) ──
  http_listener {
    name                           = "api-https-listener"
    frontend_ip_configuration_name = "frontend-ip"
    frontend_port_name             = "https-port"
    protocol                       = "Https"
    ssl_certificate_name           = "appgw-tls-cert"
    host_name                      = "api.example.com"
  }

  # ── HTTP → HTTPS Redirect Listener ──
  http_listener {
    name                           = "http-redirect-listener"
    frontend_ip_configuration_name = "frontend-ip"
    frontend_port_name             = "http-port"
    protocol                       = "Http"
  }

  # ── Routing Rules ──
  request_routing_rule {
    name                       = "web-routing-rule"
    priority                   = 100
    rule_type                  = "Basic"
    http_listener_name         = "web-https-listener"
    backend_address_pool_name  = "web-backend-pool"
    backend_http_settings_name = "web-http-settings"
  }

  request_routing_rule {
    name                       = "api-routing-rule"
    priority                   = 200
    rule_type                  = "Basic"
    http_listener_name         = "api-https-listener"
    backend_address_pool_name  = "api-backend-pool"
    backend_http_settings_name = "api-http-settings"
  }

  request_routing_rule {
    name                        = "http-to-https-redirect"
    priority                    = 300
    rule_type                   = "Basic"
    http_listener_name          = "http-redirect-listener"
    redirect_configuration_name = "http-to-https"
  }

  redirect_configuration {
    name                 = "http-to-https"
    redirect_type        = "Permanent"
    target_listener_name = "web-https-listener"
    include_path         = true
    include_query_string = true
  }
}