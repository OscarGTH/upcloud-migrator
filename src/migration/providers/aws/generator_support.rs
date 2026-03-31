//! AWS-specific source HCL extraction and output processing helpers.
//!
//! These functions are used by [`AwsSourceProvider`](super::AwsSourceProvider) to
//! implement the [`SourceProvider`](crate::migration::providers::SourceProvider) trait.
//! They parse AWS Terraform patterns (resource references, data sources, etc.)
//! and rewrite them for UpCloud.

use crate::migration::generator::inside_todo_marker;
use crate::migration::providers::shared;

// ── Source HCL extraction helpers ─────────────────────────────────────────────

/// Scan an `aws_instance` source HCL block and return all security group resource names
/// referenced in `vpc_security_group_ids` or `security_groups` attributes.
/// E.g. `vpc_security_group_ids = [aws_security_group.docker_demo.id]` → `["docker_demo"]`
pub fn extract_sg_refs_from_instance_hcl(hcl: &str) -> Vec<String> {
    let mut refs = Vec::new();
    const PREFIX: &str = "aws_security_group.";
    for line in hcl.lines() {
        let mut search = line;
        while let Some(pos) = search.find(PREFIX) {
            let after = &search[pos + PREFIX.len()..];
            let end = after
                .find(|c: char| !c.is_alphanumeric() && c != '_')
                .unwrap_or(after.len());
            let name = &after[..end];
            if !name.is_empty() {
                refs.push(name.to_string());
            }
            search = &search[pos + 1..];
        }
    }
    refs
}

/// Extract the subnet name from an `aws_instance` source HCL block.
/// Returns e.g. `"public_a"` from `subnet_id = aws_subnet.public_a.id`.
pub fn extract_subnet_id_from_instance_hcl(hcl: &str) -> Option<String> {
    for line in hcl.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("subnet_id")
            && trimmed.contains('=')
            && let Some(pos) = trimmed.find("aws_subnet.")
        {
            let after = &trimmed[pos + "aws_subnet.".len()..];
            // Strip trailing `.id`, `.arn`, etc. — take only the resource name segment
            let name = after.split('.').next().unwrap_or("").trim_matches('"');
            if !name.is_empty() {
                return Some(name.to_string());
            }
        }
    }
    None
}

/// Extract subnet resource names from an `aws_db_subnet_group` or
/// `aws_elasticache_subnet_group` source HCL block.
/// Looks for `aws_subnet.NAME.id` (or `.name`) patterns in the `subnet_ids` list.
pub fn extract_subnet_names_from_subnet_group(hcl: &str) -> Vec<String> {
    let mut names = Vec::new();
    const PREFIX: &str = "aws_subnet.";
    for word in hcl.split([',', '[', ']', '\n', ' ']) {
        let word = word.trim();
        if let Some(after) = word.strip_prefix(PREFIX) {
            // after = "NAME.id" or "NAME.name" — take the first segment
            let name_end = after.find('.').unwrap_or(after.len());
            let name = &after[..name_end];
            if !name.is_empty() && !names.contains(&name.to_string()) {
                names.push(name.to_string());
            }
        }
    }
    names
}

/// Extract the target group name from an `aws_lb_listener` source HCL block.
/// Looks for `target_group_arn = aws_lb_target_group.NAME.arn` inside a forward action.
pub fn extract_tg_from_listener_source_hcl(hcl: &str) -> Option<String> {
    const PREFIX: &str = "aws_lb_target_group.";
    for line in hcl.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("target_group_arn")
            && trimmed.contains(PREFIX)
            && let Some(pos) = trimmed.find(PREFIX)
        {
            let after = &trimmed[pos + PREFIX.len()..];
            let name = after.split('.').next().unwrap_or("").trim_matches('"');
            if !name.is_empty() {
                return Some(name.to_string());
            }
        }
    }
    None
}

/// Extract (tg_name, server_name) from an `aws_lb_target_group_attachment` source HCL.
/// Returns e.g. `("web", "web")` from:
///   `target_group_arn = aws_lb_target_group.web.arn`
///   `target_id        = aws_instance.web[0].id`
pub fn extract_tg_server_from_attachment_source_hcl(hcl: &str) -> Option<(String, String)> {
    let mut tg_name: Option<String> = None;
    let mut server_name: Option<String> = None;
    for line in hcl.lines() {
        let trimmed = line.trim();
        if tg_name.is_none()
            && trimmed.starts_with("target_group_arn")
            && let Some(pos) = trimmed.find("aws_lb_target_group.")
        {
            let after = &trimmed[pos + "aws_lb_target_group.".len()..];
            let name = after.split('.').next().unwrap_or("").trim_matches('"');
            if !name.is_empty() {
                tg_name = Some(name.to_string());
            }
        }
        if server_name.is_none()
            && trimmed.starts_with("target_id")
            && let Some(pos) = trimmed.find("aws_instance.")
        {
            let after = &trimmed[pos + "aws_instance.".len()..];
            // Strip any index suffix like `[0]` — take alphanumeric+underscore only
            let name: String = after
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_')
                .collect();
            if !name.is_empty() {
                server_name = Some(name);
            }
        }
    }
    match (tg_name, server_name) {
        (Some(tg), Some(srv)) => Some((tg, srv)),
        _ => None,
    }
}

/// Extract LB resource name from an `aws_lb_listener` source HCL.
/// Looks for `load_balancer_arn = aws_lb.NAME.arn` pattern.
pub fn extract_lb_name_from_listener_hcl(hcl: &str) -> Option<String> {
    hcl.lines()
        .find(|l| l.trim().starts_with("load_balancer_arn"))
        .and_then(|l| l.find("aws_lb.").map(|p| &l[p + "aws_lb.".len()..]))
        .and_then(|s| s.split('.').next())
        .map(|s| s.trim_matches('"').to_string())
        .filter(|s| !s.is_empty())
}

// ── Output reference sanitization and rewriting ───────────────────────────────

/// Replace AWS-specific data source references that leaked into output HCL.
/// Handles patterns like `data.aws_caller_identity.current.account_id`.
/// Also replaces cross-resource AWS refs like `aws_security_group.main.id`.
/// Skips any `aws_` occurrences that are already inside a `<TODO: ...>` marker.
pub fn sanitize_aws_refs(s: String) -> String {
    shared::sanitize_provider_refs(s, "aws_", "aws_", "AWS")
}

/// Map an AWS resource type name to its UpCloud equivalent.
pub fn upcloud_type_for_aws(aws_type: &str) -> Option<&'static str> {
    match aws_type {
        "aws_instance" | "aws_launch_template" | "aws_launch_configuration" => {
            Some("upcloud_server")
        }
        "aws_lb" | "aws_alb" | "aws_elb" => Some("upcloud_loadbalancer"),
        "aws_vpc" => Some("upcloud_router"),
        "aws_subnet" => Some("upcloud_network"),
        "aws_security_group" => Some("upcloud_firewall_rules"),
        "aws_db_instance" | "aws_rds_cluster" | "aws_rds_cluster_instance" => {
            Some("upcloud_managed_database_postgresql")
        }
        "aws_elasticache_cluster" | "aws_elasticache_replication_group" => {
            Some("upcloud_managed_database_valkey")
        }
        "aws_eks_cluster" => Some("upcloud_kubernetes_cluster"),
        "aws_eip" => Some("upcloud_floating_ip_address"),
        _ => None,
    }
}

/// Map an AWS resource type name to its UpCloud resource name.
/// Some AWS resource types get a suffix appended to their name in the UpCloud mapping.
/// E.g. aws_vpc "main" → upcloud_router "main_router".
pub fn upcloud_resource_name_for(aws_type: &str, resource_name: &str) -> String {
    match aws_type {
        "aws_vpc" => format!("{}_router", resource_name),
        _ => resource_name.to_string(),
    }
}

/// Map an AWS attribute name to its UpCloud equivalent for a given UpCloud type.
/// Returns `None` for attributes with no direct equivalent (caller injects a TODO).
pub fn upcloud_attr_for(upcloud_type: &str, aws_attr: &str) -> Option<&'static str> {
    match (upcloud_type, aws_attr) {
        // upcloud_server
        ("upcloud_server", "id") => Some("id"),
        ("upcloud_server", "public_ip") => Some("network_interface[0].ip_address"),
        ("upcloud_server", "private_ip") => Some("network_interface[1].ip_address"),
        // upcloud_loadbalancer
        ("upcloud_loadbalancer", "id") => Some("id"),
        ("upcloud_loadbalancer", "dns_name") => None, // handled as special case in rewrite_output_refs
        // upcloud_router
        ("upcloud_router", "id") => Some("id"),
        // upcloud_network
        ("upcloud_network", "id") => Some("id"),
        ("upcloud_network", "cidr_block") => Some("ip_network[0].address"),
        // upcloud_firewall_rules
        ("upcloud_firewall_rules", "id") => Some("id"),
        // upcloud_managed_database_* (postgresql / valkey / mysql / opensearch share these)
        (t, "id") if t.starts_with("upcloud_managed_database") => Some("id"),
        (t, "endpoint") if t.starts_with("upcloud_managed_database") => Some("service_host"),
        (t, "address") if t.starts_with("upcloud_managed_database") => Some("service_host"),
        (t, "port") if t.starts_with("upcloud_managed_database") => Some("service_port"),
        (t, "username") if t.starts_with("upcloud_managed_database") => Some("service_username"),
        (t, "password") if t.starts_with("upcloud_managed_database") => Some("service_password"),
        (t, "primary_endpoint_address") if t.starts_with("upcloud_managed_database") => {
            Some("service_host")
        }
        // upcloud_kubernetes_cluster
        ("upcloud_kubernetes_cluster", "id") => Some("id"),
        ("upcloud_kubernetes_cluster", "name") => Some("name"),
        // upcloud_floating_ip_address
        ("upcloud_floating_ip_address", "id") => Some("id"),
        ("upcloud_floating_ip_address", "public_ip") => Some("ip_address"),
        ("upcloud_floating_ip_address", "allocation_id") => Some("id"),
        _ => None,
    }
}

/// Rewrite AWS resource references in output/locals blocks to UpCloud equivalents.
/// When the attribute has no direct mapping a `<TODO: was .attr>` suffix is
/// injected so it surfaces in the TODO review screen. Unknown AWS resource
/// types get a full `<TODO: was aws_type.name.attr>` replacement.
pub fn rewrite_output_refs(s: &str) -> String {
    let mut result = s.to_string();
    let mut search_from = 0usize;

    while let Some(rel) = result[search_from..].find("aws_") {
        let start = search_from + rel;

        if inside_todo_marker(&result, start) {
            search_from = start + 4;
            continue;
        }

        // Capture the full Terraform traversal: TYPE.NAME.ATTR[…].subattr…
        // Valid chars: alphanumeric, '_', '.', '[', ']', '*' (splat operator in [*])
        let end = result[start..]
            .find(|c: char| !c.is_alphanumeric() && !matches!(c, '.' | '_' | '[' | ']' | '*'))
            .map(|off| start + off)
            .unwrap_or(result.len());

        let candidate = &result[start..end];

        // Need at least two dots: aws_TYPE.NAME.ATTR
        if candidate.matches('.').count() < 2 {
            search_from = end;
            continue;
        }

        // Split into aws_type / resource_name / attr_path
        let first_dot = candidate.find('.').unwrap();
        let second_dot = first_dot + 1 + candidate[first_dot + 1..].find('.').unwrap();
        let aws_type = &candidate[..first_dot];
        let resource_name = &candidate[first_dot + 1..second_dot];
        let attr_path = &candidate[second_dot + 1..];

        // The lookup key is just the first identifier segment (before '[' or '.')
        let attr_key = attr_path.split(['[', '.']).next().unwrap_or(attr_path);

        let new_ref = if let Some(upcloud_type) = upcloud_type_for_aws(aws_type) {
            let upcloud_name = upcloud_resource_name_for(aws_type, resource_name);
            if upcloud_type == "upcloud_loadbalancer" && attr_key == "dns_name" {
                // dns_name moved to the per-network block; get the public network's dns_name.
                format!(
                    "[for n in {upcloud_type}.{upcloud_name}.networks : n.dns_name if n.type == \"public\"][0]",
                    upcloud_type = upcloud_type,
                    upcloud_name = upcloud_name,
                )
            } else if let Some(upcloud_attr) = upcloud_attr_for(upcloud_type, attr_key) {
                format!("{}.{}.{}", upcloud_type, upcloud_name, upcloud_attr)
            } else {
                // Emit as a quoted string so the result is valid HCL.
                // A bare `type.name.<TODO:...>` is invalid because <TODO:...>
                // is not a legal attribute identifier.
                format!(
                    "\"<TODO: was {}.{}.{}, check UpCloud provider docs>\"",
                    upcloud_type, upcloud_name, attr_path,
                )
            }
        } else {
            format!("\"<TODO: was {}, no known UpCloud equivalent>\"", candidate)
        };

        let owned = candidate.to_string();
        result = result.replacen(&owned, &new_ref, 1);
        search_from = 0; // restart — string length may have changed
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::migration::generator::remove_todo_interpolations;

    #[test]
    fn extract_sg_refs_finds_security_groups() {
        let hcl = concat!(
            "resource \"aws_instance\" \"docker_server\" {\n",
            "  vpc_security_group_ids = [aws_security_group.docker_demo.id]\n",
            "}\n"
        );
        let refs = extract_sg_refs_from_instance_hcl(hcl);
        assert!(
            refs.contains(&"docker_demo".to_string()),
            "should extract SG name: {refs:?}"
        );
    }

    #[test]
    fn todo_interpolation_stripped_from_heredoc() {
        let input = "proxy_pass http://${<TODO: remove AWS resource ref>[0].private_ip}:3000;";
        let out = remove_todo_interpolations(input.to_string());
        assert_eq!(
            out,
            "proxy_pass http://<TODO: remove AWS resource ref>:3000;"
        );
    }

    #[test]
    fn todo_interpolation_with_space_after_brace() {
        let input = "url: '${ <TODO: remove AWS resource ref>}';";
        let out = remove_todo_interpolations(input.to_string());
        assert_eq!(out, "url: '<TODO: remove AWS resource ref>';");
    }

    #[test]
    fn valid_interpolations_are_not_touched() {
        let input = "hostname = \"web-${count.index + 1}\"";
        let out = remove_todo_interpolations(input.to_string());
        assert_eq!(out, input);
    }

    #[test]
    fn multiple_todo_interpolations_all_replaced() {
        let input = "a=${<TODO: remove AWS resource ref>.x} b=${<TODO: remove AWS resource ref>.y}";
        let out = remove_todo_interpolations(input.to_string());
        assert_eq!(
            out,
            "a=<TODO: remove AWS resource ref> b=<TODO: remove AWS resource ref>"
        );
    }
}
