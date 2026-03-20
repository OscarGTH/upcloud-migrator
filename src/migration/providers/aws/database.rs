use crate::migration::types::{MigrationResult, MigrationStatus};
use crate::terraform::types::TerraformResource;

/// Returns true if `name` is a recognized property of `upcloud_managed_database_postgresql`.
/// Used to filter AWS RDS parameter names before suggesting or injecting them.
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
            | "backup_interval_hours"
            | "backup_minute"
            | "backup_retention_days"
            | "bgwriter_delay"
            | "bgwriter_flush_after"
            | "bgwriter_lru_maxpages"
            | "bgwriter_lru_multiplier"
            | "deadlock_timeout"
            | "default_toast_compression"
            | "enable_ha_replica_dns"
            | "idle_in_transaction_session_timeout"
            | "io_combine_limit"
            | "io_max_combine_limit"
            | "io_max_concurrency"
            | "io_method"
            | "io_workers"
            | "ip_filter"
            | "jit"
            | "log_autovacuum_min_duration"
            | "log_error_verbosity"
            | "log_line_prefix"
            | "log_min_duration_statement"
            | "log_temp_files"
            | "max_connections"
            | "max_files_per_process"
            | "max_locks_per_transaction"
            | "max_logical_replication_workers"
            | "max_parallel_workers"
            | "max_parallel_workers_per_gather"
            | "max_pred_locks_per_transaction"
            | "max_prepared_transactions"
            | "max_replication_slots"
            | "max_slot_wal_keep_size"
            | "max_stack_depth"
            | "max_standby_archive_delay"
            | "max_standby_streaming_delay"
            | "max_sync_workers_per_subscription"
            | "max_wal_senders"
            | "max_worker_processes"
            | "node_count"
            | "password_encryption"
            | "pg_partman_bgw_interval"
            | "pg_partman_bgw_role"
            | "pg_stat_monitor_enable"
            | "pg_stat_monitor_pgsm_enable_query_plan"
            | "pg_stat_monitor_pgsm_max_buckets"
            | "pg_stat_statements_track"
            | "public_access"
            | "service_log"
            | "shared_buffers_percentage"
            | "switchover_windows"
            | "synchronous_replication"
            | "temp_file_limit"
            | "timezone"
            | "track_activity_query_size"
            | "track_commit_timestamp"
            | "track_functions"
            | "track_io_timing"
            | "variant"
            | "version"
            | "wal_sender_timeout"
            | "wal_writer_delay"
            | "work_mem"
    )
}

fn map_engine(engine: &str) -> (&'static str, &'static str) {
    match engine.to_lowercase().as_str() {
        e if e.contains("postgres") => ("upcloud_managed_database_postgresql", "pg"),
        e if e.contains("mysql") => ("upcloud_managed_database_mysql", "mysql"),
        e if e.contains("mariadb") => ("upcloud_managed_database_mysql", "mysql"),
        _ => ("upcloud_managed_database_postgresql", "pg"),
    }
}

fn map_instance_class(class: &str) -> &'static str {
    match class {
        c if c.contains("micro") || c.contains("small") => "1x1xCPU-2GB-25GB",
        c if c.contains("medium") => "1x2xCPU-4GB-50GB",
        c if c.contains("xlarge") => "2x6xCPU-16GB-100GB", // must precede "large" — xlarge contains "large"
        c if c.contains("large") => "2x4xCPU-8GB-100GB",
        _ => "1x2xCPU-4GB-50GB",
    }
}


/// Extract the resource name from a `parameter_group_name` attribute value.
/// Handles references like `aws_db_parameter_group.NAME.name` → `NAME`.
fn param_group_resource_name(attr_value: &str) -> Option<String> {
    let v = attr_value.trim_matches('"');
    let parts: Vec<&str> = v.split('.').collect();
    if parts.len() >= 2 && (parts[0] == "aws_db_parameter_group" || parts[0] == "aws_elasticache_parameter_group") {
        Some(parts[1].to_string())
    } else {
        None
    }
}

/// Extract the resource name from a `db_subnet_group_name` or `subnet_group_name` attribute.
/// Handles references like `aws_db_subnet_group.NAME.name` → `NAME`,
/// `aws_elasticache_subnet_group.NAME.name` → `NAME`, or a plain string name.
fn subnet_group_resource_name(attr_value: &str) -> Option<String> {
    let v = attr_value.trim_matches('"');
    let parts: Vec<&str> = v.split('.').collect();
    if parts.len() >= 2
        && (parts[0] == "aws_db_subnet_group" || parts[0] == "aws_elasticache_subnet_group")
    {
        Some(parts[1].to_string())
    } else if !v.is_empty() && !v.contains('.') && !v.contains('$') {
        // Plain name literal (e.g. "my-subnet-group")
        Some(v.to_string())
    } else {
        None
    }
}

pub fn map_rds_instance(res: &TerraformResource) -> MigrationResult {
    let engine = res.attributes.get("engine").map(|e| e.trim_matches('"')).unwrap_or("postgres");
    let (upcloud_type, engine_short) = map_engine(engine);
    let instance_class = res.attributes.get("instance_class").map(|c| c.trim_matches('"')).unwrap_or("db.t3.medium");
    let plan = map_instance_class(instance_class);
    let _db_name = res.attributes.get("db_name").or_else(|| res.attributes.get("name"))
        .map(|n| n.trim_matches('"').to_string())
        .unwrap_or_else(|| "mydb".into());

    // If the instance references a parameter group, embed a marker so the generator
    // can inject those properties inline after cross-resolving.
    let param_group_marker = res.attributes.get("parameter_group_name")
        .and_then(|v| param_group_resource_name(v))
        .map(|group_name| format!("\n    # __DB_PROPS:{}:{}__", engine_short, group_name))
        .unwrap_or_default();

    // Embed the subnet group name so the generator can resolve it to the right network.
    let network_uuid_placeholder = res.attributes.get("db_subnet_group_name")
        .and_then(|v| subnet_group_resource_name(v))
        .map(|sg| format!("<TODO: upcloud_network UUID subnet_group={}>", sg))
        .unwrap_or_else(|| "<TODO: upcloud_network UUID>".to_string());

    let hcl = format!(
        r#"resource "{upcloud_type}" "{name}" {{
  name  = "{name}-db"
  plan  = "{plan}"
  title = "{name}"
  zone  = "__ZONE__"

  # Private network access — required for servers to reach the database.
  # Set uuid to the upcloud_network in the same zone.
  network {{
    family = "IPv4"
    name   = "private"
    type   = "private"
    uuid   = "{network_uuid_placeholder}"
  }}

  properties {{
    public_access = false{param_group_marker}
  }}
}}
"#,
        upcloud_type = upcloud_type,
        name = res.name,
        plan = plan,
        network_uuid_placeholder = network_uuid_placeholder,
        param_group_marker = param_group_marker,
    );

    let network_note = if network_uuid_placeholder.contains("subnet_group=") {
        "network.uuid resolved from db_subnet_group — verify the network is correct.".into()
    } else {
        "Set network.uuid to the upcloud_network resource in the same zone.".into()
    };

    MigrationResult {
        resource_type: res.resource_type.clone(),
        resource_name: res.name.clone(),
        source_file: res.source_file.display().to_string(),
        status: MigrationStatus::Compatible,
        upcloud_type: upcloud_type.into(),
        upcloud_hcl: Some(hcl),
        snippet: None,
        parent_resource: None,
        notes: vec![
            format!("engine '{}' → {}", engine, upcloud_type),
            "Migrate DB parameters and connection strings manually".into(),
            network_note,
        ],
        source_hcl: None,
    }
}

pub fn map_rds_cluster(res: &TerraformResource) -> MigrationResult {
    let engine = res.attributes.get("engine").map(|e| e.trim_matches('"')).unwrap_or("aurora-postgresql");
    let (upcloud_type, engine_short) = map_engine(engine);

    let param_group_marker = res.attributes.get("db_cluster_parameter_group_name")
        .and_then(|v| param_group_resource_name(v))
        .map(|group_name| format!("\n    # __DB_PROPS:{}:{}__", engine_short, group_name))
        .unwrap_or_default();

    let network_uuid_placeholder = res.attributes.get("db_subnet_group_name")
        .and_then(|v| subnet_group_resource_name(v))
        .map(|sg| format!("<TODO: upcloud_network UUID subnet_group={}>", sg))
        .unwrap_or_else(|| "<TODO: upcloud_network UUID>".to_string());

    let hcl = format!(
        r#"resource "{upcloud_type}" "{name}" {{
  name  = "{name}-cluster"
  plan  = "1x2xCPU-4GB-50GB"
  title = "{name}"
  zone  = "__ZONE__"

  network {{
    family = "IPv4"
    name   = "private"
    type   = "private"
    uuid   = "{network_uuid_placeholder}"
  }}

  properties {{
    public_access = false{param_group_marker}
  }}
}}
"#,
        upcloud_type = upcloud_type,
        name = res.name,
        network_uuid_placeholder = network_uuid_placeholder,
        param_group_marker = param_group_marker,
    );

    MigrationResult {
        resource_type: res.resource_type.clone(),
        resource_name: res.name.clone(),
        source_file: res.source_file.display().to_string(),
        status: MigrationStatus::Compatible,
        upcloud_type: upcloud_type.into(),
        upcloud_hcl: Some(hcl),
        snippet: None,
        parent_resource: None,
        notes: vec![
            "Aurora cluster → UpCloud Managed Database (no multi-master equivalent)".into(),
            "Review cluster-specific features like read replicas".into(),
        ],
            source_hcl: None,
    }
}

/// Extract `parameter { name = "..." value = "..." }` blocks from raw HCL text.
pub(crate) fn extract_parameter_blocks(raw_hcl: &str) -> Vec<(String, String)> {
    let mut params = Vec::new();
    let mut in_block = false;
    let mut cur_name: Option<String> = None;
    let mut cur_value: Option<String> = None;

    for line in raw_hcl.lines() {
        let t = line.trim();
        if t.starts_with("parameter") && t.contains('{') {
            in_block = true;
            cur_name = None;
            cur_value = None;
        } else if in_block {
            if t == "}" {
                if let (Some(n), Some(v)) = (cur_name.take(), cur_value.take()) {
                    params.push((n, v));
                }
                in_block = false;
            } else if let Some((k, v)) = t.split_once('=') {
                let key = k.trim();
                let val = v.trim().trim_matches('"').to_string();
                match key {
                    "name" => cur_name = Some(val),
                    "value" => cur_value = Some(val),
                    _ => {}
                }
            }
        }
    }
    params
}

pub fn map_db_parameter_group(res: &TerraformResource) -> MigrationResult {
    let family = res.attributes.get("family").map(|f| f.trim_matches('"')).unwrap_or("postgres");
    let (db_type, props_prefix) = if family.contains("mysql") || family.contains("mariadb") {
        ("upcloud_managed_database_mysql", "mysql")
    } else {
        ("upcloud_managed_database_postgresql", "pg")
    };

    let params = extract_parameter_blocks(&res.raw_hcl);

    let (valid, invalid): (Vec<_>, Vec<_>) = params.iter().partition(|(n, _)| is_valid_pg_property(n));

    let mut notes = vec![
        format!(
            "No standalone UpCloud equivalent — {} parameter(s) merged inline into the target {} properties block.",
            params.len(), db_type
        ),
    ];
    if !invalid.is_empty() {
        notes.push(format!(
            "{} unsupported parameter(s) commented out: {}",
            invalid.len(),
            invalid.iter().map(|(n, _)| n.as_str()).collect::<Vec<_>>().join(", ")
        ));
    }
    let _ = (valid, props_prefix); // used only for notes above

    MigrationResult {
        resource_type: res.resource_type.clone(),
        resource_name: res.name.clone(),
        source_file: res.source_file.display().to_string(),
        status: MigrationStatus::Partial,
        upcloud_type: format!("({} parameters → properties block)", props_prefix),
        upcloud_hcl: None, // no standalone output — parameters are injected inline via __DB_PROPS marker
        snippet: None,
        parent_resource: None,
        notes,
        source_hcl: None,
    }
}

pub fn map_db_subnet_group(res: &TerraformResource) -> MigrationResult {
    MigrationResult {
        resource_type: res.resource_type.clone(),
        resource_name: res.name.clone(),
        source_file: res.source_file.display().to_string(),
        status: MigrationStatus::Partial,
        upcloud_type: "(subnet group → network block)".into(),
        upcloud_hcl: None,
        snippet: None,
        parent_resource: None,
        notes: vec![
            "aws_db_subnet_group has no UpCloud equivalent resource.".into(),
            "Network placement is configured directly in the `network {}` block of your upcloud_managed_database_* resource.".into(),
        ],
        source_hcl: None,
    }
}

pub fn map_elasticache_subnet_group(res: &TerraformResource) -> MigrationResult {
    MigrationResult {
        resource_type: res.resource_type.clone(),
        resource_name: res.name.clone(),
        source_file: res.source_file.display().to_string(),
        status: MigrationStatus::Partial,
        upcloud_type: "(subnet group → network block)".into(),
        upcloud_hcl: None,
        snippet: None,
        parent_resource: None,
        notes: vec![
            "aws_elasticache_subnet_group has no UpCloud equivalent resource.".into(),
            "Network placement is configured directly in the `network {}` block of your upcloud_managed_database_valkey resource.".into(),
        ],
        source_hcl: None,
    }
}

pub fn map_elasticache_parameter_group(res: &TerraformResource) -> MigrationResult {
    let params = extract_parameter_blocks(&res.raw_hcl);

    let mut lines = vec![
        format!("# aws_elasticache_parameter_group \"{}\" has no standalone UpCloud resource.", res.name),
        "# Add these parameters to the properties {} block of your upcloud_managed_database_valkey resource:".into(),
        "#".into(),
        "#   properties {".into(),
        "#     public_access = false".into(),
    ];
    if params.is_empty() {
        lines.push("#     # <TODO: migrate valkey parameters manually>".into());
    } else {
        for (name, value) in &params {
            // AWS ElastiCache param names use hyphens; UpCloud Valkey properties use
            // "valkey_" prefix with underscores (e.g. maxmemory-policy → valkey_maxmemory_policy)
            let valkey_name = format!("valkey_{}", name.replace('-', "_"));
            lines.push(format!("#     {} = \"{}\"", valkey_name, value));
        }
    }
    lines.push("#   }".into());

    let hcl = lines.join("\n");

    MigrationResult {
        resource_type: res.resource_type.clone(),
        resource_name: res.name.clone(),
        source_file: res.source_file.display().to_string(),
        status: MigrationStatus::Partial,
        upcloud_type: "(valkey parameters → properties block)".into(),
        upcloud_hcl: Some(hcl),
        snippet: None,
        parent_resource: None,
        notes: vec![
            "aws_elasticache_parameter_group has no UpCloud equivalent resource.".into(),
            format!("Add the {} properties shown above to your upcloud_managed_database_valkey resource.", params.len()),
        ],
        source_hcl: None,
    }
}

/// Map an ElastiCache node_type to the closest UpCloud Valkey plan.
/// Valkey plans don't include a disk-size suffix.
fn map_elasticache_node_type(node_type: &str) -> &'static str {
    match node_type {
        t if t.contains("micro") || t.contains("small") => "1x1xCPU-2GB",
        t if t.contains("medium") => "1x2xCPU-4GB",
        t if t.contains("xlarge") => "1x4xCPU-28GB", // must precede "large"
        t if t.contains("large") => "1x2xCPU-8GB",
        _ => "1x1xCPU-2GB",
    }
}

pub fn map_elasticache_cluster(res: &TerraformResource) -> MigrationResult {
    let engine = res.attributes.get("engine").map(|e| e.trim_matches('"')).unwrap_or("redis");
    let node_type = res.attributes.get("node_type").map(|v| v.trim_matches('"')).unwrap_or("");
    let plan = map_elasticache_node_type(node_type);

    let network_uuid_placeholder = res.attributes.get("subnet_group_name")
        .and_then(|v| subnet_group_resource_name(v))
        .map(|sg| format!("<TODO: upcloud_network UUID subnet_group={}>", sg))
        .unwrap_or_else(|| "<TODO: upcloud_network UUID>".to_string());

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
    uuid   = "{network_uuid_placeholder}"
  }}

  properties {{
    public_access = false
  }}
}}
"#,
        name = res.name,
        plan = plan,
        network_uuid_placeholder = network_uuid_placeholder,
    );

    let mut notes = vec![
        format!("ElastiCache ({}) → UpCloud Managed Valkey (Redis-compatible)", engine),
        "Update connection strings to point to UpCloud Valkey endpoint".into(),
    ];
    if !node_type.is_empty() {
        notes.push(format!("node_type '{}' → plan '{}'", node_type, plan));
    }

    MigrationResult {
        resource_type: res.resource_type.clone(),
        resource_name: res.name.clone(),
        source_file: res.source_file.display().to_string(),
        status: MigrationStatus::Compatible,
        upcloud_type: "upcloud_managed_database_valkey".into(),
        upcloud_hcl: Some(hcl),
        snippet: None,
        parent_resource: None,
        notes,
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
            attributes: attrs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect(),
            source_file: PathBuf::from("test.tf"),
            raw_hcl: String::new(),
        }
    }

    // ── map_rds_instance ──────────────────────────────────────────────────────

    #[test]
    fn rds_postgres_maps_to_postgresql_resource() {
        let res = make_res("aws_db_instance", "db", &[
            ("engine", "postgres"),
            ("instance_class", "db.t3.medium"),
        ]);
        let r = map_rds_instance(&res);
        assert_eq!(r.upcloud_type, "upcloud_managed_database_postgresql");
        let hcl = r.upcloud_hcl.unwrap();
        assert!(hcl.contains("resource \"upcloud_managed_database_postgresql\" \"db\""), "{hcl}");
        assert!(hcl.contains("zone  = \"__ZONE__\""), "{hcl}");
    }

    #[test]
    fn rds_mysql_maps_to_mysql_resource() {
        let res = make_res("aws_db_instance", "mydb", &[("engine", "mysql")]);
        let r = map_rds_instance(&res);
        assert_eq!(r.upcloud_type, "upcloud_managed_database_mysql");
        assert!(r.upcloud_hcl.unwrap().contains("upcloud_managed_database_mysql"));
    }

    #[test]
    fn rds_mariadb_maps_to_mysql_resource() {
        let res = make_res("aws_db_instance", "maria", &[("engine", "mariadb")]);
        let r = map_rds_instance(&res);
        assert_eq!(r.upcloud_type, "upcloud_managed_database_mysql");
    }

    #[test]
    fn rds_unknown_engine_defaults_to_postgresql() {
        let res = make_res("aws_db_instance", "db", &[("engine", "oracle-ee")]);
        let r = map_rds_instance(&res);
        assert_eq!(r.upcloud_type, "upcloud_managed_database_postgresql");
    }

    #[test]
    fn rds_instance_class_maps_to_plan() {
        let cases = [
            ("db.t3.micro",  "1x1xCPU-2GB-25GB"),
            ("db.t3.small",  "1x1xCPU-2GB-25GB"),
            ("db.t3.medium", "1x2xCPU-4GB-50GB"),
            ("db.r5.large",  "2x4xCPU-8GB-100GB"),
            ("db.r5.xlarge", "2x6xCPU-16GB-100GB"),
        ];
        for (class, expected_plan) in &cases {
            let res = make_res("aws_db_instance", "db", &[
                ("engine", "postgres"),
                ("instance_class", class),
            ]);
            let hcl = map_rds_instance(&res).upcloud_hcl.unwrap();
            assert!(hcl.contains(expected_plan), "class {class} should map to plan {expected_plan}\n{hcl}");
        }
    }

    #[test]
    fn rds_instance_public_access_disabled() {
        let res = make_res("aws_db_instance", "db", &[("engine", "postgres")]);
        let hcl = map_rds_instance(&res).upcloud_hcl.unwrap();
        assert!(hcl.contains("public_access = false"), "{hcl}");
    }

    #[test]
    fn rds_instance_has_private_network_block() {
        let res = make_res("aws_db_instance", "db", &[("engine", "postgres")]);
        let hcl = map_rds_instance(&res).upcloud_hcl.unwrap();
        assert!(hcl.contains("network {"), "must have a network block\n{hcl}");
        assert!(hcl.contains("type   = \"private\""), "{hcl}");
        assert!(hcl.contains("upcloud_network UUID"), "must have a TODO for network uuid\n{hcl}");
    }

    #[test]
    fn elasticache_has_private_network_block() {
        let res = make_res("aws_elasticache_cluster", "cache", &[("engine", "redis")]);
        let hcl = map_elasticache_cluster(&res).upcloud_hcl.unwrap();
        assert!(hcl.contains("network {"), "{hcl}");
        assert!(hcl.contains("type   = \"private\""), "{hcl}");
    }

    // ── map_rds_cluster ───────────────────────────────────────────────────────

    #[test]
    fn rds_cluster_aurora_pg_maps_to_postgresql() {
        let res = make_res("aws_rds_cluster", "cluster", &[("engine", "aurora-postgresql")]);
        let r = map_rds_cluster(&res);
        assert_eq!(r.upcloud_type, "upcloud_managed_database_postgresql");
        let hcl = r.upcloud_hcl.unwrap();
        assert!(hcl.contains("resource \"upcloud_managed_database_postgresql\" \"cluster\""), "{hcl}");
    }

    #[test]
    fn rds_cluster_aurora_mysql_maps_to_mysql() {
        let res = make_res("aws_rds_cluster", "c", &[("engine", "aurora-mysql")]);
        assert_eq!(map_rds_cluster(&res).upcloud_type, "upcloud_managed_database_mysql");
    }

    // ── map_elasticache_cluster ───────────────────────────────────────────────

    #[test]
    fn elasticache_redis_maps_to_valkey() {
        let res = make_res("aws_elasticache_cluster", "cache", &[("engine", "redis")]);
        let r = map_elasticache_cluster(&res);
        assert_eq!(r.upcloud_type, "upcloud_managed_database_valkey");
        let hcl = r.upcloud_hcl.unwrap();
        assert!(hcl.contains("resource \"upcloud_managed_database_valkey\" \"cache\""), "{hcl}");
    }

    #[test]
    fn elasticache_public_access_disabled() {
        let res = make_res("aws_elasticache_cluster", "cache", &[("engine", "redis")]);
        let hcl = map_elasticache_cluster(&res).upcloud_hcl.unwrap();
        assert!(hcl.contains("public_access = false"), "{hcl}");
    }
}