//! HCL helpers and template builders shared across provider modules.

use crate::migration::generator::{inside_todo_marker, remove_todo_interpolations};

pub fn is_tf_expr(val: &str) -> bool {
    val.starts_with("var.") || val.starts_with("${") || val.starts_with("local.")
}

/// Return the HCL representation of a scalar value.
///
/// - Pure reference (`var.x`, `local.x`) → unquoted.
/// - Pure interpolation (`${...}` with nothing after the closing `}`) → unquoted.
/// - Mixed template (`${var.x}-suffix`) or plain string → quoted.
fn is_pure_tf_expr(val: &str) -> bool {
    if val.starts_with("var.") || val.starts_with("local.") {
        return true;
    }
    // Pure interpolation: starts with ${ and ends with its matching }
    if val.starts_with("${") && val.ends_with('}') {
        // Make sure the closing } belongs to the opening ${ (no trailing literal text)
        let inner = &val[2..val.len() - 1];
        return !inner.contains('}');
    }
    false
}

pub fn hcl_value(val: &str) -> String {
    if is_pure_tf_expr(val) {
        val.to_string()
    } else {
        format!("\"{}\"", val)
    }
}

pub const FIREWALL_CATCHALL_EGRESS: &str = "  firewall_rule {\n    direction = \"out\"\n    action    = \"accept\"\n    family    = \"IPv4\"\n    comment   = \"Allow all outbound\"\n  }\n";

/// Returns true if `name` is a recognized property of `upcloud_managed_database_postgresql`.
/// Used to filter provider-specific parameter names before suggesting or injecting them.
pub fn is_valid_pg_property(name: &str) -> bool {
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

/// Generate an `upcloud_managed_database_*` HCL resource block.
///
/// `extra_properties` is optional additional lines to inject inside the `properties` block
/// (e.g. `version = "16"` or parameter group markers). Each line should NOT have a trailing newline.
pub fn upcloud_managed_database_hcl(
    upcloud_type: &str,
    name: &str,
    display_name: &str,
    plan: &str,
    network_uuid_placeholder: &str,
    extra_properties: &str,
) -> String {
    let extra = if extra_properties.is_empty() {
        String::new()
    } else {
        format!("\n{}", extra_properties)
    };
    format!(
        r#"resource "{upcloud_type}" "{name}" {{
  name  = "{display_name}"
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
    public_access = false{extra}
  }}
}}
"#,
        upcloud_type = upcloud_type,
        name = name,
        display_name = display_name,
        plan = plan,
        network_uuid_placeholder = network_uuid_placeholder,
        extra = extra,
    )
}

/// Generate an `upcloud_storage` HCL resource block.
pub fn upcloud_storage_hcl(
    name: &str,
    title: &str,
    size: u32,
    tier: &str,
    count_line: &str,
) -> String {
    format!(
        r#"resource "upcloud_storage" "{name}" {{
{count_line}  title = "{title}"
  size  = {size}
  tier  = "{tier}"
  zone  = "__ZONE__"
}}
"#,
        name = name,
        count_line = count_line,
        title = title,
        size = size,
        tier = tier,
    )
}

/// Generate an `upcloud_loadbalancer` HCL resource block.
pub fn upcloud_loadbalancer_hcl(
    name: &str,
    plan: &str,
    networks_block: &str,
    extra_comment: &str,
) -> String {
    let comment = if extra_comment.is_empty() {
        String::new()
    } else {
        format!("\n{}", extra_comment)
    };
    format!(
        r#"resource "upcloud_loadbalancer" "{name}" {{
  name              = "{name}"
  plan              = "{plan}"
  zone              = "__ZONE__"
  configured_status = "started"

{networks}{comment}

  # Backends and frontends are separate resources
}}
"#,
        name = name,
        plan = plan,
        networks = networks_block,
        comment = comment,
    )
}

/// Generate an `upcloud_firewall_rules` HCL resource block.
pub fn upcloud_firewall_rules_hcl(name: &str, rule_blocks: &str) -> String {
    format!(
        "resource \"upcloud_firewall_rules\" \"{name}\" {{\n  server_id = upcloud_server.<TODO>.id\n\n{rules}}}\n",
        name = name,
        rules = rule_blocks,
    )
}

/// Sanitize provider-specific references that leaked into output HCL.
///
/// Replaces `data.{data_prefix}*` data source references and `{resource_prefix}*` resource
/// references (with at least one dot) with `<TODO: ...>` markers. Skips references already
/// inside a `<TODO: ...>` marker.
///
/// `provider_label` is used in the TODO text (e.g. "AWS" or "Azure").
pub fn sanitize_provider_refs(
    mut s: String,
    data_prefix: &str,
    resource_prefix: &str,
    provider_label: &str,
) -> String {
    // data source references (e.g. data.aws_caller_identity.current.account_id)
    let data_needle = format!("data.{}", data_prefix);
    let mut search_from = 0;
    while let Some(rel) = s[search_from..].find(&data_needle) {
        let start = search_from + rel;
        if inside_todo_marker(&s, start) {
            search_from = start + data_needle.len();
            continue;
        }
        let end = s[start..]
            .find(|c: char| !c.is_alphanumeric() && c != '.' && c != '_')
            .map(|off| start + off)
            .unwrap_or(s.len());
        let provider_ref = s[start..end].to_string();
        s = s.replacen(
            &provider_ref,
            &format!("<TODO: remove {} data source ref>", provider_label),
            1,
        );
        search_from = 0;
    }
    // resource references (e.g. aws_instance.web.id or azurerm_subnet.app.id)
    let mut search_from = 0;
    while let Some(rel) = s[search_from..].find(resource_prefix) {
        let start = search_from + rel;
        if inside_todo_marker(&s, start) {
            search_from = start + resource_prefix.len();
            continue;
        }
        let end = s[start..]
            .find(|c: char| !c.is_alphanumeric() && c != '.' && c != '_')
            .map(|off| start + off)
            .unwrap_or(s.len());
        let candidate = &s[start..end];
        if candidate.matches('.').count() >= 1 {
            let owned = candidate.to_string();
            s = s.replacen(
                &owned,
                &format!("<TODO: remove {} resource ref>", provider_label),
                1,
            );
            search_from = 0;
        } else {
            search_from = end;
        }
    }
    s = remove_todo_interpolations(s);
    s
}

// When `is_all_traffic` is true, protocol/port constraints are omitted.
// Callers map provider-specific wildcards (AWS `-1`, Azure `*`) to that bool.
pub fn build_firewall_rule(
    direction: &str,
    from_port: i32,
    to_port: i32,
    protocol: &str,
    description: Option<&str>,
    is_all_traffic: bool,
) -> String {
    let mut s = String::from("  firewall_rule {\n");
    s.push_str(&format!("    direction = \"{}\"\n", direction));
    s.push_str("    action    = \"accept\"\n");
    s.push_str("    family    = \"IPv4\"\n");

    if !is_all_traffic {
        let proto = match protocol {
            "tcp" => "tcp",
            "udp" => "udp",
            "icmp" | "1" => "icmp",
            _ => "tcp",
        };
        s.push_str(&format!("    protocol  = \"{}\"\n", proto));
        if from_port > 0 || to_port < 65535 {
            s.push_str(&format!("    destination_port_start = \"{}\"\n", from_port));
            s.push_str(&format!("    destination_port_end   = \"{}\"\n", to_port));
        }
    }

    if let Some(desc) = description
        && !desc.is_empty()
    {
        s.push_str(&format!("    comment = \"{}\"\n", desc));
    }

    s.push_str("  }");
    s
}

pub fn upcloud_router_hcl(name: &str) -> String {
    format!(
        r#"resource "upcloud_router" "{name}_router" {{
  name = "{name}-router"
}}
"#,
        name = name,
    )
}

pub fn upcloud_object_storage_hcl(resource_name: &str, bucket_name: &str) -> String {
    format!(
        r#"resource "upcloud_managed_object_storage" "{name}" {{
  name              = "{name}-storage"
  region            = "__OBJSTORAGE_REGION__"
  configured_status = "started"
}}

resource "upcloud_managed_object_storage_bucket" "{name}_bucket" {{
  service_uuid = upcloud_managed_object_storage.{name}.id
  name         = "{bucket}"
}}
"#,
        name = resource_name,
        bucket = bucket_name,
    )
}

pub fn storage_devices_snippet(server_ref: &str, storage_ref: &str) -> String {
    format!(
        "# Add inside resource \"upcloud_server\" \"{server_ref}\" {{\n  storage_devices {{\n    storage = upcloud_storage.{storage_ref}.id\n    type    = \"disk\"\n  }}"
    )
}

pub fn upcloud_kubernetes_cluster_hcl(id: &str, name_hcl: &str, version_hcl: &str) -> String {
    format!(
        r#"resource "upcloud_kubernetes_cluster" "{id}" {{
  control_plane_ip_filter = ["0.0.0.0/0"]  # restrict to known CIDRs in production
  name                    = {name_hcl}
  network                 = "<TODO: upcloud_network reference>"
  zone                    = "__ZONE__"
  version                 = {version_hcl}
  # plan = "prod-md"  # use "dev" for non-production clusters (check `upctl kubernetes plans`)
  # private_node_groups = true  # uncomment if node-group subnets are private
}}
"#,
        id = id,
        name_hcl = name_hcl,
        version_hcl = version_hcl,
    )
}
