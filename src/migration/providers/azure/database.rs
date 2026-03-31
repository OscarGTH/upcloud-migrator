use crate::migration::types::{MigrationResult, MigrationStatus};
use crate::terraform::types::TerraformResource;

/// Returns true if `name` is a recognized property of `upcloud_managed_database_postgresql`.
pub(crate) fn is_valid_pg_property(name: &str) -> bool {
    matches!(
        name,
        "admin_password"
            | "admin_username"
            | "automatic_utility_network_ip_filter"
            | "autovacuum_analyze_scale_factor"
            | "autovacuum_analyze_threshold"
            | "autovacuum_freeze_max_age"
            | "autovacuum_max_workers"
            | "autovacuum_naptime"
            | "autovacuum_vacuum_cost_delay"
            | "autovacuum_vacuum_cost_limit"
            | "autovacuum_vacuum_scale_factor"
            | "autovacuum_vacuum_threshold"
            | "backup_hour"
            | "backup_minute"
            | "bgwriter_delay"
            | "bgwriter_flush_after"
            | "bgwriter_lru_maxpages"
            | "bgwriter_lru_multiplier"
            | "deadlock_timeout"
            | "idle_in_transaction_session_timeout"
            | "jit"
            | "log_autovacuum_min_duration"
            | "log_error_verbosity"
            | "log_line_prefix"
            | "log_min_duration_statement"
            | "log_temp_files"
            | "max_connections"
            | "max_locks_per_transaction"
            | "max_parallel_workers"
            | "max_parallel_workers_per_gather"
            | "max_replication_slots"
            | "max_wal_senders"
            | "max_worker_processes"
            | "public_access"
            | "shared_buffers_percentage"
            | "synchronous_replication"
            | "timezone"
            | "track_activity_query_size"
            | "track_commit_timestamp"
            | "track_functions"
            | "track_io_timing"
            | "version"
            | "work_mem"
    )
}

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

    let hcl = format!(
        r#"resource "upcloud_managed_database_postgresql" "{name}" {{
  name  = "{name}-db"
  plan  = "{plan}"
  title = "{name}"
  zone  = "__ZONE__"

  network {{
    family = "IPv4"
    name   = "private"
    type   = "private"
    uuid   = "<TODO: upcloud_network UUID>"
  }}

  properties {{
    public_access = false
    version       = "{version}"
  }}
}}
"#,
        name = res.name,
        plan = plan,
        version = version,
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
            format!("Azure PostgreSQL (SKU: {}) → UpCloud Managed PostgreSQL (plan: {})", sku, plan),
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

    let hcl = format!(
        r#"resource "upcloud_managed_database_postgresql" "{name}" {{
  name  = "{name}-db"
  plan  = "{plan}"
  title = "{name}"
  zone  = "__ZONE__"

  network {{
    family = "IPv4"
    name   = "private"
    type   = "private"
    uuid   = "<TODO: upcloud_network UUID>"
  }}

  properties {{
    public_access = false
    version       = "{version}"
  }}
}}
"#,
        name = res.name,
        plan = plan,
        version = version,
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
            format!("Azure PostgreSQL Flexible (SKU: {}) → UpCloud Managed PostgreSQL (plan: {})", sku, plan),
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

    let hcl = format!(
        r#"resource "upcloud_managed_database_mysql" "{name}" {{
  name  = "{name}-db"
  plan  = "{plan}"
  title = "{name}"
  zone  = "__ZONE__"

  network {{
    family = "IPv4"
    name   = "private"
    type   = "private"
    uuid   = "<TODO: upcloud_network UUID>"
  }}

  properties {{
    public_access = false
    version       = "{version}"
  }}
}}
"#,
        name = res.name,
        plan = plan,
        version = version,
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
            format!("Azure MySQL (SKU: {}) → UpCloud Managed MySQL (plan: {})", sku, plan),
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

    let hcl = format!(
        r#"resource "upcloud_managed_database_mysql" "{name}" {{
  name  = "{name}-db"
  plan  = "{plan}"
  title = "{name}"
  zone  = "__ZONE__"

  network {{
    family = "IPv4"
    name   = "private"
    type   = "private"
    uuid   = "<TODO: upcloud_network UUID>"
  }}

  properties {{
    public_access = false
  }}
}}
"#,
        name = res.name,
        plan = plan,
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
            format!("Azure MySQL Flexible (SKU: {}) → UpCloud Managed MySQL (plan: {})", sku, plan),
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

    let hcl = format!(
        r#"resource "upcloud_managed_database_valkey" "{name}" {{
  name  = "{name}-cache"
  plan  = "{plan}"
  title = "{name}"
  zone  = "__ZONE__"

  network {{
    family = "IPv4"
    name   = "private"
    type   = "private"
    uuid   = "<TODO: upcloud_network UUID>"
  }}

  properties {{
    public_access = false
  }}
}}
"#,
        name = res.name,
        plan = plan,
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
            format!("Azure Redis Cache ({}/{}) → UpCloud Managed Valkey (plan: {})", sku, capacity, plan),
            "Valkey is Redis-compatible. Migrate connection strings and auth tokens.".into(),
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
            format!("Azure CosmosDB ({}) has no direct UpCloud equivalent.", kind),
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
        let res = make_res("azurerm_postgresql_server", "db", &[("sku_name", "B_Gen5_1")]);
        let hcl = map_postgresql_server(&res).upcloud_hcl.unwrap();
        assert!(hcl.contains("1x1xCPU-2GB-25GB"), "{hcl}");
    }

    #[test]
    fn general_purpose_sku_maps_to_medium_plan() {
        let res = make_res("azurerm_postgresql_server", "db", &[("sku_name", "GP_Gen5_4")]);
        let hcl = map_postgresql_server(&res).upcloud_hcl.unwrap();
        assert!(hcl.contains("1x2xCPU-4GB-50GB"), "{hcl}");
    }

    #[test]
    fn memory_optimized_sku_maps_to_large_plan() {
        let res = make_res("azurerm_postgresql_server", "db", &[("sku_name", "MO_Gen5_8")]);
        let hcl = map_postgresql_server(&res).upcloud_hcl.unwrap();
        assert!(hcl.contains("2x4xCPU-8GB-100GB"), "{hcl}");
    }

    #[test]
    fn burstable_prefix_with_underscores_maps_to_small_plan() {
        let res = make_res("azurerm_postgresql_server", "db", &[("sku_name", "B_Standard_B1ms")]);
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
        assert!(hcl.contains("resource \"upcloud_managed_database_postgresql\" \"pgdb\""), "{hcl}");
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
        assert!(r.upcloud_hcl.unwrap().contains("upcloud_managed_database_mysql"), "type in HCL");
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
        assert!(hcl.contains("resource \"upcloud_managed_database_valkey\" \"cache\""), "{hcl}");
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
        assert_eq!(r.status, crate::migration::types::MigrationStatus::Unsupported);
        assert!(r.upcloud_hcl.is_none());
    }

    // ── map_mssql_server / map_mssql_database ────────────────────────────────

    #[test]
    fn mssql_server_is_unsupported() {
        let res = make_res("azurerm_mssql_server", "sql", &[]);
        let r = map_mssql_server(&res);
        assert_eq!(r.status, crate::migration::types::MigrationStatus::Unsupported);
        assert!(r.upcloud_hcl.is_none());
    }

    #[test]
    fn mssql_database_is_unsupported() {
        let res = make_res("azurerm_mssql_database", "sqldb", &[]);
        let r = map_mssql_database(&res);
        assert_eq!(r.status, crate::migration::types::MigrationStatus::Unsupported);
    }
}
