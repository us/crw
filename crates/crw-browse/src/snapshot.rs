//! Accessibility tree serializer for the `tree` tool.
//!
//! Takes the raw `Accessibility.getFullAXTree` response and renders it as a
//! compact, LLM-friendly format: one line per node, indentation reflects
//! parent/child structure, 80-char name truncation, role+name+token layout.

use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;

const MAX_NAME_LEN: usize = 80;
const INDENT: &str = "  ";

/// CDP encodes `nodeId` as a string for the accessibility domain, but some
/// browsers/builds have historically emitted a raw integer (DOM domain uses
/// `DOM.NodeId = integer` and a few payloads blend the two). Normalize both
/// into an owned `String` so downstream rendering stays uniform.
fn node_id_as_string(v: &Value) -> Option<String> {
    if let Some(s) = v.as_str() {
        return Some(s.to_string());
    }
    if let Some(n) = v.as_u64() {
        return Some(n.to_string());
    }
    if let Some(n) = v.as_i64() {
        return Some(n.to_string());
    }
    None
}

/// Render the CDP AX tree payload (the `nodes` array) as an indented listing.
///
/// Uses each node's `childIds` to reconstruct the tree. Roots are nodes that
/// aren't referenced as a child anywhere. Emits at most `max_nodes` lines.
/// If `childIds` information is missing (or the payload isn't an array),
/// falls back to a flat listing so the tool never silently returns empty.
pub fn render_compact(ax_nodes: &Value, max_nodes: usize) -> String {
    let Some(array) = ax_nodes.as_array() else {
        return String::new();
    };
    if array.is_empty() || max_nodes == 0 {
        return String::new();
    }

    // Owned-string indexes: `nodeId` may arrive as string OR integer depending
    // on the CDP version, so we can't borrow a `&str` from the JSON directly.
    let mut by_id: HashMap<String, &Value> = HashMap::with_capacity(array.len());
    for node in array {
        if let Some(id) = node.get("nodeId").and_then(node_id_as_string) {
            by_id.insert(id, node);
        }
    }

    // Collect every referenced child id; anything not in this set is a root.
    let mut referenced: HashSet<String> = HashSet::new();
    for node in array {
        if let Some(children) = node.get("childIds").and_then(|v| v.as_array()) {
            for c in children {
                if let Some(s) = node_id_as_string(c) {
                    referenced.insert(s);
                }
            }
        }
    }

    let roots: Vec<String> = array
        .iter()
        .filter_map(|n| n.get("nodeId").and_then(node_id_as_string))
        .filter(|id| !referenced.contains(id))
        .collect();

    let mut out = String::new();
    let mut emitted = 0usize;
    let mut visited: HashSet<String> = HashSet::new();
    for root in &roots {
        if emitted >= max_nodes {
            break;
        }
        render_node(
            root,
            &by_id,
            0,
            max_nodes,
            &mut emitted,
            &mut visited,
            &mut out,
        );
    }

    // Fallback: some nodes weren't reachable from roots (disconnected tree, or
    // childIds missing entirely). Append them flat so we don't drop content.
    if emitted < max_nodes {
        for node in array {
            if emitted >= max_nodes {
                break;
            }
            let id = match node.get("nodeId").and_then(node_id_as_string) {
                Some(s) => s,
                None => continue,
            };
            if visited.contains(&id) {
                continue;
            }
            write_line(node, 0, &mut out);
            emitted += 1;
        }
    }

    out
}

fn render_node(
    id: &str,
    by_id: &HashMap<String, &Value>,
    depth: usize,
    max_nodes: usize,
    emitted: &mut usize,
    visited: &mut HashSet<String>,
    out: &mut String,
) {
    if *emitted >= max_nodes || !visited.insert(id.to_string()) {
        return;
    }
    let Some(node) = by_id.get(id).copied() else {
        return;
    };
    write_line(node, depth, out);
    *emitted += 1;
    if let Some(children) = node.get("childIds").and_then(|v| v.as_array()) {
        for c in children {
            if *emitted >= max_nodes {
                break;
            }
            if let Some(child_id) = node_id_as_string(c) {
                render_node(
                    &child_id,
                    by_id,
                    depth + 1,
                    max_nodes,
                    emitted,
                    visited,
                    out,
                );
            }
        }
    }
}

fn write_line(node: &Value, depth: usize, out: &mut String) {
    let role = node
        .get("role")
        .and_then(|r| r.get("value"))
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");
    let name = node
        .get("name")
        .and_then(|n| n.get("value"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let node_id = node
        .get("nodeId")
        .and_then(node_id_as_string)
        .unwrap_or_else(|| "?".to_string());
    let truncated = truncate(name, MAX_NAME_LEN);
    for _ in 0..depth {
        out.push_str(INDENT);
    }
    // `writeln!` writes directly into the buffer via `format_args!`; avoids
    // the temporary-String allocation `push_str(&format!(...))` would make
    // once per node.
    let _ = writeln!(out, "[{node_id}] {role}: {truncated}");
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let mut out: String = s.chars().take(max - 1).collect();
        out.push('…');
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_value_yields_empty_string() {
        assert_eq!(render_compact(&serde_json::json!(null), 100), "");
        assert_eq!(render_compact(&serde_json::json!({}), 100), "");
    }

    #[test]
    fn renders_flat_nodes_without_child_ids() {
        let nodes = serde_json::json!([
            { "nodeId": "1", "role": { "value": "WebArea" }, "name": { "value": "Example" } },
            { "nodeId": "2", "role": { "value": "link" }, "name": { "value": "More info" } },
        ]);
        let out = render_compact(&nodes, 100);
        assert!(out.contains("[1] WebArea: Example"));
        assert!(out.contains("[2] link: More info"));
        // Neither references the other, so both appear at depth 0.
        for line in out.lines() {
            assert!(!line.starts_with(' '), "unexpected indent: {line}");
        }
    }

    #[test]
    fn indents_children_via_child_ids() {
        let nodes = serde_json::json!([
            { "nodeId": "1", "role": { "value": "WebArea" }, "name": { "value": "root" }, "childIds": ["2"] },
            { "nodeId": "2", "role": { "value": "main" }, "name": { "value": "body" }, "childIds": ["3"] },
            { "nodeId": "3", "role": { "value": "button" }, "name": { "value": "Go" } },
        ]);
        let out = render_compact(&nodes, 100);
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(lines.len(), 3);
        assert!(
            lines[0].starts_with("[1] WebArea"),
            "root at depth 0: {}",
            lines[0]
        );
        assert!(lines[1].starts_with("  [2] main"), "depth 1: {}", lines[1]);
        assert!(
            lines[2].starts_with("    [3] button"),
            "depth 2: {}",
            lines[2]
        );
    }

    #[test]
    fn truncates_long_names() {
        let long = "x".repeat(200);
        let nodes = serde_json::json!([
            { "nodeId": "1", "role": { "value": "heading" }, "name": { "value": long } }
        ]);
        let out = render_compact(&nodes, 100);
        assert!(out.contains("…"));
        assert!(
            out.len() < 200,
            "expected truncated line, got {} chars",
            out.len()
        );
    }

    #[test]
    fn respects_max_nodes_cap() {
        let nodes = serde_json::json!([
            { "nodeId": "1", "role": { "value": "link" }, "name": { "value": "a" } },
            { "nodeId": "2", "role": { "value": "link" }, "name": { "value": "b" } },
            { "nodeId": "3", "role": { "value": "link" }, "name": { "value": "c" } },
        ]);
        let out = render_compact(&nodes, 2);
        assert!(out.contains("[1]"));
        assert!(out.contains("[2]"));
        assert!(!out.contains("[3]"));
    }

    #[test]
    fn accepts_integer_node_ids() {
        // Integer form: nodeId + childIds as numbers.
        let nodes = serde_json::json!([
            { "nodeId": 1, "role": { "value": "WebArea" }, "name": { "value": "root" }, "childIds": [2] },
            { "nodeId": 2, "role": { "value": "link" }, "name": { "value": "click" } },
        ]);
        let out = render_compact(&nodes, 100);
        assert!(out.contains("[1] WebArea: root"), "{out}");
        assert!(out.contains("  [2] link: click"), "{out}");
    }

    #[test]
    fn mixed_string_and_integer_node_ids() {
        // Some payloads have one side integer, the other string — must still
        // stitch children to parents correctly.
        let nodes = serde_json::json!([
            { "nodeId": "1", "role": { "value": "WebArea" }, "name": { "value": "" }, "childIds": [2] },
            { "nodeId": 2, "role": { "value": "link" }, "name": { "value": "x" } },
        ]);
        let out = render_compact(&nodes, 100);
        assert!(out.contains("[1] WebArea"), "{out}");
        assert!(out.contains("  [2] link: x"), "{out}");
    }

    #[test]
    fn handles_cycle_without_infinite_loop() {
        // Pathological input: childIds forms a cycle.
        let nodes = serde_json::json!([
            { "nodeId": "1", "role": { "value": "r" }, "name": { "value": "" }, "childIds": ["2"] },
            { "nodeId": "2", "role": { "value": "r" }, "name": { "value": "" }, "childIds": ["1"] },
        ]);
        let out = render_compact(&nodes, 100);
        // Each node emitted at most once.
        assert_eq!(out.matches("[1]").count(), 1);
        assert_eq!(out.matches("[2]").count(), 1);
    }
}
