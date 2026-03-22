use std::collections::HashMap;

/// Dependency graph node: a resource and the resources it depends on.
#[derive(Debug, Clone)]
pub struct GraphNode {
    /// Display name, e.g. `upcloud_server.web`
    pub name: String,
    /// Names of resources this node directly depends on.
    pub deps: Vec<String>,
}

// ── DOT parser ────────────────────────────────────────────────────────────────

/// Parse the DOT output of `terraform graph` into a sorted list of `GraphNode`s.
/// Provider meta-nodes and internal Terraform scaffolding are filtered out.
pub fn parse_dot(dot: &str) -> Vec<GraphNode> {
    let mut id_to_label: HashMap<String, String> = HashMap::new();
    let mut raw_edges: Vec<(String, String)> = Vec::new();

    for line in dot.lines() {
        let t = line.trim();

        // Node declaration: "ID" [label = "LABEL", ...]
        if t.starts_with('"') && t.contains("[label") {
            if let Some((id, rest)) = read_quoted(t)
                && let Some(label) = extract_label_attr(rest)
            {
                id_to_label.insert(id, label);
            }
            continue;
        }

        // Edge: "SRC" -> "DST"
        if t.starts_with('"')
            && t.contains("\" -> \"")
            && let Some((src, rest)) = read_quoted(t)
        {
            let rest = rest.trim();
            if let Some(after) = rest.strip_prefix("->") {
                let after = after.trim();
                if after.starts_with('"')
                    && let Some((dst, _)) = read_quoted(after)
                {
                    raw_edges.push((src, dst));
                }
            }
        }
    }

    let is_interesting = |label: &str| -> bool {
        if label.starts_with("provider[") || label.starts_with("meta.") || label == "root" {
            return false;
        }
        label.contains('.')
    };

    let interesting_ids: std::collections::HashSet<&str> = id_to_label
        .iter()
        .filter(|(_, label)| is_interesting(label))
        .map(|(id, _)| id.as_str())
        .collect();

    let mut nodes: Vec<GraphNode> = id_to_label
        .iter()
        .filter(|(id, label)| interesting_ids.contains(id.as_str()) && is_interesting(label))
        .map(|(id, label)| {
            let mut deps: Vec<String> = raw_edges
                .iter()
                .filter(|(src, _)| src == id)
                .filter_map(|(_, dst)| id_to_label.get(dst).filter(|l| is_interesting(l)).cloned())
                .collect();
            deps.sort();
            deps.dedup();
            GraphNode {
                name: label.clone(),
                deps,
            }
        })
        .collect();

    nodes.sort_by(|a, b| a.name.cmp(&b.name));
    nodes
}

fn read_quoted(s: &str) -> Option<(String, &str)> {
    let s = s.strip_prefix('"')?;
    let mut result = String::new();
    let mut chars = s.char_indices().peekable();
    loop {
        match chars.next()? {
            (i, '"') => return Some((result, &s[i + 1..])),
            (_, '\\') => match chars.next()? {
                (_, '"') => result.push('"'),
                (_, c) => {
                    result.push('\\');
                    result.push(c);
                }
            },
            (_, c) => result.push(c),
        }
    }
}

fn extract_label_attr(s: &str) -> Option<String> {
    let pos = s.find("label")?;
    let after = s[pos + 5..].trim_start();
    let after = after.strip_prefix('=')?;
    let after = after.trim_start();
    let (label, _) = read_quoted(after)?;
    Some(label)
}

// ── Fallback: build graph from resolved HCL ───────────────────────────────────

pub fn build_graph_from_hcl(
    resolved_hcl_map: &HashMap<(String, String), String>,
) -> Vec<GraphNode> {
    let all_names: Vec<String> = resolved_hcl_map
        .keys()
        .map(|(t, n)| format!("{}.{}", t, n))
        .collect();

    let mut nodes: Vec<GraphNode> = resolved_hcl_map
        .iter()
        .map(|((rtype, rname), hcl)| {
            let full_name = format!("{}.{}", rtype, rname);
            let mut deps: Vec<String> = all_names
                .iter()
                .filter(|candidate| *candidate != &full_name && hcl.contains(candidate.as_str()))
                .cloned()
                .collect();
            deps.sort();
            GraphNode {
                name: full_name,
                deps,
            }
        })
        .collect();

    nodes.sort_by(|a, b| a.name.cmp(&b.name));
    nodes
}

// ── Mermaid export ────────────────────────────────────────────────────────────

/// Convert a list of graph nodes into a Mermaid `graph TD` block.
pub fn nodes_to_mermaid(nodes: &[GraphNode]) -> String {
    // Use index-based node IDs to avoid Mermaid special-character issues.
    let id_map: HashMap<&str, String> = nodes
        .iter()
        .enumerate()
        .map(|(i, n)| (n.name.as_str(), format!("n{i}")))
        .collect();

    let mut lines = vec!["graph TD".to_string()];

    // Node declarations with quoted labels
    for (i, node) in nodes.iter().enumerate() {
        let label = node.name.replace('"', "'");
        lines.push(format!("    n{i}[\"{label}\"]"));
    }

    // Edges
    for node in nodes {
        if let Some(src_id) = id_map.get(node.name.as_str()) {
            for dep in &node.deps {
                if let Some(dst_id) = id_map.get(dep.as_str()) {
                    lines.push(format!("    {src_id} --> {dst_id}"));
                }
            }
        }
    }

    lines.join("\n")
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_DOT: &str = r#"
digraph {
  compound = "true"
  newrank = "true"
  subgraph "root" {
    "[root] upcloud_server.web (expand)" [label = "upcloud_server.web", shape = "box"]
    "[root] upcloud_network.main (expand)" [label = "upcloud_network.main", shape = "box"]
    "[root] upcloud_router.main (expand)" [label = "upcloud_router.main", shape = "box"]
    "[root] provider[\"registry.terraform.io/hashicorp/upcloud\"]" [label = "provider[\"registry.terraform.io/hashicorp/upcloud\"]", shape = "diamond"]
    "[root] upcloud_server.web (expand)" -> "[root] upcloud_network.main (expand)"
    "[root] upcloud_server.web (expand)" -> "[root] provider[\"registry.terraform.io/hashicorp/upcloud\"]"
    "[root] upcloud_network.main (expand)" -> "[root] upcloud_router.main (expand)"
  }
}
"#;

    #[test]
    fn parse_dot_extracts_nodes_and_edges() {
        let nodes = parse_dot(SAMPLE_DOT);
        assert_eq!(nodes.len(), 3, "should have 3 resource nodes");

        let server = nodes
            .iter()
            .find(|n| n.name == "upcloud_server.web")
            .unwrap();
        assert_eq!(server.deps, vec!["upcloud_network.main"]);

        let net = nodes
            .iter()
            .find(|n| n.name == "upcloud_network.main")
            .unwrap();
        assert_eq!(net.deps, vec!["upcloud_router.main"]);

        let router = nodes
            .iter()
            .find(|n| n.name == "upcloud_router.main")
            .unwrap();
        assert!(router.deps.is_empty());
    }

    #[test]
    fn nodes_to_mermaid_produces_valid_block() {
        let nodes = vec![
            GraphNode {
                name: "upcloud_server.web".into(),
                deps: vec!["upcloud_network.main".into()],
            },
            GraphNode {
                name: "upcloud_network.main".into(),
                deps: vec![],
            },
        ];
        let mermaid = nodes_to_mermaid(&nodes);
        assert!(mermaid.starts_with("graph TD"));
        assert!(mermaid.contains("upcloud_server.web"));
        assert!(mermaid.contains("upcloud_network.main"));
        assert!(mermaid.contains("-->"));
    }
}
