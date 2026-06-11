//! CircleCI: jobs nest under `jobs:`; `workflows:` is meta.

use crate::{
    add_child, classify::circleci::CONSTRUCT_KEYS, cst_attr, make_attrs,
    seed_top_entry_as_job_block, seed_top_entry_as_meta, seed_top_level, SeededGraph,
    CST_HAS_CHILD, CST_MAPPING, CST_MAPPING_ENTRY, CST_SCALAR, CST_VALUE_OF,
};
use pipeline_cst::Document;
use seesaw_core::graph::{GhostId, TypedGraph};

#[must_use]
pub fn seed_from_document(doc: &Document, source_file: &str) -> SeededGraph {
    let mut seeded = seed_top_level(
        doc,
        source_file,
        CONSTRUCT_KEYS,
        |graph, parent_map, entry_node, key, value, source, anchors, classify| {
            if key == "jobs" {
                let _ = seed_top_entry_as_job_block(
                    graph, parent_map, entry_node, key, value, source, anchors, classify,
                );
            } else if matches!(key, "workflows" | "commands" | "executors") {
                // `workflows:` job entries are REFERENCES (not job defs);
                // `commands:`/`executors:` are reusable templates whose nested
                // `steps:` are DEFINITIONS referenced elsewhere, not pipeline
                // steps. Seeding any of them with the normal classify would tag
                // those nested jobs/steps construct=job/step and mint spurious
                // hub nodes with no backward pendant (asymmetric loss). Seed
                // them with an EMPTY classify so they round-trip as opaque
                // structure without creating constructs.
                seed_top_entry_as_meta(
                    graph,
                    parent_map,
                    entry_node,
                    key,
                    value,
                    source,
                    anchors,
                    &[],
                );
            } else {
                seed_top_entry_as_meta(
                    graph, parent_map, entry_node, key, value, source, anchors, classify,
                );
            }
        },
    );
    hoist_run_blocks(&mut seeded.graph);
    seeded
}

/// A CircleCI step's `run:` may be a scalar command OR a block
/// `{name, command, …}`. The block form leaves step.run (a scalar field) with
/// nothing to match — the command + name nest one level too deep — so the step
/// materialises EMPTY and is dropped on emit. Hoist each run-block's `command`
/// and `name` up to be direct children of the step, where command→step.run and
/// name→step.name fire. The original (unmatched) `run` block entry is harmless:
/// step.run's scalar rule ignores its mapping value, so nothing is duplicated.
fn hoist_run_blocks(graph: &mut TypedGraph) {
    let step_ids: Vec<GhostId> = graph
        .iter_nodes()
        .filter(|n| {
            n.type_id == CST_MAPPING
                && n.attrs.get(cst_attr::CONSTRUCT).map(String::as_str) == Some("step")
        })
        .map(|n| n.id)
        .collect();
    // Collect first (immutable borrow), then apply (mutable) — avoids aliasing.
    let mut hoists: Vec<(GhostId, String, String, String, String)> = Vec::new();
    for step_id in step_ids {
        let Some(run_entry) = child_entry(graph, step_id, "run") else {
            continue;
        };
        let Some(run_val) = outgoing(graph, run_entry, CST_VALUE_OF) else {
            continue;
        };
        if graph.get_node(&run_val).map(|n| n.type_id.as_str()) != Some(CST_MAPPING) {
            continue; // scalar `run:` — already handled by step.run
        }
        for inner_key in ["command", "name"] {
            if let Some(ie) = child_entry(graph, run_val, inner_key) {
                if let Some(sc) = outgoing(graph, ie, CST_VALUE_OF) {
                    if let Some(scn) = graph.get_node(&sc) {
                        if let Some(text) = scn.attrs.get(cst_attr::TEXT) {
                            let (s, e) = (
                                scn.attrs
                                    .get(cst_attr::SPAN_START)
                                    .cloned()
                                    .unwrap_or_default(),
                                scn.attrs
                                    .get(cst_attr::SPAN_END)
                                    .cloned()
                                    .unwrap_or_default(),
                            );
                            hoists.push((step_id, inner_key.to_string(), text.clone(), s, e));
                        }
                    }
                }
            }
        }
    }
    for (step_id, key, text, span_s, span_e) in hoists {
        let entry_attrs = make_attrs(&[
            (cst_attr::KEY, &key),
            ("entry_role", "annotation"),
            (cst_attr::FROM_MERGE, "false"),
            (cst_attr::SPAN_START, &span_s),
            (cst_attr::SPAN_END, &span_e),
        ]);
        let entry_id = add_child(
            graph,
            step_id,
            CST_HAS_CHILD,
            CST_MAPPING_ENTRY,
            entry_attrs,
        );
        let sc_attrs = make_attrs(&[
            (cst_attr::TEXT, &text),
            (cst_attr::PARENT_KEY, &key),
            (cst_attr::SPAN_START, &span_s),
            (cst_attr::SPAN_END, &span_e),
        ]);
        add_child(graph, entry_id, CST_VALUE_OF, CST_SCALAR, sc_attrs);
    }
}

/// The `cst:MappingEntry` child of `parent` whose key == `key`, if any.
fn child_entry(graph: &TypedGraph, parent: GhostId, key: &str) -> Option<GhostId> {
    graph
        .iter_edges()
        .into_iter()
        .filter(|(s, _, e)| *s == parent && e.type_id == CST_HAS_CHILD)
        .find_map(|(_, t, _)| {
            graph.get_node(&t).filter(|n| {
                n.type_id == CST_MAPPING_ENTRY
                    && n.attrs.get(cst_attr::KEY).map(String::as_str) == Some(key)
            })?;
            Some(t)
        })
}

fn outgoing(graph: &TypedGraph, source: GhostId, kind: &str) -> Option<GhostId> {
    graph
        .iter_edges()
        .into_iter()
        .find(|(s, _, e)| *s == source && e.type_id == kind)
        .map(|(_, t, _)| t)
}
