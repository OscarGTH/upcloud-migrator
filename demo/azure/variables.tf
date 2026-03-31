variable "project" {
  description = "Project name prefix for all resources"
  type        = string
  default     = "saas"
}

variable "environment" {
  description = "Environment (dev, staging, prod)"
  type        = string
  default     = "prod"
}

variable "location" {
  description = "Azure region"
  type        = string
  default     = "westeurope"
}

variable "db_admin_username" {
  description = "PostgreSQL admin username"
  type        = string
  default     = "pgadmin"
}

variable "db_admin_password" {
  description = "PostgreSQL admin password"
  type        = string
  sensitive   = true
}

variable "tls_certificate_password" {
  description = "Password for the PFX certificate"
  type        = string
  sensitive   = true
  default     = ""
}

variable "web_instance_count" {
  description = "Number of web server instances"
  type        = number
  default     = 2
}

variable "api_instance_count" {
  description = "Number of API server instances"
  type        = number
  default     = 2
}

variable "vm_size" {
  description = "VM size for web/API servers"
  type        = string
  default     = "Standard_B2s"
}

variable "address_space" {
  description = "VNet address space"
  type        = string
  default     = "10.0.0.0/16"
}

locals {
  name_prefix = "${var.project}-${var.environment}"
  common_tags = {
    Project     = var.project
    Environment = var.environment
    ManagedBy   = "terraform"
  }
}