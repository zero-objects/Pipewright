//! Jenkinsfile (Groovy DSL) emitter — walks a CST subgraph and
//! renders it as nested `key { ... }` blocks.
//!
//! Surface form (excerpt from a real Jenkinsfile):
//!
//! ```text
//! pipeline {
//!     agent any
//!     stages {
//!         stage('build') {
//!             steps {
//!                 sh 'cargo build --release'
//!             }
//!         }
//!     }
//! }
//! ```
//!
//! Mapping → `key { ... }`; scalar value → `key 'value'`;
//! a sequence-valued field expands as repeated child blocks
//! (`stages` contains many `stage(...)` blocks).
//!
//! Special-case: a child mapping tagged `construct=<C>` with a
//! `name` carrier renders as `<C>('<name>') { ... }` — that's the
//! Jenkins idiom for `stage('build') { … }`. The seeder hoists the
//! name as a hub:attr satellite during forward; the reverse path
//! re-materialises it inside the inner mapping as a carrier
//! comment which we strip back into the positional arg here.

use std::fmt::Write;

use seesaw_core::graph::{GhostId, TypedGraph};

use crate::{
    cst_attr, CST_CARRIER_COMMENT, CST_HAS_CHILD, CST_MAPPING, CST_MAPPING_ENTRY, CST_SCALAR,
    CST_SEQUENCE, CST_SEQUENCE_ITEM, CST_VALUE_OF,
};

/// Emit a Jenkinsfile for the CST subgraph rooted at `root`.
/// `root` is expected to be the `cst:Mapping[construct=pipeline]`
/// node — the wrapper produced by the seeder for the body of the
/// outer `pipeline { ... }` block.
#[must_use]
pub fn emit_jenkinsfile(graph: &TypedGraph, root: GhostId) -> String {
    let mut out = String::new();
    out.push_str("pipeline {\n");
    emit_block_body(graph, root, 4, &mut out);
    out.push_str("}\n");
    out
}

fn emit_block_body(graph: &TypedGraph, mapping_id: GhostId, indent: usize, out: &mut String) {
    let entries = child_entries(graph, mapping_id);
    for entry_id in entries {
        let Some(entry) = graph.get_node(&entry_id) else {
            continue;
        };
        let key = entry.attrs.get(cst_attr::KEY).cloned().unwrap_or_default();
        if key.is_empty() {
            continue;
        }
        let Some(value_id) = outgoing(graph, entry_id, CST_VALUE_OF) else {
            continue;
        };
        let Some(value) = graph.get_node(&value_id) else {
            continue;
        };
        match value.type_id.as_str() {
            CST_SCALAR => {
                let text = value.attrs.get(cst_attr::TEXT).cloned().unwrap_or_default();
                write_indent(indent, out);
                if text.is_empty() {
                    let _ = writeln!(out, "{key}");
                } else if needs_quoting(&text) {
                    let _ = writeln!(out, "{key} {}", quote_scalar(&text));
                } else {
                    let _ = writeln!(out, "{key} {text}");
                }
            }
            CST_MAPPING => {
                write_indent(indent, out);
                let _ = writeln!(out, "{key} {{");
                emit_block_body(graph, value_id, indent + 4, out);
                write_indent(indent, out);
                out.push_str("}\n");
            }
            CST_SEQUENCE => {
                // Sequence under a Jenkins key — expand each item
                // as a child block. `stages: [stage_a, stage_b]`
                // → `stages { stage('a') {...} stage('b') {...} }`.
                write_indent(indent, out);
                let _ = writeln!(out, "{key} {{");
                for item_id in child_items(graph, value_id) {
                    if let Some(inner) = outgoing(graph, item_id, CST_VALUE_OF) {
                        emit_sequence_item(graph, inner, indent + 4, out);
                    }
                }
                write_indent(indent, out);
                out.push_str("}\n");
            }
            _ => {}
        }
    }
}

/// Emit one item from a sequence. If it's a tagged construct with
/// a `name` carrier (the Jenkins idiom: `stage('build') { … }`),
/// render with the positional-arg form; otherwise just expand
/// as `<construct> { … }`.
fn emit_sequence_item(graph: &TypedGraph, value_id: GhostId, indent: usize, out: &mut String) {
    let Some(value) = graph.get_node(&value_id) else {
        return;
    };
    if value.type_id != CST_MAPPING {
        // Scalar / sequence item — render flat.
        write_indent(indent, out);
        if value.type_id == CST_SCALAR {
            let text = value.attrs.get(cst_attr::TEXT).cloned().unwrap_or_default();
            if needs_quoting(&text) {
                let _ = writeln!(out, "{}", quote_scalar(&text));
            } else {
                let _ = writeln!(out, "{text}");
            }
        }
        return;
    }
    let construct = value
        .attrs
        .get(cst_attr::CONSTRUCT)
        .cloned()
        .unwrap_or_else(|| "stage".to_string());
    let name = find_name_carrier(graph, value_id);
    write_indent(indent, out);
    if let Some(n) = name {
        let _ = writeln!(out, "{construct}('{n}') {{");
    } else {
        let _ = writeln!(out, "{construct} {{");
    }
    emit_block_body(graph, value_id, indent + 4, out);
    write_indent(indent, out);
    out.push_str("}\n");
}

fn find_name_carrier(graph: &TypedGraph, mapping_id: GhostId) -> Option<String> {
    for (_, t, e) in graph
        .iter_edges()
        .into_iter()
        .filter(|(s, _, e)| *s == mapping_id && e.type_id == CST_HAS_CHILD)
    {
        let Some(nd) = graph.get_node(&t) else {
            continue;
        };
        if nd.type_id == CST_CARRIER_COMMENT
            && nd
                .attrs
                .get(cst_attr::TARGET_FIELD)
                .map(std::string::String::as_str)
                == Some("name")
        {
            if let Some(v) = nd.attrs.get(cst_attr::VALUE) {
                if !v.is_empty() {
                    return Some(v.clone());
                }
            }
        }
        let _ = e;
    }
    None
}

fn needs_quoting(s: &str) -> bool {
    s.is_empty()
        || s.contains([' ', '/', '.', ':', '\'', '"', '\n'])
        || s.starts_with(|c: char| c.is_ascii_digit())
}

/// Render a scalar as a quoted Groovy string whose escaping the seeder's
/// scalar resolver reverses EXACTLY. Single quotes only do YAML-style `''`
/// doubling on resolve (NOT backslash escapes), so a value containing `'`
/// (e.g. `lib('shared-ci@main')`) must be DOUBLE-quoted — there backslash
/// escapes round-trip symmetrically (resolve's unescape_double_quoted + the
/// jenkinsfile tokenizer both honour `\X`). A bare `'…'` with backslash-
/// escaped apostrophes would accumulate `\'` across passes.
fn quote_scalar(text: &str) -> String {
    if text.contains('\'') || text.contains('\\') {
        format!("\"{}\"", text.replace('\\', "\\\\").replace('"', "\\\""))
    } else {
        format!("'{text}'")
    }
}

fn child_entries(graph: &TypedGraph, parent: GhostId) -> Vec<GhostId> {
    graph
        .iter_edges()
        .into_iter()
        .filter(|(s, _, e)| *s == parent && e.type_id == CST_HAS_CHILD)
        .filter_map(|(_, t, _)| {
            graph
                .get_node(&t)
                .filter(|n| n.type_id == CST_MAPPING_ENTRY)
                .map(|_| t)
        })
        .collect()
}

fn child_items(graph: &TypedGraph, parent: GhostId) -> Vec<GhostId> {
    graph
        .iter_edges()
        .into_iter()
        .filter(|(s, _, e)| *s == parent && e.type_id == CST_HAS_CHILD)
        .filter_map(|(_, t, _)| {
            graph
                .get_node(&t)
                .filter(|n| n.type_id == CST_SEQUENCE_ITEM)
                .map(|_| t)
        })
        .collect()
}

fn outgoing(graph: &TypedGraph, source: GhostId, kind: &str) -> Option<GhostId> {
    graph
        .iter_edges()
        .into_iter()
        .find(|(s, _, e)| *s == source && e.type_id == kind)
        .map(|(_, t, _)| t)
}

fn write_indent(n: usize, out: &mut String) {
    for _ in 0..n {
        out.push(' ');
    }
}
