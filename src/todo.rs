use std::path::Path;

#[derive(Debug, Clone, PartialEq)]
pub enum TodoStatus {
    Pending,
    Loading,
    Resolved,
    Skipped,
}

#[derive(Debug, Clone)]
pub struct TodoItem {
    /// Output filename (e.g. "main.tf")
    pub file: String,
    /// 1-based line number
    pub line_no: usize,
    /// The placeholder text, e.g. `<TODO: base64 encoded certificate>`
    pub placeholder: String,
    /// Full line content containing the placeholder
    pub line_content: String,
    /// Surrounding lines for context
    pub context: Vec<String>,
    pub status: TodoStatus,
    /// Value the user typed or accepted
    pub resolution: Option<String>,
    /// Value suggested by AI
    pub ai_suggestion: Option<String>,
}

/// Scan all .tf files in `output_dir` for remaining `<TODO...>` markers.
/// Only matches real HCL value lines, skips HCL comment lines (starting with `#`).
pub fn scan_output_todos(output_dir: &Path) -> Vec<TodoItem> {
    let mut todos = Vec::new();

    let entries = walkdir::WalkDir::new(output_dir)
        .max_depth(2)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().map(|x| x == "tf").unwrap_or(false));

    for entry in entries {
        let path = entry.path();
        let content = match std::fs::read_to_string(path) {
            Ok(c) => c,
            Err(_) => continue,
        };

        let filename = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unknown.tf")
            .to_string();

        let lines: Vec<&str> = content.lines().collect();
        for (i, line) in lines.iter().enumerate() {
            let trimmed = line.trim_start();
            if trimmed.starts_with('#') {
                continue;
            }
            {
                let key = trimmed.split('=').next().unwrap_or("").trim();
                if key == "comment" {
                    continue;
                }
            }

            if !line.contains("<TODO") {
                continue;
            }

            let placeholder = extract_placeholder(line);

            let start = i.saturating_sub(2);
            let end = (i + 3).min(lines.len());
            let context: Vec<String> = lines[start..end].iter().map(|l| l.to_string()).collect();

            todos.push(TodoItem {
                file: filename.clone(),
                line_no: i + 1,
                placeholder,
                line_content: line.to_string(),
                context,
                status: TodoStatus::Pending,
                resolution: None,
                ai_suggestion: None,
            });
        }
    }

    todos
}

/// Apply a resolution to the specific line in the output file (by line number).
/// Only replaces the first occurrence of the placeholder on `item.line_no`.
pub fn apply_resolution(
    output_dir: &Path,
    item: &TodoItem,
    resolution: &str,
) -> anyhow::Result<()> {
    let path = output_dir.join(&item.file);
    let content = std::fs::read_to_string(&path)?;
    let trailing_newline = content.ends_with('\n');
    let mut lines: Vec<String> = content.lines().map(|l| l.to_string()).collect();
    if let Some(line) = lines.get_mut(item.line_no.saturating_sub(1)) {
        *line = line.replacen(&item.placeholder, resolution, 1);
    }
    let mut updated = lines.join("\n");
    if trailing_newline {
        updated.push('\n');
    }
    std::fs::write(&path, updated)?;
    Ok(())
}

fn extract_placeholder(line: &str) -> String {
    // Match <TODO: ...> or <TODO>
    if let Some(start) = line.find("<TODO") {
        if let Some(end) = line[start..].find('>') {
            return line[start..start + end + 1].to_string();
        }
    }
    "<TODO>".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn make_temp_dir() -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "upcloud_todo_test_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .subsec_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn cleanup(dir: &std::path::Path) {
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn test_extract_placeholder_angle_bracket() {
        let line = r#"  server_id = upcloud_server.<TODO>.id"#;
        assert_eq!(extract_placeholder(line), "<TODO>");
    }

    #[test]
    fn test_extract_placeholder_with_description() {
        let line = r#"  network_uuid = "<TODO: reference upcloud_network UUID>""#;
        assert_eq!(
            extract_placeholder(line),
            "<TODO: reference upcloud_network UUID>"
        );
    }

    #[test]
    fn test_extract_placeholder_no_closing_bracket() {
        // If there's no closing >, fall back to default
        let line = r#"  something = "<TODO: unclosed"#;
        assert_eq!(extract_placeholder(line), "<TODO>");
    }

    #[test]
    fn test_scan_skips_comment_lines() {
        let dir = make_temp_dir();
        let content = r#"# aws_security_group main (score 75/100)
# NOTE: Replace <TODO> with the upcloud_server resource name.
resource "upcloud_firewall_rules" "main" {
  server_id = upcloud_server.<TODO>.id
}
"#;
        fs::write(dir.join("network.tf"), content).unwrap();
        let todos = scan_output_todos(&dir);
        assert_eq!(todos.len(), 1, "should only match value line, not comment");
        assert_eq!(todos[0].line_no, 4);
        assert!(todos[0].line_content.contains("server_id"));
        cleanup(&dir);
    }

    #[test]
    fn test_scan_finds_todo_in_value() {
        let dir = make_temp_dir();
        let content = r#"resource "upcloud_loadbalancer" "main" {
  network_uuid = "<TODO: reference upcloud_network UUID>"
  zone         = "de-fra1"
}
"#;
        fs::write(dir.join("lb.tf"), content).unwrap();
        let todos = scan_output_todos(&dir);
        assert_eq!(todos.len(), 1);
        assert_eq!(todos[0].placeholder, "<TODO: reference upcloud_network UUID>");
        assert_eq!(todos[0].line_no, 2);
        cleanup(&dir);
    }

    #[test]
    fn test_scan_multiple_files() {
        let dir = make_temp_dir();
        fs::write(
            dir.join("a.tf"),
            "server_id = upcloud_server.<TODO>.id\n",
        )
        .unwrap();
        fs::write(
            dir.join("b.tf"),
            "network = \"<TODO: network id>\"\n# comment <TODO> not this\n",
        )
        .unwrap();
        let mut todos = scan_output_todos(&dir);
        todos.sort_by_key(|t| t.file.clone());
        assert_eq!(todos.len(), 2);
        assert_eq!(todos[0].file, "a.tf");
        assert_eq!(todos[1].file, "b.tf");
        cleanup(&dir);
    }

    #[test]
    fn test_scan_no_false_positives_in_pure_comment_file() {
        let dir = make_temp_dir();
        let content = r#"# This file has <TODO> only in comments
# NOTE: Replace <TODO: something> manually
# TODO: this is a plain comment todo
resource "upcloud_server" "main" {
  hostname = "my-server"
}
"#;
        fs::write(dir.join("server.tf"), content).unwrap();
        let todos = scan_output_todos(&dir);
        assert_eq!(todos.len(), 0, "no non-comment TODO markers present");
        cleanup(&dir);
    }


    #[test]
    fn test_apply_resolution_replaces_placeholder() {
        let dir = make_temp_dir();
        let content = r#"resource "upcloud_firewall_rules" "sg" {
  server_id = upcloud_server.<TODO>.id
}
"#;
        fs::write(dir.join("fw.tf"), content).unwrap();

        let item = TodoItem {
            file: "fw.tf".into(),
            line_no: 2,
            placeholder: "<TODO>".into(),
            line_content: "  server_id = upcloud_server.<TODO>.id".into(),
            context: vec![],
            status: TodoStatus::Pending,
            resolution: None,
            ai_suggestion: None,
        };

        apply_resolution(&dir, &item, "main").unwrap();
        let updated = fs::read_to_string(dir.join("fw.tf")).unwrap();
        assert!(
            updated.contains("server_id = upcloud_server.main.id"),
            "placeholder should be replaced: {updated}"
        );
        assert!(
            !updated.contains("<TODO>"),
            "no TODO left: {updated}"
        );
        cleanup(&dir);
    }

    #[test]
    fn test_apply_resolution_only_on_correct_line() {
        let dir = make_temp_dir();
        // Two identical TODO patterns on different lines
        let content = "  a = \"<TODO: val>\"\n  b = \"<TODO: val>\"\n";
        fs::write(dir.join("multi.tf"), content).unwrap();

        // Resolve only line 1
        let item = TodoItem {
            file: "multi.tf".into(),
            line_no: 1,
            placeholder: "<TODO: val>".into(),
            line_content: "  a = \"<TODO: val>\"".into(),
            context: vec![],
            status: TodoStatus::Pending,
            resolution: None,
            ai_suggestion: None,
        };
        apply_resolution(&dir, &item, "resolved").unwrap();
        let updated = fs::read_to_string(dir.join("multi.tf")).unwrap();
        let lines: Vec<&str> = updated.lines().collect();
        assert!(lines[0].contains("resolved"), "line 1 should be resolved");
        assert!(lines[1].contains("<TODO: val>"), "line 2 should be unchanged");
        cleanup(&dir);
    }


    #[test]
    fn test_scan_login_block_ssh_key_todo() {
        // compute.rs: login block with unresolved SSH key
        let dir = make_temp_dir();
        let content = r#"resource "upcloud_server" "web" {
  hostname = "web"
  zone     = "de-fra1"
  plan     = "1xCPU-2GB"

  login {
    user = "root"
    keys = ["<TODO: paste SSH public key>"]
  }
}
"#;
        fs::write(dir.join("main.tf"), content).unwrap();
        let todos = scan_output_todos(&dir);
        assert_eq!(todos.len(), 1, "login block SSH key TODO should be detected");
        assert_eq!(todos[0].placeholder, "<TODO: paste SSH public key>");
        assert!(todos[0].line_content.contains("keys"));
        cleanup(&dir);
    }

    #[test]
    fn test_scan_acm_certificate_todos() {
        // loadbalancer.rs: ACM cert has two unresolvable TODOs
        let dir = make_temp_dir();
        let content = r#"resource "upcloud_loadbalancer_manual_certificate_bundle" "cert" {
  name        = "cert"
  certificate = "<TODO: base64 encoded certificate>"
  private_key = "<TODO: base64 encoded private key>"
}
"#;
        fs::write(dir.join("lb.tf"), content).unwrap();
        let todos = scan_output_todos(&dir);
        assert_eq!(todos.len(), 2, "both cert/key TODOs should be detected");
        let placeholders: Vec<_> = todos.iter().map(|t| t.placeholder.as_str()).collect();
        assert!(placeholders.contains(&"<TODO: base64 encoded certificate>"));
        assert!(placeholders.contains(&"<TODO: base64 encoded private key>"));
        cleanup(&dir);
    }

    #[test]
    fn test_scan_lb_cross_ref_todo() {
        // loadbalancer.rs: unresolved LB backend reference
        let dir = make_temp_dir();
        let content = r#"resource "upcloud_loadbalancer_backend" "api" {
  loadbalancer = upcloud_loadbalancer.<TODO>.id
  name         = "api"
}
"#;
        fs::write(dir.join("lb.tf"), content).unwrap();
        let todos = scan_output_todos(&dir);
        assert_eq!(todos.len(), 1, "unresolved LB cross-ref should be detected");
        assert_eq!(todos[0].placeholder, "<TODO>");
        assert!(todos[0].line_content.contains("loadbalancer"));
        cleanup(&dir);
    }

    #[test]
    fn test_scan_generator_header_comments_not_matched() {
        // generator.rs writes `# resource_type resource_name` and `# NOTE:` headers
        // None of these should produce TODO matches even if they mention <TODO>
        let dir = make_temp_dir();
        let content = r#"# aws_security_group web
# NOTE: Security groups → UpCloud Firewall Rules
# NOTE: Set server_id to the target upcloud_server resource name.
resource "upcloud_firewall_rules" "web" {
  server_id = upcloud_server.app.id

  firewall_rule {
    direction = "in"
    action    = "accept"
    family    = "IPv4"
    comment   = "Allow HTTPS"
  }
}
"#;
        fs::write(dir.join("network.tf"), content).unwrap();
        let todos = scan_output_todos(&dir);
        assert_eq!(todos.len(), 0, "fully-resolved output should have zero TODOs");
        cleanup(&dir);
    }

    #[test]
    fn test_scan_ssh_key_with_keypair_name_todo() {
        // compute.rs: unresolved key pair reference
        let dir = make_temp_dir();
        let content = "    keys = [\"<TODO: SSH public key for aws_key_pair.prod>\"]\n";
        fs::write(dir.join("main.tf"), content).unwrap();
        let todos = scan_output_todos(&dir);
        assert_eq!(todos.len(), 1);
        assert_eq!(
            todos[0].placeholder,
            "<TODO: SSH public key for aws_key_pair.prod>"
        );
        cleanup(&dir);
    }

    #[test]
    fn test_apply_resolution_preserves_trailing_newline() {
        let dir = make_temp_dir();
        let content = "  x = \"<TODO: foo>\"\n";
        fs::write(dir.join("t.tf"), content).unwrap();

        let item = TodoItem {
            file: "t.tf".into(),
            line_no: 1,
            placeholder: "<TODO: foo>".into(),
            line_content: "  x = \"<TODO: foo>\"".into(),
            context: vec![],
            status: TodoStatus::Pending,
            resolution: None,
            ai_suggestion: None,
        };
        apply_resolution(&dir, &item, "bar").unwrap();
        let updated = fs::read_to_string(dir.join("t.tf")).unwrap();
        assert!(updated.ends_with('\n'), "trailing newline should be preserved");
        cleanup(&dir);
    }
}
