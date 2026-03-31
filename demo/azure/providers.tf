terraform {
  required_version = ">= 1.5.0"

  required_providers {
    azurerm = {
      source  = "hashicorp/azurerm"
      version = "~> 4.66"
    }
  }

  backend "azurerm" {
    resource_group_name  = "tfstate-rg"
    storage_account_name = "yourcompanytfstate"
    container_name       = "tfstate"
    key                  = "saas-prod.terraform.tfstate"
  }
}

provider "azurerm" {
  features {}
}