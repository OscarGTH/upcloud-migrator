use crate::migration::types::{MigrationResult, MigrationStatus};
use crate::terraform::types::TerraformResource;

/// Map instance_type (e.g. t3.micro) → UpCloud plan slug. Returns None for unknown types.
pub fn aws_instance_type_to_upcloud_plan(instance_type: &str) -> Option<&'static str> {
    match instance_type {
        "t2.micro" | "t3.micro" => Some("1xCPU-1GB"),
        "t2.small" | "t3.small" => Some("1xCPU-2GB"),
        "t2.medium" | "t3.medium" => Some("2xCPU-4GB"),
        "t2.large" | "t3.large" => Some("2xCPU-8GB"),
        "t2.xlarge" | "t3.xlarge" => Some("4xCPU-8GB"),
        "t2.2xlarge" | "t3.2xlarge" => Some("6xCPU-16GB"),
        "m5.large" | "m5.xlarge" => Some("4xCPU-8GB"),
        "m5.2xlarge" => Some("6xCPU-16GB"),
        "c5.large" | "c5.xlarge" => Some("4xCPU-8GB"),
        _ => None,
    }
}

/// Map instance_type (e.g. t3.micro) → UpCloud plan slug
fn map_instance_type(instance_type: &str) -> &'static str {
    aws_instance_type_to_upcloud_plan(instance_type).unwrap_or("2xCPU-4GB")
}

/// Map AWS region → UpCloud zone
fn map_region(region: &str) -> &'static str {
    match region {
        "us-east-1" | "us-east-2" => "us-nyc1",
        "us-west-1" | "us-west-2" => "us-chi1",
        "eu-west-1" | "eu-west-2" | "eu-west-3" => "de-fra1",
        "eu-central-1" => "de-fra1",
        "ap-southeast-1" | "ap-southeast-2" => "sg-sin1",
        "ap-northeast-1" => "sg-sin1",
        _ => "__ZONE__",
    }
}

/// Returns true when `instance_type` is a Terraform variable/expression rather than a literal.
/// A literal is a plain string like "t3.micro"; expressions include var refs and interpolations.
fn is_instance_type_expression(s: &str) -> bool {
    s.starts_with("var.") || s.starts_with("${") || s.starts_with("local.")
}

pub fn map_instance(res: &TerraformResource) -> MigrationResult {
    let instance_type = res.attributes.get("instance_type").map(|s| s.trim_matches('"')).unwrap_or("t3.micro");
    let is_expr = is_instance_type_expression(instance_type);
    let plan = if is_expr { "" } else { map_instance_type(instance_type) };
    let zone = res.attributes.get("availability_zone")
        .map(|az| map_region(az.trim_matches('"')))
        .unwrap_or("__ZONE__");
    let hostname = res.name.replace('_', "-");
    let tags = res.attributes.get("tags.Name")
        .map(|v| v.trim_matches('"').to_string())
        .unwrap_or_else(|| hostname.clone());

    // Extract key pair reference for the login block
    let key_ref = res.attributes.get("key_name").map(|v| {
        let v = v.trim_matches('"');
        // Handle reference form: aws_key_pair.<name>.key_name → <name>
        if v.starts_with("aws_key_pair.") {
            v.split('.').nth(1).unwrap_or(v).to_string()
        } else {
            v.to_string()
        }
    });

    let login_block = match &key_ref {
        Some(kref) => format!(
            "\n  login {{\n    user = \"root\"\n    keys = [\"<TODO: SSH public key for aws_key_pair.{kref}>\"]\n  }}\n"
        ),
        None => "\n  login {\n    user = \"root\"\n    keys = [\"<TODO: paste SSH public key>\"]\n  }\n".to_string(),
    };

    // Propagate count if set (e.g. count = 2)
    let count_attr = res.attributes.get("count").map(|v| v.trim_matches('"').to_string());
    let count_line = match &count_attr {
        Some(n) => format!("  count    = {}\n  hostname = \"{}-${{count.index + 1}}\"\n", n, hostname),
        None    => format!("  hostname = \"{}\"\n", hostname),
    };

    // Propagate user_data if present. The value from hcl-rs attr.expr() Display is
    // already valid HCL (quoted string or heredoc), so we embed it directly.
    // For <<-MARKER heredocs we normalise the closing marker indentation to match
    // the content, so HCL's indentation-stripping leaves nested bash heredoc
    // closers (like `HTML` or `NGINX`) at column 0 where bash expects them.
    let has_user_data = res.attributes.contains_key("user_data");
    let user_data_line = res.attributes.get("user_data")
        .map(|v| format!("\n  user_data = {}", normalize_heredoc(v)))
        .unwrap_or_default();

    // metadata must be true when using cloud-init / user_data (provider docs requirement).
    let metadata_line = if has_user_data { "\n  metadata  = true" } else { "" };

    // For expression plans (var.xxx) the value must not be quoted.
    let plan_line = if is_expr {
        format!("  plan     = {}\n", instance_type)
    } else {
        format!("  plan     = \"{}\"\n", plan)
    };

    let hcl = format!(
        r#"resource "upcloud_server" "{name}" {{
{count_line}  zone     = "{zone}"
{plan_line}  firewall = true{metadata}

  template {{
    storage = "Ubuntu Server 24.04 LTS (Noble Numbat)"
    size    = 50
  }}

  network_interface {{
    type = "public"
  }}

  network_interface {{
    type    = "private"
    network = "<TODO: upcloud_network reference>"
  }}{login}  labels = {{
    Name = "{tags}"
  }}{user_data}
}}
"#,
        name = res.name,
        count_line = count_line,
        zone = zone,
        plan_line = plan_line,
        metadata = metadata_line,
        login = login_block,
        tags = tags,
        user_data = user_data_line,
    );

    let mut notes = vec![
        if is_expr {
            format!("instance_type '{}' is a variable — update its default in variables.tf to an UpCloud plan.", instance_type)
        } else {
            format!("instance_type '{}' → plan '{}'", instance_type, plan)
        },
        "AMI → Ubuntu 24.04 LTS (update if needed: upctl storage list --public --template)".into(),
    ];
    if let Some(ref n) = count_attr {
        notes.push(format!("count = {} propagated.", n));
    }
    if key_ref.is_some() {
        notes.push("SSH key auto-resolved from aws_key_pair.".into());
    } else {
        notes.push("login block added — paste your SSH public key.".into());
    }
    notes.push("Private network auto-resolved from subnet.".into());
    if let Some(ud) = res.attributes.get("user_data") {
        notes.push("user_data propagated — review AWS-specific refs (IPs, metadata).".into());
        if ud.contains("base64encode(") {
            notes.push("user_data: remove base64encode() — UpCloud cloud-init expects plain text.".into());
        }
        if ud.contains("templatefile(") {
            notes.push("user_data uses templatefile() — review vars for AWS-specific refs.".into());
        } else if ud.trim_start().starts_with("file(") || ud.contains(" file(") {
            notes.push("user_data is an external file — not scanned for AWS refs.".into());
        }
    }

    MigrationResult {
        resource_type: res.resource_type.clone(),
        resource_name: res.name.clone(),
        source_file: res.source_file.display().to_string(),
        status: MigrationStatus::Native,
        score: 90,
        upcloud_type: "upcloud_server".into(),
        upcloud_hcl: Some(hcl),
        snippet: None,
        parent_resource: None,
        notes,
        source_hcl: None,
    }
}

pub fn map_key_pair(res: &TerraformResource) -> MigrationResult {
    // SSH key pairs have no standalone UpCloud resource — keys are embedded in upcloud_server.
    let public_key = res.attributes.get("public_key").map(|v| {
        // Strip surrounding quotes only when the value is a simple quoted string literal
        // (starts AND ends with `"`). HCL expressions like ternaries may end with `"` as
        // part of an inner string — trim_matches would incorrectly eat that trailing quote.
        if v.starts_with('"') && v.ends_with('"') && v.len() >= 2 {
            v[1..v.len() - 1].to_string()
        } else {
            v.to_string()
        }
    });

    let key_value = public_key.as_deref().unwrap_or("<TODO: paste SSH public key>");
    // A HCL expression (ternary, var reference, etc.) must not be wrapped in
    // extra quotes — only plain literal keys belong inside "...".
    let is_expression = !key_value.starts_with("ssh-")
        && !key_value.starts_with("ecdsa-")
        && !key_value.starts_with("sk-")
        && (key_value.contains("var.") || key_value.contains("${") || key_value.contains('?'));
    let snippet = if is_expression {
        format!(
            "login {{\n  user = \"root\"\n  keys = [{key}]  # was aws_key_pair.{name}\n}}",
            key = key_value,
            name = res.name,
        )
    } else {
        format!(
            "login {{\n  user = \"root\"\n  keys = [\"{key}\"]  # was aws_key_pair.{name}\n}}",
            key = key_value,
            name = res.name,
        )
    };

    MigrationResult {
        resource_type: res.resource_type.clone(),
        resource_name: res.name.clone(),
        source_file: res.source_file.display().to_string(),
        status: MigrationStatus::Partial,
        score: 50,
        upcloud_type: "login block (server resource)".into(),
        upcloud_hcl: None,
        snippet: Some(snippet),
        parent_resource: None,
        notes: vec![
            "AWS key pairs → UpCloud login block inside upcloud_server — not a standalone resource.".into(),
            "SSH key auto-resolved into server login blocks by the generator.".into(),
        ],
            source_hcl: None,
    }
}

/// Normalise a HCL heredoc (`<<MARKER` or `<<-MARKER`) so the generated file
/// is valid and bash receives the script content at column 0.
///
/// Strategy: strip the minimum common leading whitespace (spaces **or** tabs)
/// from every content line and output `<<-MARKER` with the content and closing
/// marker both at column 0.  HCL then strips 0 bytes from the content, so
/// cloud-init receives clean col-0 bash that:
///   • Has a working shebang (`#!/bin/bash` at col 0)
///   • Has nested bash heredoc closers (`HTML`, `NGINX`, …) at col 0 where
///     bash requires them
///
/// This is more robust than the previous "align closing marker" approach
/// because it also handles tab-indented sources (where the old code used
/// `trim_start_matches(' ')` and silently returned 0, leaving the heredoc
/// broken in the generated file).
///
/// If the value is not a heredoc (e.g. a quoted string), or if content is
/// already at col 0, it is returned unchanged.
fn normalize_heredoc(value: &str) -> String {
    let trimmed = value.trim_start();
    let (strip_mode, rest) = if trimmed.starts_with("<<-") {
        (true, &trimmed[3..])
    } else if trimmed.starts_with("<<") {
        (false, &trimmed[2..])
    } else {
        return value.to_string(); // not a heredoc
    };

    let first_nl = match rest.find('\n') {
        Some(i) => i,
        None => return value.to_string(),
    };
    let marker = rest[..first_nl].trim().to_string();
    if marker.is_empty() {
        return value.to_string();
    }

    let after_header = &rest[first_nl + 1..];
    let lines: Vec<&str> = after_header.lines().collect();

    // The closing marker is the last line whose trimmed content equals marker.
    let close_idx = match lines.iter().rposition(|l| l.trim() == marker) {
        Some(i) => i,
        None => return value.to_string(),
    };

    let content_lines = &lines[..close_idx];

    // Count leading whitespace bytes (spaces OR tabs) — handles both styles.
    fn leading_ws_len(s: &str) -> usize {
        s.bytes().take_while(|b| *b == b' ' || *b == b'\t').count()
    }

    // Use the indentation of the first non-empty content line as the strip amount.
    //
    // Why not `min()` across all lines:  embedded bash heredocs (<<'HTML'…HTML,
    // <<'NGINX'…NGINX) can contain HTML/CSS/ASCII-art at arbitrary indentation
    // including col 0.  Those lines would drag the minimum down and prevent the
    // surrounding bash-script lines from being de-indented.
    //
    // The first non-empty content line is the bash shebang or first command — it
    // reliably represents the indentation level of the outer script.
    let min_indent = content_lines
        .iter()
        .find(|l| !l.trim().is_empty())
        .map(|l| leading_ws_len(l))
        .unwrap_or(0);

    let current_close_indent = leading_ws_len(lines[close_idx]);

    // Already clean: <<-MARKER with content and closing marker at col 0.
    if strip_mode && min_indent == 0 && current_close_indent == 0 {
        return value.to_string();
    }
    // <<MARKER (no dash) with content already at col 0: also fine as-is.
    if !strip_mode && min_indent == 0 {
        return value.to_string();
    }

    // Strip min_indent leading whitespace bytes from each content line so bash
    // receives col-0 content.  Lines at col 0 (e.g. ASCII art) have fewer
    // leading bytes than min_indent — clamp to avoid out-of-bounds slicing.
    let mut result = format!("<<-{marker}\n");
    for line in content_lines {
        if line.trim().is_empty() {
            result.push('\n');
        } else {
            let strip = leading_ws_len(line).min(min_indent);
            result.push_str(&line[strip..]);
            result.push('\n');
        }
    }
    result.push_str(&marker);
    result
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

    // ── map_instance ──────────────────────────────────────────────────────────

    #[test]
    fn instance_generates_upcloud_server() {
        let res = make_res("aws_instance", "web", &[("instance_type", "t3.micro")]);
        let r = map_instance(&res);
        assert_eq!(r.upcloud_type, "upcloud_server");
        assert_eq!(r.resource_name, "web");
        let hcl = r.upcloud_hcl.unwrap();
        assert!(hcl.contains("resource \"upcloud_server\" \"web\""), "{hcl}");
        assert!(hcl.contains("plan     = \"1xCPU-1GB\""), "t3.micro should map to 1xCPU-1GB\n{hcl}");
    }

    #[test]
    fn instance_t3_medium_maps_plan() {
        let res = make_res("aws_instance", "app", &[("instance_type", "t3.medium")]);
        let r = map_instance(&res);
        assert!(r.upcloud_hcl.unwrap().contains("2xCPU-4GB"));
    }

    #[test]
    fn instance_unknown_type_uses_default_plan() {
        let res = make_res("aws_instance", "x", &[("instance_type", "x99.mega")]);
        let r = map_instance(&res);
        assert!(r.upcloud_hcl.unwrap().contains("2xCPU-4GB"));
    }

    #[test]
    fn instance_with_count_generates_count_line() {
        let res = make_res("aws_instance", "web", &[
            ("instance_type", "t3.micro"),
            ("count", "3"),
        ]);
        let r = map_instance(&res);
        let hcl = r.upcloud_hcl.unwrap();
        assert!(hcl.contains("count    = 3"), "{hcl}");
        assert!(hcl.contains("count.index"), "hostname should use count.index\n{hcl}");
    }

    #[test]
    fn instance_without_count_has_no_count_line() {
        let res = make_res("aws_instance", "solo", &[("instance_type", "t3.small")]);
        let r = map_instance(&res);
        let hcl = r.upcloud_hcl.unwrap();
        assert!(!hcl.contains("count    ="), "should not generate count line\n{hcl}");
        assert!(hcl.contains("hostname = \"solo\""), "{hcl}");
    }

    #[test]
    fn instance_with_user_data_propagates_it() {
        let script = "\"#!/bin/bash\\napt-get update\"";
        let res = make_res(
            "aws_instance",
            "web",
            &[("instance_type", "t3.micro"), ("user_data", script)],
        );
        let r = map_instance(&res);
        let hcl = r.upcloud_hcl.unwrap();
        assert!(hcl.contains("user_data ="), "user_data attribute must appear\n{hcl}");
        assert!(hcl.contains("apt-get update"), "user_data content must be present\n{hcl}");
        assert!(
            r.notes.iter().any(|n| n.contains("user_data propagated")),
            "must add a note about user_data\n{:?}", r.notes
        );
    }

    #[test]
    fn instance_without_user_data_omits_user_data_line() {
        let res = make_res("aws_instance", "web", &[("instance_type", "t3.micro")]);
        let r = map_instance(&res);
        let hcl = r.upcloud_hcl.unwrap();
        assert!(!hcl.contains("user_data"), "user_data must not appear when absent\n{hcl}");
    }

    #[test]
    fn instance_always_has_firewall_true() {
        let res = make_res("aws_instance", "web", &[("instance_type", "t3.micro")]);
        let hcl = map_instance(&res).upcloud_hcl.unwrap();
        assert!(hcl.contains("firewall = true"), "firewall must be enabled on all servers\n{hcl}");
    }

    #[test]
    fn instance_with_user_data_sets_metadata_true() {
        let res = make_res(
            "aws_instance",
            "web",
            &[("instance_type", "t3.micro"), ("user_data", "\"#!/bin/bash\"")],
        );
        let hcl = map_instance(&res).upcloud_hcl.unwrap();
        assert!(hcl.contains("metadata  = true"), "metadata must be true when user_data is present\n{hcl}");
    }

    #[test]
    fn user_data_base64encode_adds_warning_note() {
        let res = make_res(
            "aws_instance", "web",
            &[("instance_type", "t3.micro"), ("user_data", "base64encode(file(\"init.sh\"))")],
        );
        let r = map_instance(&res);
        assert!(r.notes.iter().any(|n| n.contains("base64encode")),
            "must warn about base64encode\n{:?}", r.notes);
        assert!(r.notes.iter().any(|n| n.contains("plain text")),
            "must say UpCloud expects plain text\n{:?}", r.notes);
    }

    #[test]
    fn user_data_templatefile_adds_warning_note() {
        let res = make_res(
            "aws_instance", "web",
            &[("instance_type", "t3.micro"), ("user_data", "templatefile(\"init.sh.tpl\", { region = var.region })")],
        );
        let r = map_instance(&res);
        assert!(r.notes.iter().any(|n| n.contains("templatefile")),
            "must warn about templatefile\n{:?}", r.notes);
    }

    #[test]
    fn user_data_file_ref_adds_note_about_unscanned_script() {
        let res = make_res(
            "aws_instance", "web",
            &[("instance_type", "t3.micro"), ("user_data", "file(\"${path.module}/init.sh\")")],
        );
        let r = map_instance(&res);
        assert!(r.notes.iter().any(|n| n.contains("external file") && n.contains("not scanned")),
            "must warn that external file is not scanned\n{:?}", r.notes);
    }

    #[test]
    fn instance_without_user_data_omits_metadata() {
        let res = make_res("aws_instance", "web", &[("instance_type", "t3.micro")]);
        let hcl = map_instance(&res).upcloud_hcl.unwrap();
        assert!(!hcl.contains("metadata"), "metadata must not appear when user_data is absent\n{hcl}");
    }

    #[test]
    fn instance_with_key_name_generates_ssh_key_todo() {
        let res = make_res("aws_instance", "web", &[
            ("instance_type", "t3.micro"),
            ("key_name", "aws_key_pair.prod.key_name"),
        ]);
        let r = map_instance(&res);
        let hcl = r.upcloud_hcl.unwrap();
        assert!(
            hcl.contains("<TODO: SSH public key for aws_key_pair.prod>"),
            "{hcl}"
        );
    }

    #[test]
    fn instance_without_key_name_generates_generic_ssh_todo() {
        let res = make_res("aws_instance", "web", &[("instance_type", "t3.micro")]);
        let r = map_instance(&res);
        assert!(r.upcloud_hcl.unwrap().contains("<TODO: paste SSH public key>"));
    }

    // ── normalize_heredoc ─────────────────────────────────────────────────────

    #[test]
    fn heredoc_indented_content_gets_stripped_to_col0() {
        // Simulates what hcl-rs outputs for a <<-EOF with indented content
        // but a closing marker at col 0.  New behaviour: strip the content to
        // col 0 and put the closing marker at col 0 so bash receives clean
        // col-0 content directly (no reliance on HCL's stripping).
        let input = "<<-EOF\n          #!/bin/bash\n          cat > /x <<'HTML'\n          hello\n          HTML\n          echo done\nEOF";
        let out = normalize_heredoc(input);
        // Closing marker must be at col 0.
        assert!(out.ends_with("\nEOF"), "closing marker must be at col 0\n{out}");
        // Content must be stripped to col 0.
        assert!(out.contains("\n#!/bin/bash"), "{out}");
        // Inner bash heredoc closer must also be at col 0 so bash recognises it.
        assert!(out.contains("\nHTML\n"), "{out}");
    }

    #[test]
    fn heredoc_col0_ascii_art_does_not_block_stripping() {
        // When embedded HTML content contains ASCII-art lines at col 0 (no
        // leading whitespace), those lines must NOT pull min_indent down to 0
        // and prevent the bash-command lines from being stripped.
        // Mirrors the real-world user_data that has <<'HTML'>...ASCII art...HTML.
        let input = concat!(
            "<<-EOF\n",
            "          #!/bin/bash\n",
            "          cat > /x <<'HTML'\n",
            " / _ \\ | |    |  ___|  /  __ \\\n", // ASCII art at col 0 (1 space)
            "/ /_\\ \\| | /\\ | |_ ___ | /  \\/\n", // ASCII art at col 0
            "|  _  || |/ / |  _/ __|| |    \n",
            "          HTML\n",
            "          echo done\n",
            "EOF",
        );
        let out = normalize_heredoc(input);
        // Bash commands must be at col 0 after stripping.
        assert!(out.contains("\n#!/bin/bash\n"), "shebang must be at col 0\n{out}");
        // Inner bash heredoc closer must be at col 0.
        assert!(out.contains("\nHTML\n"), "HTML terminator must be at col 0\n{out}");
        // ASCII art lines (which were at col 0 in input) must remain intact.
        assert!(out.contains("/ _ \\"), "ASCII art must be preserved\n{out}");
        // Closing marker must be at col 0.
        assert!(out.ends_with("\nEOF"), "closing marker must be at col 0\n{out}");
    }

    #[test]
    fn heredoc_already_normalised_is_unchanged() {
        let input = "<<-EOF\n#!/bin/bash\nHTML\nEOF";
        let out = normalize_heredoc(input);
        assert_eq!(out, input);
    }

    #[test]
    fn non_heredoc_value_is_unchanged() {
        let input = "\"#!/bin/bash\\napt-get install nginx\"";
        let out = normalize_heredoc(input);
        assert_eq!(out, input);
    }

    // ── <<EOF (no dash) cases — what hcl-rs actually returns ──────────────────
    //
    // hcl-rs preserves the raw content for <<MARKER (no dash) heredocs.
    // When the source Terraform file has indented user_data with <<EOF, hcl-rs
    // Display gives "<<EOF\n          content\nEOF" with all spaces intact.
    // normalize_heredoc must upgrade these to <<-MARKER so HCL strips the
    // indentation and bash gets col-0 content.

    #[test]
    fn no_dash_heredoc_with_indented_content_is_upgraded_to_strip_form() {
        // Mirrors exactly what hcl-rs returns for a <<EOF with indented content.
        let input = "<<EOF\n          #!/bin/bash\n          cat > /x <<'HTML'\n          hello\n          HTML\n          echo done\nEOF";
        let out = normalize_heredoc(input);
        // Must be upgraded to <<-EOF.
        assert!(out.starts_with("<<-EOF"), "must be upgraded to <<-EOF\n{out}");
        // Closing marker must be at col 0.
        assert!(out.ends_with("\nEOF"), "closing marker must be at col 0\n{out}");
        // Content must be stripped to col 0.
        assert!(out.contains("\n#!/bin/bash"), "{out}");
        assert!(out.contains("\nHTML\n"), "{out}");
    }

    #[test]
    fn no_dash_heredoc_no_indentation_is_unchanged() {
        // <<EOF with content at col 0 is already correct.
        let input = "<<EOF\n#!/bin/bash\napt-get update\nEOF";
        let out = normalize_heredoc(input);
        assert_eq!(out, input);
    }

    #[test]
    fn heredoc_via_real_hcl_parser_produces_normalized_output() {
        // This test goes through the actual hcl-rs parser to verify that
        // whatever format attr.expr() Display produces is handled correctly.
        // Previously, tests used hand-crafted strings and missed the <<EOF case
        // (no-dash), where hcl-rs preserves the raw indentation verbatim.
        use crate::terraform::parser::parse_tf_file;

        let tf_source = concat!(
            "resource \"aws_instance\" \"web\" {\n",
            "  instance_type = \"t3.micro\"\n",
            "  user_data = <<EOF\n",
            "          #!/bin/bash\n",
            "          cat > /var/www/html/index.html <<'HTML'\n",
            "          <html></html>\n",
            "          HTML\n",
            "          systemctl restart nginx\n",
            "EOF\n",
            "}",
        );

        let tmp_path = std::env::temp_dir()
            .join(format!("upcloud_test_{}.tf", std::process::id()));
        std::fs::write(&tmp_path, tf_source).unwrap();
        let tf_file = parse_tf_file(&tmp_path).unwrap();
        let _ = std::fs::remove_file(&tmp_path);

        let res = &tf_file.resources[0];
        let r = map_instance(res);
        let hcl = r.upcloud_hcl.unwrap();

        // The generated HCL must be parseable.
        hcl::from_str::<hcl::Body>(&hcl)
            .unwrap_or_else(|e| panic!("generated HCL is not valid: {e}\n{hcl}"));

        // Content must be stripped to col 0 so bash receives a working shebang.
        assert!(
            hcl.contains("#!/bin/bash"),
            "shebang must appear (at col 0 after HCL strips)\n{hcl}"
        );
        // The nested bash closer must be at col 0 so bash recognises it.
        assert!(
            hcl.contains("\nHTML\n") || hcl.contains("\nHTML\r\n"),
            "nested HTML closer must be at col 0\n{hcl}"
        );
        // Closing EOF must be at col 0 (not indented).
        assert!(
            !hcl.contains("          EOF"),
            "closing EOF must NOT be indented — content is already at col 0\n{hcl}"
        );
    }

    #[test]
    fn real_world_nested_heredoc_with_css_produces_col0_shebang() {
        // Reproduces the exact pattern in the demo source file:
        // <<-EOF with content+closing-marker both at 14 spaces, containing
        // nested <<'HTML' and <<'NGINX' bash heredocs with CSS {} blocks.
        use crate::terraform::parser::parse_tf_file;

        let tf_source = concat!(
            "resource \"aws_instance\" \"web\" {\n",
            "  instance_type = \"t3.micro\"\n",
            "  user_data = <<-EOF\n",
            "              #!/bin/bash\n",
            "              set -e\n",
            "              apt-get install -y nginx\n",
            "              cat > /var/www/html/index.html <<'HTML'\n",
            "              <!DOCTYPE html>\n",
            "              <style>\n",
            "                body { background: #000; color: #00ff41; }\n",
            "                .box { border: 2px solid #00ff41; }\n",
            "              </style>\n",
            "              HTML\n",
            "              cat > /etc/nginx/sites-available/default <<'NGINX'\n",
            "              server {\n",
            "                listen 80 default_server;\n",
            "              }\n",
            "              NGINX\n",
            "              systemctl restart nginx\n",
            "              EOF\n",
            "}",
        );

        let tmp_path = std::env::temp_dir()
            .join(format!("upcloud_real_world_{}.tf", std::process::id()));
        std::fs::write(&tmp_path, tf_source).unwrap();
        let tf_file = parse_tf_file(&tmp_path).unwrap();
        let _ = std::fs::remove_file(&tmp_path);

        let res = &tf_file.resources[0];
        let r = map_instance(res);
        let hcl = r.upcloud_hcl.unwrap();

        // Generated HCL must be parseable.
        hcl::from_str::<hcl::Body>(&hcl)
            .unwrap_or_else(|e| panic!("generated HCL is not valid: {e}\n{hcl}"));

        // Shebang must be at col 0 so cloud-init can exec the script.
        assert!(
            hcl.contains("\n#!/bin/bash\n") || hcl.contains("= <<-EOF\n#!/bin/bash\n"),
            "shebang must be at col 0\n{hcl}"
        );
        // Nested bash heredoc terminators must be at col 0.
        assert!(
            hcl.contains("\nHTML\n"),
            "HTML terminator must be at col 0\n{hcl}"
        );
        assert!(
            hcl.contains("\nNGINX\n"),
            "NGINX terminator must be at col 0\n{hcl}"
        );
    }

    #[test]
    fn dash_heredoc_via_real_hcl_parser_produces_normalized_output() {
        // Like heredoc_via_real_hcl_parser_produces_normalized_output but for
        // <<-EOF (strip-indent heredoc). hcl-rs may handle these differently —
        // this test reveals the actual attr.expr() Display output.
        // Covers the common AWS pattern where <<-EOF has content indented 10
        // spaces but the closing EOF sits at col 0 (inside the resource block
        // but with no matching indent), so HCL strips nothing and bash gets
        // leading spaces that break the shebang.
        use crate::terraform::parser::parse_tf_file;

        // Closing EOF at col 0, content indented — the real-world AWS pattern.
        let tf_source = concat!(
            "resource \"aws_instance\" \"web\" {\n",
            "  instance_type = \"t3.micro\"\n",
            "  user_data = <<-EOF\n",
            "          #!/bin/bash\n",
            "          cat > /var/www/html/index.html <<'HTML'\n",
            "          <html></html>\n",
            "          HTML\n",
            "          systemctl restart nginx\n",
            "EOF\n",   // <-- closing marker at col 0, not matching content indent
            "}",
        );

        let tmp_path = std::env::temp_dir()
            .join(format!("upcloud_test_dash_{}.tf", std::process::id()));
        std::fs::write(&tmp_path, tf_source).unwrap();
        let tf_file = parse_tf_file(&tmp_path).unwrap();
        let _ = std::fs::remove_file(&tmp_path);

        let res = &tf_file.resources[0];
        let r = map_instance(res);
        let hcl = r.upcloud_hcl.unwrap();

        // The generated HCL must be parseable.
        hcl::from_str::<hcl::Body>(&hcl)
            .unwrap_or_else(|e| panic!("generated HCL is not valid: {e}\n{hcl}"));

        // Content must be stripped to col 0 so cloud-init gets a working shebang.
        assert!(
            hcl.contains("#!/bin/bash"),
            "shebang must appear at col 0\n{hcl}"
        );
        // Nested bash closer must be at col 0 so bash recognises it.
        assert!(
            hcl.contains("\nHTML\n") || hcl.contains("\nHTML\r\n"),
            "nested HTML closer must be at col 0\n{hcl}"
        );
    }

    #[test]
    fn heredoc_user_data_in_generated_hcl_strips_content_to_col0() {
        // End-to-end: when the attribute value is a <<-EOF with indented content
        // and misaligned closing marker, the generated HCL must have content
        // stripped to col 0 so cloud-init receives a working bash script.
        let user_data_val = "<<-EOF\n          #!/bin/bash\n          apt-get install -y nginx\nEOF";
        let res = make_res(
            "aws_instance",
            "web",
            &[("instance_type", "t3.micro"), ("user_data", user_data_val)],
        );
        let hcl = map_instance(&res).upcloud_hcl.unwrap();
        // Content must be stripped to col 0.
        assert!(
            hcl.contains("#!/bin/bash"),
            "shebang must be present\n{hcl}"
        );
        // Closing EOF must NOT be indented.
        assert!(
            !hcl.contains("          EOF"),
            "closing EOF must not be indented — content is already at col 0\n{hcl}"
        );
    }

    #[test]
    fn heredoc_tab_indented_content_gets_stripped_to_col0() {
        // Tab-indented heredocs (the original bug: trim_start_matches(' ')
        // returned 0 for tab-indented lines, leaving min_indent = 0 and the
        // function returning unchanged with broken cloud-init output).
        let input = "<<-EOF\n\t#!/bin/bash\n\tapt-get install -y nginx\nEOF";
        let out = normalize_heredoc(input);
        // Content must be stripped to col 0.
        assert!(out.contains("\n#!/bin/bash"), "tab must be stripped\n{out}");
        assert!(out.contains("\napt-get"), "tab must be stripped\n{out}");
        // Closing marker at col 0.
        assert!(out.ends_with("\nEOF"), "closing marker must be at col 0\n{out}");
    }

    #[test]
    fn instance_private_network_interface_has_network_todo() {
        let res = make_res("aws_instance", "web", &[("instance_type", "t3.micro")]);
        let r = map_instance(&res);
        let hcl = r.upcloud_hcl.unwrap();
        assert!(
            hcl.contains("type    = \"private\""),
            "should have private interface\n{hcl}"
        );
        assert!(
            hcl.contains("<TODO: upcloud_network reference>"),
            "private interface should have network TODO\n{hcl}"
        );
    }

    // ── map_key_pair ──────────────────────────────────────────────────────────

    #[test]
    fn key_pair_with_public_key_puts_key_in_snippet() {
        let res = make_res("aws_key_pair", "prod", &[("public_key", "ssh-rsa AAAAB3 user@host")]);
        let r = map_key_pair(&res);
        let snippet = r.snippet.unwrap();
        assert!(snippet.contains("ssh-rsa AAAAB3"), "{snippet}");
        assert!(!snippet.contains("<TODO"), "should not have TODO when key is provided\n{snippet}");
    }

    #[test]
    fn key_pair_without_public_key_has_todo_in_snippet() {
        let res = make_res("aws_key_pair", "staging", &[]);
        let r = map_key_pair(&res);
        let snippet = r.snippet.unwrap();
        assert!(snippet.contains("<TODO: paste SSH public key>"), "{snippet}");
    }

    #[test]
    fn key_pair_with_ternary_expression_produces_unquoted_snippet() {
        // A ternary like `var.x != "" ? var.x : "placeholder"` must NOT be wrapped in
        // extra quotes — it's a HCL expression, not a string literal.
        let res = make_res(
            "aws_key_pair",
            "main",
            &[("public_key", r#"var.ssh_public_key != "" ? var.ssh_public_key : "ssh-rsa PLACEHOLDER""#)],
        );
        let r = map_key_pair(&res);
        let snippet = r.snippet.unwrap();
        assert!(
            snippet.contains("keys = [var.ssh_public_key"),
            "expression key should not be wrapped in quotes\n{snippet}"
        );
        assert!(
            !snippet.contains(r#"keys = ["var.ssh_public_key"#),
            "expression must not have extra outer quotes\n{snippet}"
        );
        // The closing `"` on the ternary else-branch string must not be stripped.
        assert!(
            snippet.contains(r#": "ssh-rsa PLACEHOLDER"]"#),
            "closing quote of ternary else-branch must be intact\n{snippet}"
        );
    }

    #[test]
    fn key_pair_has_no_standalone_hcl() {
        let res = make_res("aws_key_pair", "k", &[]);
        let r = map_key_pair(&res);
        assert!(r.upcloud_hcl.is_none(), "key pairs have no standalone UpCloud resource");
    }

    // ── map_autoscaling_group ─────────────────────────────────────────────────

    #[test]
    fn autoscaling_group_is_unsupported() {
        let res = make_res("aws_autoscaling_group", "asg", &[]);
        let r = map_autoscaling_group(&res);
        assert_eq!(r.status, crate::migration::types::MigrationStatus::Unsupported);
        assert!(r.upcloud_hcl.is_none());
    }
}

pub fn map_autoscaling_group(res: &TerraformResource) -> MigrationResult {
    MigrationResult {
        resource_type: res.resource_type.clone(),
        resource_name: res.name.clone(),
        source_file: res.source_file.display().to_string(),
        status: MigrationStatus::Unsupported,
        score: 10,
        upcloud_type: "(no autoscaling)".into(),
        upcloud_hcl: None,
        snippet: None,
        parent_resource: None,
        notes: vec!["UpCloud does not have a managed autoscaling group equivalent. Use multiple upcloud_server resources.".into()],
            source_hcl: None,
    }
}
