//! Earthfile renderer — bare-minimum DSL emitter.
//!
//! Earthly's surface syntax is verb-prefixed (`VERSION 0.8`,
//! `RUN cargo build`, …) not key-colon-value, so the YAML emitter
//! produces output the Earthfile parser can't read. This module
//! walks the same CST subgraph the reverse cascade builds and
//! writes it as Earthfile lines.
//!
//! Scope (matching what the current ruleset materialises):
//! * Top-level mapping entries → `KEY value` lines. Scalar value
//!   only — block/sequence values are not currently produced for
//!   earthly pipeline attrs.
//! * Recipe blocks (`name:` at top level whose value is a
//!   mapping of `VERB args` entries) — emitted as `name:` then
//!   indented `VERB args` lines.
//!
//! Out of scope until the earthly seeder/forward rules round-trip
//! more: comments, multi-line ARG/ENV, COPY artefact paths,
//! WITH DOCKER blocks, etc.

use std::fmt::Write;

use seesaw_core::graph::{GhostId, TypedGraph};

use crate::{
    cst_attr, CST_HAS_CHILD, CST_MAPPING, CST_MAPPING_ENTRY, CST_SCALAR, CST_SEQUENCE_ITEM,
    CST_VALUE_OF,
};

/// Emit Earthfile syntax for the CST subgraph rooted at `root`.
#[must_use]
pub fn emit_earthfile(graph: &TypedGraph, root: GhostId) -> String {
    let mut out = String::new();
    emit_mapping(graph, root, 0, &mut out);
    out
}

fn emit_mapping(graph: &TypedGraph, id: GhostId, indent: usize, out: &mut String) {
    let entries = child_entries(graph, id);
    // At the FILE level (indent 0) Earthfile keys are unique (one VERSION, one
    // PROJECT, distinct target names). A cross-platform hub can carry two values
    // for a single-valued earthly field — e.g. drone `kind`+`type` and tekton
    // `apiVersion`+`kind` both map to `version`, so re-keying into earthly yields
    // two `VERSION` attrs. Emitting both produces an invalid two-`VERSION`
    // Earthfile that doesn't round-trip (the matrix `≠`); YAML targets dedupe
    // implicitly because map keys are unique. Mirror that: emit each file-level
    // key once. Inside a recipe (indent > 0) commands like `RUN` legitimately
    // repeat, so dedup is file-level only.
    let mut seen_top: std::collections::HashSet<String> = std::collections::HashSet::new();
    for entry_id in entries {
        let Some(entry) = graph.get_node(&entry_id) else {
            continue;
        };
        let key = entry.attrs.get(cst_attr::KEY).cloned().unwrap_or_default();
        if key.is_empty() {
            continue;
        }
        if indent == 0 && !seen_top.insert(key.clone()) {
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
                } else {
                    let _ = writeln!(out, "{key} {text}");
                }
            }
            CST_MAPPING => {
                // Earthly recipe block: `name:` then indented
                // body. The body's entries are VERB+args lines.
                write_indent(indent, out);
                let _ = writeln!(out, "{key}:");
                emit_mapping(graph, value_id, indent + 4, out);
            }
            "cst:Sequence" => {
                // A sequence under a top-level KEY is currently
                // unusual for earthly — flatten as space-joined
                // scalars.
                let items: Vec<String> = child_items(graph, value_id)
                    .into_iter()
                    .filter_map(|item_id| {
                        let v = outgoing(graph, item_id, CST_VALUE_OF)?;
                        graph
                            .get_node(&v)
                            .and_then(|n| n.attrs.get(cst_attr::TEXT).cloned())
                    })
                    .collect();
                write_indent(indent, out);
                let _ = writeln!(out, "{key} {}", items.join(" "));
            }
            _ => {
                write_indent(indent, out);
                let _ = writeln!(out, "{key}");
            }
        }
    }
}

fn write_indent(indent: usize, out: &mut String) {
    for _ in 0..indent {
        out.push(' ');
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
