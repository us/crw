//! Accessibility tree serializer for the `tree` tool.
//!
//! Takes the raw `Accessibility.getFullAXTree` response and renders it as a
//! compact, LLM-friendly format: one line per node, indentation reflects
//! parent/child structure, 80-char name truncation, role+name layout.
//!
//! Each emitted line is prefixed with a sequential `@e<N>` reference token.
//! The handler stores the `@e<N> -> backendDOMNodeId` map on the session so
//! later interaction tools (`click`, `fill`, ...) can resolve a ref back to
//! a DOM node without re-walking the tree. AX nodes that have no DOM mapping
//! (text fragments, virtual scrollable groups) still get a ref, but resolve
//! to `None` — `resolve_ref` rejects those at click time.

use serde::Serialize;
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;

const MAX_NAME_LEN: usize = 80;
const INDENT: &str = "  ";

/// `@e<N>` reference paired with the DOM backend node id that the AX node
/// resolves to (if any). Returned as a side output of [`render_compact`] so
/// callers can populate the session ref map.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RefMapEntry {
    pub ref_id: String,
    pub backend_node_id: Option<i64>,
}

/// Bundled result of [`render_compact`] — the rendered tree text plus the
/// ref entries discovered while walking it. Order matches emission order.
#[derive(Debug, Clone)]
pub struct RenderedTree {
    pub text: String,
    pub refs: Vec<RefMapEntry>,
}

/// Structured JSON form of an AX node, produced by [`render_json`]. Each node
/// carries the `@e<N>` ref so JSON callers can drive `click`/`fill` without
/// re-parsing the text tree. `backend_node_id` is `None` when the AX node has
/// no DOM mapping (text fragments, virtual groups) — same semantics as the
/// text variant; resolve_ref will reject these at click time.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct TreeNode {
    pub ref_id: String,
    pub role: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub backend_node_id: Option<i64>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<TreeNode>,
}

/// Bundled result of [`render_json`] — the structured root list plus the
/// ref entries collected during traversal (same order as the equivalent text
/// rendering, so callers can swap representations without ref drift).
#[derive(Debug, Clone)]
pub struct RenderedJson {
    pub roots: Vec<TreeNode>,
    pub refs: Vec<RefMapEntry>,
}

/// CDP encodes `nodeId` as a string for the accessibility domain, but some
/// browsers/builds have historically emitted a raw integer. Normalize both
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
///
/// Each emitted line is prefixed with a sequential `@e<N>` token (1-indexed).
/// The accompanying [`Vec<RefMapEntry>`] contains the same refs paired with
/// their `backendDOMNodeId` (if the AX node carries one).
pub fn render_compact(ax_nodes: &Value, max_nodes: usize) -> RenderedTree {
    let Some(array) = ax_nodes.as_array() else {
        return RenderedTree {
            text: String::new(),
            refs: Vec::new(),
        };
    };
    if array.is_empty() || max_nodes == 0 {
        return RenderedTree {
            text: String::new(),
            refs: Vec::new(),
        };
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

    let mut ctx = RenderCtx {
        by_id: &by_id,
        max_nodes,
        emitted: 0,
        visited: HashSet::new(),
        out: String::new(),
        refs: Vec::new(),
    };

    for root in &roots {
        if ctx.emitted >= max_nodes {
            break;
        }
        render_node(root, 0, &mut ctx);
    }

    // Fallback: some nodes weren't reachable from roots. Append them flat.
    if ctx.emitted < max_nodes {
        for node in array {
            if ctx.emitted >= max_nodes {
                break;
            }
            let id = match node.get("nodeId").and_then(node_id_as_string) {
                Some(s) => s,
                None => continue,
            };
            if ctx.visited.contains(&id) {
                continue;
            }
            write_line(node, 0, &mut ctx);
        }
    }

    RenderedTree {
        text: ctx.out,
        refs: ctx.refs,
    }
}

struct RenderCtx<'a> {
    by_id: &'a HashMap<String, &'a Value>,
    max_nodes: usize,
    emitted: usize,
    visited: HashSet<String>,
    out: String,
    refs: Vec<RefMapEntry>,
}

fn render_node(id: &str, depth: usize, ctx: &mut RenderCtx<'_>) {
    if ctx.emitted >= ctx.max_nodes || !ctx.visited.insert(id.to_string()) {
        return;
    }
    let Some(node) = ctx.by_id.get(id).copied() else {
        return;
    };
    write_line(node, depth, ctx);
    if let Some(children) = node.get("childIds").and_then(|v| v.as_array()) {
        for c in children {
            if ctx.emitted >= ctx.max_nodes {
                break;
            }
            if let Some(child_id) = node_id_as_string(c) {
                render_node(&child_id, depth + 1, ctx);
            }
        }
    }
}

fn write_line(node: &Value, depth: usize, ctx: &mut RenderCtx<'_>) {
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
    let backend_node_id = node
        .get("backendDOMNodeId")
        .and_then(|v| v.as_i64().or_else(|| v.as_u64().map(|u| u as i64)));
    let truncated = truncate(name, MAX_NAME_LEN);

    ctx.emitted += 1;
    let ref_id = format!("@e{}", ctx.emitted);

    for _ in 0..depth {
        ctx.out.push_str(INDENT);
    }
    let _ = writeln!(ctx.out, "{ref_id} {role}: {truncated}");

    ctx.refs.push(RefMapEntry {
        ref_id,
        backend_node_id,
    });
}

/// JSON form of [`render_compact`]. Same traversal (cycle-safe, root detection,
/// flat fallback for orphan nodes), same ref numbering — only the output shape
/// differs. Caller picks the format based on whether the LLM is parsing text
/// or structured JSON.
pub fn render_json(ax_nodes: &Value, max_nodes: usize) -> RenderedJson {
    let Some(array) = ax_nodes.as_array() else {
        return RenderedJson {
            roots: Vec::new(),
            refs: Vec::new(),
        };
    };
    if array.is_empty() || max_nodes == 0 {
        return RenderedJson {
            roots: Vec::new(),
            refs: Vec::new(),
        };
    }

    let mut by_id: HashMap<String, &Value> = HashMap::with_capacity(array.len());
    for node in array {
        if let Some(id) = node.get("nodeId").and_then(node_id_as_string) {
            by_id.insert(id, node);
        }
    }

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

    let root_ids: Vec<String> = array
        .iter()
        .filter_map(|n| n.get("nodeId").and_then(node_id_as_string))
        .filter(|id| !referenced.contains(id))
        .collect();

    let mut ctx = JsonCtx {
        by_id: &by_id,
        max_nodes,
        emitted: 0,
        visited: HashSet::new(),
        refs: Vec::new(),
    };

    let mut roots: Vec<TreeNode> = Vec::new();
    for root in &root_ids {
        if ctx.emitted >= max_nodes {
            break;
        }
        if let Some(node) = build_node(root, &mut ctx) {
            roots.push(node);
        }
    }

    // Flat fallback for unreachable nodes — preserves parity with render_compact.
    if ctx.emitted < max_nodes {
        for node in array {
            if ctx.emitted >= max_nodes {
                break;
            }
            let id = match node.get("nodeId").and_then(node_id_as_string) {
                Some(s) => s,
                None => continue,
            };
            if ctx.visited.contains(&id) {
                continue;
            }
            if let Some(orphan) = build_node(&id, &mut ctx) {
                roots.push(orphan);
            }
        }
    }

    RenderedJson {
        roots,
        refs: ctx.refs,
    }
}

struct JsonCtx<'a> {
    by_id: &'a HashMap<String, &'a Value>,
    max_nodes: usize,
    emitted: usize,
    visited: HashSet<String>,
    refs: Vec<RefMapEntry>,
}

fn build_node(id: &str, ctx: &mut JsonCtx<'_>) -> Option<TreeNode> {
    if ctx.emitted >= ctx.max_nodes || !ctx.visited.insert(id.to_string()) {
        return None;
    }
    let node = ctx.by_id.get(id).copied()?;
    let role = node
        .get("role")
        .and_then(|r| r.get("value"))
        .and_then(|v| v.as_str())
        .unwrap_or("unknown")
        .to_string();
    let name = node
        .get("name")
        .and_then(|n| n.get("value"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let backend_node_id = node
        .get("backendDOMNodeId")
        .and_then(|v| v.as_i64().or_else(|| v.as_u64().map(|u| u as i64)));
    let truncated = truncate(name, MAX_NAME_LEN);

    ctx.emitted += 1;
    let ref_id = format!("@e{}", ctx.emitted);
    ctx.refs.push(RefMapEntry {
        ref_id: ref_id.clone(),
        backend_node_id,
    });

    let mut children: Vec<TreeNode> = Vec::new();
    if let Some(child_ids) = node.get("childIds").and_then(|v| v.as_array()) {
        for c in child_ids {
            if ctx.emitted >= ctx.max_nodes {
                break;
            }
            if let Some(child_id) = node_id_as_string(c)
                && let Some(child) = build_node(&child_id, ctx)
            {
                children.push(child);
            }
        }
    }

    Some(TreeNode {
        ref_id,
        role,
        name: truncated,
        backend_node_id,
        children,
    })
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
        assert_eq!(render_compact(&serde_json::json!(null), 100).text, "");
        assert_eq!(render_compact(&serde_json::json!({}), 100).text, "");
    }

    #[test]
    fn renders_flat_nodes_without_child_ids() {
        let nodes = serde_json::json!([
            { "nodeId": "1", "role": { "value": "WebArea" }, "name": { "value": "Example" }, "backendDOMNodeId": 10 },
            { "nodeId": "2", "role": { "value": "link" }, "name": { "value": "More info" }, "backendDOMNodeId": 11 },
        ]);
        let r = render_compact(&nodes, 100);
        assert!(r.text.contains("@e1 WebArea: Example"));
        assert!(r.text.contains("@e2 link: More info"));
        for line in r.text.lines() {
            assert!(!line.starts_with(' '), "unexpected indent: {line}");
        }
        assert_eq!(r.refs.len(), 2);
        assert_eq!(r.refs[0].ref_id, "@e1");
        assert_eq!(r.refs[0].backend_node_id, Some(10));
        assert_eq!(r.refs[1].backend_node_id, Some(11));
    }

    #[test]
    fn indents_children_via_child_ids() {
        let nodes = serde_json::json!([
            { "nodeId": "1", "role": { "value": "WebArea" }, "name": { "value": "root" }, "childIds": ["2"], "backendDOMNodeId": 1 },
            { "nodeId": "2", "role": { "value": "main" }, "name": { "value": "body" }, "childIds": ["3"], "backendDOMNodeId": 2 },
            { "nodeId": "3", "role": { "value": "button" }, "name": { "value": "Go" }, "backendDOMNodeId": 3 },
        ]);
        let r = render_compact(&nodes, 100);
        let lines: Vec<&str> = r.text.lines().collect();
        assert_eq!(lines.len(), 3);
        assert!(lines[0].starts_with("@e1 WebArea"));
        assert!(lines[1].starts_with("  @e2 main"));
        assert!(lines[2].starts_with("    @e3 button"));
        assert_eq!(r.refs.len(), 3);
    }

    #[test]
    fn emits_ref_even_when_backend_node_id_missing() {
        let nodes = serde_json::json!([
            { "nodeId": "1", "role": { "value": "WebArea" }, "name": { "value": "x" } },
        ]);
        let r = render_compact(&nodes, 100);
        assert!(r.text.contains("@e1 WebArea: x"));
        assert_eq!(r.refs.len(), 1);
        assert!(
            r.refs[0].backend_node_id.is_none(),
            "ref should resolve to None when AX node has no backendDOMNodeId"
        );
    }

    #[test]
    fn truncates_long_names() {
        let long = "x".repeat(200);
        let nodes = serde_json::json!([
            { "nodeId": "1", "role": { "value": "heading" }, "name": { "value": long } }
        ]);
        let r = render_compact(&nodes, 100);
        assert!(r.text.contains("…"));
        assert!(
            r.text.len() < 200,
            "expected truncated line, got {} chars",
            r.text.len()
        );
    }

    #[test]
    fn respects_max_nodes_cap() {
        let nodes = serde_json::json!([
            { "nodeId": "1", "role": { "value": "link" }, "name": { "value": "a" } },
            { "nodeId": "2", "role": { "value": "link" }, "name": { "value": "b" } },
            { "nodeId": "3", "role": { "value": "link" }, "name": { "value": "c" } },
        ]);
        let r = render_compact(&nodes, 2);
        assert!(r.text.contains("@e1"));
        assert!(r.text.contains("@e2"));
        assert!(!r.text.contains("@e3"));
        assert_eq!(r.refs.len(), 2);
    }

    #[test]
    fn accepts_integer_node_ids() {
        let nodes = serde_json::json!([
            { "nodeId": 1, "role": { "value": "WebArea" }, "name": { "value": "root" }, "childIds": [2], "backendDOMNodeId": 5 },
            { "nodeId": 2, "role": { "value": "link" }, "name": { "value": "click" }, "backendDOMNodeId": 6 },
        ]);
        let r = render_compact(&nodes, 100);
        assert!(r.text.contains("@e1 WebArea: root"));
        assert!(r.text.contains("  @e2 link: click"));
    }

    #[test]
    fn mixed_string_and_integer_node_ids() {
        let nodes = serde_json::json!([
            { "nodeId": "1", "role": { "value": "WebArea" }, "name": { "value": "" }, "childIds": [2] },
            { "nodeId": 2, "role": { "value": "link" }, "name": { "value": "x" } },
        ]);
        let r = render_compact(&nodes, 100);
        assert!(r.text.contains("@e1 WebArea"));
        assert!(r.text.contains("  @e2 link: x"));
    }

    #[test]
    fn json_renders_nested_structure() {
        let nodes = serde_json::json!([
            { "nodeId": "1", "role": { "value": "WebArea" }, "name": { "value": "root" }, "childIds": ["2"], "backendDOMNodeId": 1 },
            { "nodeId": "2", "role": { "value": "main" }, "name": { "value": "body" }, "childIds": ["3"], "backendDOMNodeId": 2 },
            { "nodeId": "3", "role": { "value": "button" }, "name": { "value": "Go" }, "backendDOMNodeId": 3 },
        ]);
        let r = render_json(&nodes, 100);
        assert_eq!(r.roots.len(), 1);
        let root = &r.roots[0];
        assert_eq!(root.ref_id, "@e1");
        assert_eq!(root.role, "WebArea");
        assert_eq!(root.children.len(), 1);
        assert_eq!(root.children[0].ref_id, "@e2");
        assert_eq!(root.children[0].children[0].ref_id, "@e3");
        assert_eq!(r.refs.len(), 3);
        // Ref numbering must match render_compact for parity.
        let text = render_compact(&nodes, 100);
        assert_eq!(
            r.refs.iter().map(|e| e.ref_id.clone()).collect::<Vec<_>>(),
            text.refs
                .iter()
                .map(|e| e.ref_id.clone())
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn json_serializes_omitting_empty_children() {
        let nodes = serde_json::json!([
            { "nodeId": "1", "role": { "value": "link" }, "name": { "value": "x" }, "backendDOMNodeId": 9 },
        ]);
        let r = render_json(&nodes, 100);
        let json = serde_json::to_value(&r.roots).unwrap();
        let leaf = &json[0];
        assert_eq!(leaf["ref_id"], "@e1");
        assert_eq!(leaf["backend_node_id"], 9);
        assert!(
            leaf.get("children").is_none(),
            "leaf should omit empty children array, got {leaf}"
        );
    }

    #[test]
    fn json_respects_max_nodes_cap() {
        let nodes = serde_json::json!([
            { "nodeId": "1", "role": { "value": "r" }, "name": { "value": "" }, "childIds": ["2", "3"] },
            { "nodeId": "2", "role": { "value": "link" }, "name": { "value": "a" } },
            { "nodeId": "3", "role": { "value": "link" }, "name": { "value": "b" } },
        ]);
        let r = render_json(&nodes, 2);
        assert_eq!(r.refs.len(), 2);
    }

    #[test]
    fn handles_cycle_without_infinite_loop() {
        let nodes = serde_json::json!([
            { "nodeId": "1", "role": { "value": "r" }, "name": { "value": "" }, "childIds": ["2"] },
            { "nodeId": "2", "role": { "value": "r" }, "name": { "value": "" }, "childIds": ["1"] },
        ]);
        let r = render_compact(&nodes, 100);
        assert_eq!(r.text.matches("@e1").count(), 1);
        assert_eq!(r.text.matches("@e2").count(), 1);
    }
}
