use super::super::shared;
use crate::migration::types::{MigrationResult, MigrationStatus};
use crate::terraform::types::TerraformResource;

/// Re-export from shared for use in mod.rs trait impl.
pub(crate) use shared::is_valid_pg_property;

fn map_db_sku(sku_name: &str) -> &'static str {
    match sku_name {
        s if s.contains("B_") || s.contains("Burstable") => "1x1xCPU-2GB-25GB",
        s if s.contains("GP_") || s.contains("GeneralPurpose") => "1x2xCPU-4GB-50GB",
        s if s.contains("MO_") || s.contains("MemoryOptimized") => "2x4xCPU-8GB-100GB",
        _ => "1x2xCPU-4GB-50GB",
    }
}

pub fn map_postgresql_server(res: &TerraformResource) -> MigrationResult {
    let sku = res
        .attributes
        .get("sku_name")
        .map(|s| s.trim_matches('"'))
        .unwrap_or("GP_Standard_D2s_v3");
    let plan = map_db_sku(sku);
    let version = res
        .attributes
        .get("version")
        .map(|v| v.trim_matches('"'))
        .unwrap_or("16");

    let hcl = shared::upcloud_managed_database_hcl(
        "upcloud_managed_database_postgresql",
        &res.name,
        &format!("{}-db", res.name),
        plan,
        "<TODO: upcloud_network UUID>",
        &format!("    version       = \"{}\"", version),
    );

    MigrationResult {
        resource_type: res.resource_type.clone(),
        resource_name: res.name.clone(),
        source_file: res.source_file.display().to_string(),
        status: MigrationStatus::Compatible,
        upcloud_type: "upcloud_managed_database_postgresql".into(),
        upcloud_hcl: Some(hcl),
        snippet: None,
        parent_resource: None,
        notes: vec![
            format!(
                "Azure PostgreSQL (SKU: {}) → UpCloud Managed PostgreSQL (plan: {})",
                sku, plan
            ),
            "Migrate DB parameters, connection strings, and firewall rules manually.".into(),
            "Set network.uuid to the upcloud_network resource in the same zone.".into(),
        ],
        source_hcl: None,
    }
}

pub fn map_postgresql_flexible_server(res: &TerraformResource) -> MigrationResult {
    let sku = res
        .attributes
        .get("sku_name")
        .map(|s| s.trim_matches('"'))
        .unwrap_or("GP_Standard_D2s_v3");
    let plan = map_db_sku(sku);
    let version = res
        .attributes
        .get("version")
        .map(|v| v.trim_matches('"'))
        .unwrap_or("16");

    let hcl = shared::upcloud_managed_database_hcl(
        "upcloud_managed_database_postgresql",
        &res.name,
        &format!("{}-db", res.name),
        plan,
        "<TODO: upcloud_network UUID>",
        &format!("    version       = \"{}\"", version),
    );

    MigrationResult {
        resource_type: res.resource_type.clone(),
        resource_name: res.name.clone(),
        source_file: res.source_file.display().to_string(),
        status: MigrationStatus::Compatible,
        upcloud_type: "upcloud_managed_database_postgresql".into(),
        upcloud_hcl: Some(hcl),
        snippet: None,
        parent_resource: None,
        notes: vec![
            format!(
                "Azure PostgreSQL Flexible (SKU: {}) → UpCloud Managed PostgreSQL (plan: {})",
                sku, plan
            ),
            "Migrate high availability, read replicas, and connection strings manually.".into(),
        ],
        source_hcl: None,
    }
}

pub fn map_mysql_server(res: &TerraformResource) -> MigrationResult {
    let sku = res
        .attributes
        .get("sku_name")
        .map(|s| s.trim_matches('"'))
        .unwrap_or("GP_Standard_D2s_v3");
    let plan = map_db_sku(sku);
    let version = res
        .attributes
        .get("version")
        .map(|v| v.trim_matches('"'))
        .unwrap_or("8.0");

    let hcl = shared::upcloud_managed_database_hcl(
        "upcloud_managed_database_mysql",
        &res.name,
        &format!("{}-db", res.name),
        plan,
        "<TODO: upcloud_network UUID>",
        &format!("    version       = \"{}\"", version),
    );

    MigrationResult {
        resource_type: res.resource_type.clone(),
        resource_name: res.name.clone(),
        source_file: res.source_file.display().to_string(),
        status: MigrationStatus::Compatible,
        upcloud_type: "upcloud_managed_database_mysql".into(),
        upcloud_hcl: Some(hcl),
        snippet: None,
        parent_resource: None,
        notes: vec![
            format!(
                "Azure MySQL (SKU: {}) → UpCloud Managed MySQL (plan: {})",
                sku, plan
            ),
            "Migrate DB parameters and connection strings manually.".into(),
        ],
        source_hcl: None,
    }
}

pub fn map_mysql_flexible_server(res: &TerraformResource) -> MigrationResult {
    let sku = res
        .attributes
        .get("sku_name")
        .map(|s| s.trim_matches('"'))
        .unwrap_or("GP_Standard_D2s_v3");
    let plan = map_db_sku(sku);

    let hcl = shared::upcloud_managed_database_hcl(
        "upcloud_managed_database_mysql",
        &res.name,
        &format!("{}-db", res.name),
        plan,
        "<TODO: upcloud_network UUID>",
        "",
    );

    MigrationResult {
        resource_type: res.resource_type.clone(),
        resource_name: res.name.clone(),
        source_file: res.source_file.display().to_string(),
        status: MigrationStatus::Compatible,
        upcloud_type: "upcloud_managed_database_mysql".into(),
        upcloud_hcl: Some(hcl),
        snippet: None,
        parent_resource: None,
        notes: vec![
            format!(
                "Azure MySQL Flexible (SKU: {}) → UpCloud Managed MySQL (plan: {})",
                sku, plan
            ),
            "Migrate high availability and connection strings manually.".into(),
        ],
        source_hcl: None,
    }
}

pub fn map_redis_cache(res: &TerraformResource) -> MigrationResult {
    let sku = res
        .attributes
        .get("sku_name")
        .map(|s| s.trim_matches('"'))
        .unwrap_or("Basic");
    let capacity = res
        .attributes
        .get("capacity")
        .map(|c| c.trim_matches('"'))
        .unwrap_or("1");

    let plan = match (sku, capacity) {
        ("Basic", "0") | ("Basic", "1") => "1x1xCPU-2GB",
        ("Basic", _) => "1x2xCPU-4GB",
        ("Standard", "0") | ("Standard", "1") => "1x1xCPU-2GB",
        ("Standard", "2") | ("Standard", "3") => "1x2xCPU-4GB",
        ("Standard", _) => "1x2xCPU-8GB",
        ("Premium", _) => "1x4xCPU-28GB",
        _ => "1x1xCPU-2GB",
    };

    let hcl = shared::upcloud_managed_database_hcl(
        "upcloud_managed_database_valkey",
        &res.name,
        &format!("{}-cache", res.name),
        plan,
        "<TODO: upcloud_network UUID>",
        "",
    );

    MigrationResult {
        resource_type: res.resource_type.clone(),
        resource_name: res.name.clone(),
        source_file: res.source_file.display().to_string(),
        status: MigrationStatus::Compatible,
        upcloud_type: "upcloud_managed_database_valkey".into(),
        upcloud_hcl: Some(hcl),
        snippet: None,
        parent_resource: None,
        notes: vec![
            format!(
                "Azure Redis Cache ({}/{}) → UpCloud Managed Valkey (plan: {})",
                sku, capacity, plan
            ),
            "Valkey is Redis-compatible. Migrate connection strings and auth tokens.".into(),
        ],
        source_hcl: None,
    }
}

pub fn map_postgresql_flexible_server_database(res: &TerraformResource) -> MigrationResult {
    let db_name = res
        .attributes
        .get("name")
        .map(|n| n.trim_matches('"'))
        .unwrap_or(&res.name);
    let server_ref = res
        .attributes
        .get("server_id")
        .and_then(|v| {
            let v = v.trim_matches('"');
            // azurerm_postgresql_flexible_server.<name>.id
            if v.starts_with("azurerm_postgresql_flexible_server.") {
                v.split('.')
                    .nth(1)
                    .map(|n| format!("upcloud_managed_database_postgresql.{}", n))
            } else {
                None
            }
        })
        .unwrap_or_else(|| "upcloud_managed_database_postgresql.<TODO>".to_string());

    MigrationResult {
        resource_type: res.resource_type.clone(),
        resource_name: res.name.clone(),
        source_file: res.source_file.display().to_string(),
        status: MigrationStatus::Partial,
        upcloud_type: "(no separate sub-database resource)".into(),
        upcloud_hcl: None,
        snippet: None,
        parent_resource: None,
        notes: vec![
            format!(
                "UpCloud Managed PostgreSQL has no dedicated Terraform resource for sub-databases."
            ),
            format!(
                "Create the '{}' database manually after provisioning {} or via an init script.",
                db_name, server_ref
            ),
        ],
        source_hcl: None,
    }
}

pub fn map_postgresql_flexible_server_configuration(res: &TerraformResource) -> MigrationResult {
    let config_name = res
        .attributes
        .get("name")
        .map(|n| n.trim_matches('"'))
        .unwrap_or(&res.name);
    let config_value = res
        .attributes
        .get("value")
        .map(|v| v.trim_matches('"'))
        .unwrap_or("");
    let server_ref = res
        .attributes
        .get("server_id")
        .and_then(|v| {
            let v = v.trim_matches('"');
            if v.starts_with("azurerm_postgresql_flexible_server.") {
                v.split('.')
                    .nth(1)
                    .map(|n| format!("upcloud_managed_database_postgresql.{}", n))
            } else {
                None
            }
        })
        .unwrap_or_else(|| "upcloud_managed_database_postgresql.<TODO>".to_string());

    MigrationResult {
        resource_type: res.resource_type.clone(),
        resource_name: res.name.clone(),
        source_file: res.source_file.display().to_string(),
        status: MigrationStatus::Partial,
        upcloud_type: "properties block in upcloud_managed_database_postgresql".into(),
        upcloud_hcl: None,
        snippet: Some(format!(
            r#"  # Add to {server_ref}:
  properties {{
    # {config_name} = "{config_value}"
  }}
"#,
            server_ref = server_ref,
            config_name = config_name,
            config_value = config_value,
        )),
        parent_resource: None,
        notes: vec![
            format!(
                "PostgreSQL configuration '{}' = '{}' should be set in the properties block of {}.",
                config_name, config_value, server_ref
            ),
            "UpCloud Managed Database properties replace Azure server configurations.".into(),
        ],
        source_hcl: None,
    }
}

pub fn map_cosmosdb_account(res: &TerraformResource) -> MigrationResult {
    let kind = res
        .attributes
        .get("kind")
        .map(|k| k.trim_matches('"'))
        .unwrap_or("GlobalDocumentDB");

    MigrationResult {
        resource_type: res.resource_type.clone(),
        resource_name: res.name.clone(),
        source_file: res.source_file.display().to_string(),
        status: MigrationStatus::Unsupported,
        upcloud_type: "(no CosmosDB equivalent)".into(),
        upcloud_hcl: None,
        snippet: None,
        parent_resource: None,
        notes: vec![
            format!(
                "Azure CosmosDB ({}) has no direct UpCloud equivalent.",
                kind
            ),
            "Consider using UpCloud Managed PostgreSQL or MySQL for relational workloads.".into(),
        ],
        source_hcl: None,
    }
}

pub fn map_mssql_server(res: &TerraformResource) -> MigrationResult {
    MigrationResult {
        resource_type: res.resource_type.clone(),
        resource_name: res.name.clone(),
        source_file: res.source_file.display().to_string(),
        status: MigrationStatus::Unsupported,
        upcloud_type: "(no MS SQL equivalent)".into(),
        upcloud_hcl: None,
        snippet: None,
        parent_resource: None,
        notes: vec![
            "Azure SQL Database (MS SQL) has no direct UpCloud equivalent.".into(),
            "Consider migrating to UpCloud Managed PostgreSQL or running SQL Server on an UpCloud server.".into(),
        ],
        source_hcl: None,
    }
}

pub fn map_mssql_database(res: &TerraformResource) -> MigrationResult {
    MigrationResult {
        resource_type: res.resource_type.clone(),
        resource_name: res.name.clone(),
        source_file: res.source_file.display().to_string(),
        status: MigrationStatus::Unsupported,
        upcloud_type: "(no MS SQL equivalent)".into(),
        upcloud_hcl: None,
        snippet: None,
        parent_resource: None,
        notes: vec![
            "Azure SQL Database has no direct UpCloud equivalent.".into(),
            "Consider migrating to UpCloud Managed PostgreSQL or MySQL.".into(),
        ],
        source_hcl: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn make_res(resource_type: &str, name: &str, attrs: &[(&str, &str)]) -> TerraformResource {
        TerraformResource {
            resource_type: resource_type.to_string(),
            name: name.to_string(),
            attributes: attrs
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            source_file: PathBuf::from("test.tf"),
            raw_hcl: String::new(),
        }
    }

    // ── map_db_sku plan tiers (tested via map_postgresql_server) ─────────────

    #[test]
    fn burstable_sku_maps_to_small_plan() {
        let res = make_res(
            "azurerm_postgresql_server",
            "db",
            &[("sku_name", "B_Gen5_1")],
        );
        let hcl = map_postgresql_server(&res).upcloud_hcl.unwrap();
        assert!(hcl.contains("1x1xCPU-2GB-25GB"), "{hcl}");
    }

    #[test]
    fn general_purpose_sku_maps_to_medium_plan() {
        let res = make_res(
            "azurerm_postgresql_server",
            "db",
            &[("sku_name", "GP_Gen5_4")],
        );
        let hcl = map_postgresql_server(&res).upcloud_hcl.unwrap();
        assert!(hcl.contains("1x2xCPU-4GB-50GB"), "{hcl}");
    }

    #[test]
    fn memory_optimized_sku_maps_to_large_plan() {
        let res = make_res(
            "azurerm_postgresql_server",
            "db",
            &[("sku_name", "MO_Gen5_8")],
        );
        let hcl = map_postgresql_server(&res).upcloud_hcl.unwrap();
        assert!(hcl.contains("2x4xCPU-8GB-100GB"), "{hcl}");
    }

    #[test]
    fn burstable_prefix_with_underscores_maps_to_small_plan() {
        let res = make_res(
            "azurerm_postgresql_server",
            "db",
            &[("sku_name", "B_Standard_B1ms")],
        );
        let hcl = map_postgresql_server(&res).upcloud_hcl.unwrap();
        assert!(hcl.contains("1x1xCPU-2GB-25GB"), "{hcl}");
    }

    // ── map_postgresql_server ─────────────────────────────────────────────────

    #[test]
    fn postgresql_server_maps_to_pg_type() {
        let res = make_res("azurerm_postgresql_server", "pgdb", &[]);
        let r = map_postgresql_server(&res);
        assert_eq!(r.upcloud_type, "upcloud_managed_database_postgresql");
        let hcl = r.upcloud_hcl.unwrap();
        assert!(
            hcl.contains("resource \"upcloud_managed_database_postgresql\" \"pgdb\""),
            "{hcl}"
        );
    }

    #[test]
    fn postgresql_server_version_propagated() {
        let res = make_res("azurerm_postgresql_server", "db", &[("version", "15")]);
        let hcl = map_postgresql_server(&res).upcloud_hcl.unwrap();
        assert!(hcl.contains("version       = \"15\""), "{hcl}");
    }

    #[test]
    fn postgresql_server_defaults_version_16() {
        let res = make_res("azurerm_postgresql_server", "db", &[]);
        let hcl = map_postgresql_server(&res).upcloud_hcl.unwrap();
        assert!(hcl.contains("version       = \"16\""), "{hcl}");
    }

    #[test]
    fn postgresql_server_public_access_disabled() {
        let res = make_res("azurerm_postgresql_server", "db", &[]);
        let hcl = map_postgresql_server(&res).upcloud_hcl.unwrap();
        assert!(hcl.contains("public_access = false"), "{hcl}");
    }

    #[test]
    fn postgresql_server_has_private_network_block() {
        let res = make_res("azurerm_postgresql_server", "db", &[]);
        let hcl = map_postgresql_server(&res).upcloud_hcl.unwrap();
        assert!(hcl.contains("network {"), "{hcl}");
        assert!(hcl.contains("type   = \"private\""), "{hcl}");
        assert!(hcl.contains("<TODO: upcloud_network UUID>"), "{hcl}");
    }

    // ── map_postgresql_flexible_server ────────────────────────────────────────

    #[test]
    fn pg_flexible_maps_to_pg_type() {
        let res = make_res("azurerm_postgresql_flexible_server", "flex", &[]);
        assert_eq!(
            map_postgresql_flexible_server(&res).upcloud_type,
            "upcloud_managed_database_postgresql"
        );
    }

    // ── map_mysql_server ──────────────────────────────────────────────────────

    #[test]
    fn mysql_server_maps_to_mysql_type() {
        let res = make_res("azurerm_mysql_server", "mydb", &[]);
        let r = map_mysql_server(&res);
        assert_eq!(r.upcloud_type, "upcloud_managed_database_mysql");
        assert!(
            r.upcloud_hcl
                .unwrap()
                .contains("upcloud_managed_database_mysql"),
            "type in HCL"
        );
    }

    // ── map_mysql_flexible_server ─────────────────────────────────────────────

    #[test]
    fn mysql_flexible_maps_to_mysql_type() {
        let res = make_res("azurerm_mysql_flexible_server", "flex", &[]);
        assert_eq!(
            map_mysql_flexible_server(&res).upcloud_type,
            "upcloud_managed_database_mysql"
        );
    }

    // ── map_redis_cache ───────────────────────────────────────────────────────

    #[test]
    fn redis_basic_c0_maps_to_valkey_small_plan() {
        let res = make_res(
            "azurerm_redis_cache",
            "cache",
            &[("sku_name", "Basic"), ("capacity", "0")],
        );
        let r = map_redis_cache(&res);
        assert_eq!(r.upcloud_type, "upcloud_managed_database_valkey");
        let hcl = r.upcloud_hcl.unwrap();
        assert!(
            hcl.contains("resource \"upcloud_managed_database_valkey\" \"cache\""),
            "{hcl}"
        );
        assert!(hcl.contains("1x1xCPU-2GB"), "{hcl}");
    }

    #[test]
    fn redis_premium_maps_to_large_valkey_plan() {
        let res = make_res(
            "azurerm_redis_cache",
            "cache",
            &[("sku_name", "Premium"), ("capacity", "1")],
        );
        let hcl = map_redis_cache(&res).upcloud_hcl.unwrap();
        assert!(hcl.contains("1x4xCPU-28GB"), "{hcl}");
    }

    #[test]
    fn redis_has_public_access_disabled() {
        let res = make_res("azurerm_redis_cache", "cache", &[]);
        let hcl = map_redis_cache(&res).upcloud_hcl.unwrap();
        assert!(hcl.contains("public_access = false"), "{hcl}");
    }

    // ── map_cosmosdb_account ──────────────────────────────────────────────────

    #[test]
    fn cosmosdb_is_unsupported() {
        let res = make_res("azurerm_cosmosdb_account", "cosmos", &[]);
        let r = map_cosmosdb_account(&res);
        assert_eq!(
            r.status,
            crate::migration::types::MigrationStatus::Unsupported
        );
        assert!(r.upcloud_hcl.is_none());
    }

    // ── map_mssql_server / map_mssql_database ────────────────────────────────

    #[test]
    fn mssql_server_is_unsupported() {
        let res = make_res("azurerm_mssql_server", "sql", &[]);
        let r = map_mssql_server(&res);
        assert_eq!(
            r.status,
            crate::migration::types::MigrationStatus::Unsupported
        );
        assert!(r.upcloud_hcl.is_none());
    }

    #[test]
    fn mssql_database_is_unsupported() {
        let res = make_res("azurerm_mssql_database", "sqldb", &[]);
        let r = map_mssql_database(&res);
        assert_eq!(
            r.status,
            crate::migration::types::MigrationStatus::Unsupported
        );
    }
}
