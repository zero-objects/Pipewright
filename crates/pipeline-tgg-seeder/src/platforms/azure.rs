//! Azure Pipelines: jobs can live under `jobs:`, `phases:`,
//! `stages.<n>.jobs:` (deeper). For now we honour the two flat
//! containers; nested stages get classification-driven tagging.

use crate::{
    add_child, classify::azure::CONSTRUCT_KEYS, cst_attr, make_attrs, seed_top_entry_as_job_block,
    seed_top_entry_as_meta, seed_top_level, SeededGraph, CST_HAS_CHILD, CST_MAPPING,
    CST_MAPPING_ENTRY, CST_SEQUENCE, CST_VALUE_OF,
};
use pipeline_cst::Document;
use seesaw_core::graph::{GhostId, Status, TypedGraph};
use std::collections::BTreeMap;

#[must_use]
pub fn seed_from_document(doc: &Document, source_file: &str) -> SeededGraph {
    let mut seeded = seed_top_level(
        doc,
        source_file,
        CONSTRUCT_KEYS,
        |graph, parent_map, entry_node, key, value, source, anchors, classify| {
            if key == "jobs" || key == "phases" {
                let _ = seed_top_entry_as_job_block(
                    graph, parent_map, entry_node, key, value, source, anchors, classify,
                );
            } else {
                seed_top_entry_as_meta(
                    graph, parent_map, entry_node, key, value, source, anchors, classify,
                );
            }
        },
    );
    hoist_deployment_steps(&mut seeded.graph);
    seeded
}

/// A `deployment:` job carries its steps under a deployment-strategy wrapper:
/// `strategy.runOnce.deploy.steps` (also rolling / canary, hooks preDeploy /
/// routeTraffic / postRouteTraffic / on.*). The strategy wrapper has no hub
/// pendant, so `job.steps ← steps` never fires — the deployment job's steps
/// nest several levels too deep and the job materialises stepless, dropping on
/// emit. Re-link every `steps:` sequence found anywhere under a job's
/// `strategy:` to be a direct `steps:` child of the job, where `job.steps`
/// fires. The strategy/runOnce/deploy wrapper entries stay (inert — no rule
/// matches them); the steps round-trip hoisted onto the job (hub-equal, the
/// steps themselves preserved).
fn hoist_deployment_steps(graph: &mut TypedGraph) {
    let job_ids: Vec<GhostId> = graph
        .iter_nodes()
        .filter(|n| {
            n.type_id == CST_MAPPING
                && n.attrs.get(cst_attr::CONSTRUCT).map(String::as_str) == Some("job")
        })
        .map(|n| n.id)
        .collect();
    // Collect (job, steps-sequence) pairs first (immutable borrow), then link.
    let mut links: Vec<(GhostId, GhostId, String, String)> = Vec::new();
    for job_id in job_ids {
        let Some(strategy_entry) = child_entry(graph, job_id, "strategy") else {
            continue;
        };
        let Some(strategy_val) = outgoing(graph, strategy_entry, CST_VALUE_OF) else {
            continue;
        };
        let mut seqs = Vec::new();
        collect_steps_sequences(graph, strategy_val, &mut seqs);
        for seq in seqs {
            let (s, e) = graph
                .get_node(&seq)
                .map(|n| {
                    (
                        n.attrs
                            .get(cst_attr::SPAN_START)
                            .cloned()
                            .unwrap_or_default(),
                        n.attrs.get(cst_attr::SPAN_END).cloned().unwrap_or_default(),
                    )
                })
                .unwrap_or_default();
            links.push((job_id, seq, s, e));
        }
    }
    for (job_id, seq, span_s, span_e) in links {
        let entry_attrs = make_attrs(&[
            (cst_attr::KEY, "steps"),
            ("entry_role", "annotation"),
            (cst_attr::FROM_MERGE, "false"),
            (cst_attr::SPAN_START, &span_s),
            (cst_attr::SPAN_END, &span_e),
        ]);
        let entry_id = add_child(graph, job_id, CST_HAS_CHILD, CST_MAPPING_ENTRY, entry_attrs);
        // Share the existing steps sequence: a second `value_of` edge from the
        // new job-level entry. job.steps matches job → steps entry → sequence.
        graph.add_edge(entry_id, seq, CST_VALUE_OF, BTreeMap::new(), Status::Solid);
    }
}

/// Recursively gather every `steps:` mapping-entry value that is a
/// `cst:Sequence`, reachable through nested mappings under `node`.
fn collect_steps_sequences(graph: &TypedGraph, node: GhostId, out: &mut Vec<GhostId>) {
    let entries: Vec<GhostId> = graph
        .iter_edges()
        .into_iter()
        .filter(|(s, _, e)| *s == node && e.type_id == CST_HAS_CHILD)
        .filter_map(|(_, t, _)| {
            graph
                .get_node(&t)
                .filter(|n| n.type_id == CST_MAPPING_ENTRY)
                .map(|_| t)
        })
        .collect();
    for entry in entries {
        let key = graph
            .get_node(&entry)
            .and_then(|n| n.attrs.get(cst_attr::KEY).cloned())
            .unwrap_or_default();
        let Some(val) = outgoing(graph, entry, CST_VALUE_OF) else {
            continue;
        };
        let vkind = graph
            .get_node(&val)
            .map(|n| n.type_id.clone())
            .unwrap_or_default();
        if key == "steps" && vkind == CST_SEQUENCE {
            out.push(val);
        } else if vkind == CST_MAPPING {
            collect_steps_sequences(graph, val, out);
        }
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
