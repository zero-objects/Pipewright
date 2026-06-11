//! Tekton: the pipeline body lives under `spec:` per the
//! Kubernetes-resource convention. The seeder keeps the outer
//! mapping tagged construct=pipeline (so apiVersion / kind round-
//! trip as pipeline-level fields via the catalog) and lifts every
//! child of `spec:` to be a direct child of the outer mapping —
//! the spec wrapper disappears from the hub-graph side and is
//! reconstructed on emit by the platform-aware reverse path.

use crate::{
    add_child, classify::tekton::CONSTRUCT_KEYS, cst_attr, make_attrs, open_pipeline, seed_value,
    synthesize_name_carrier, AnchorTable, SeededGraph, CST_HAS_CHILD, CST_MAPPING,
    CST_MAPPING_ENTRY, CST_SEQUENCE, CST_SEQUENCE_ITEM, CST_VALUE_OF,
};
use pipeline_cst::{Document, Node, NodeKind};
use seesaw_core::graph::{GhostId, TypedGraph};

/// Create a `cst:MappingEntry` for `key` on `parent` and seed its value via the
/// generic classify-driven walk.
fn seed_body_entry(
    graph: &mut TypedGraph,
    parent: GhostId,
    key: &str,
    value: &Node,
    entry: &Node,
    source: &str,
    anchors: &AnchorTable<'_>,
) {
    let attrs = make_attrs(&[
        (cst_attr::KEY, key),
        ("entry_role", "annotation"),
        (cst_attr::FROM_MERGE, "false"),
        (cst_attr::SPAN_START, &entry.span.start.to_string()),
        (cst_attr::SPAN_END, &entry.span.end.to_string()),
    ]);
    let entry_id = add_child(graph, parent, CST_HAS_CHILD, CST_MAPPING_ENTRY, attrs);
    seed_value(
        graph,
        entry_id,
        key,
        entry.anchor.as_deref(),
        value,
        source,
        anchors,
        CONSTRUCT_KEYS,
    );
}

fn name_of<'a>(node: &'a Node, source: &'a str) -> Option<&'a str> {
    node.children.iter().find_map(|e| {
        let NodeKind::MappingEntry { key_text } = &e.kind else {
            return None;
        };
        if key_text != "name" {
            return None;
        }
        e.children.get(1).and_then(|v| match v.kind {
            NodeKind::Scalar { .. } => Some(source[v.span.start..v.span.end].trim()),
            _ => None,
        })
    })
}

/// Seed `spec.tasks[]` (or the emitted flat `tasks:`) as the pipeline's JOB
/// list. A tekton pipeline task IS a job; its inline `taskSpec:` is a
/// transparent wrapper whose body (steps, workspaces, …) is HOISTED onto the
/// job so taskSpec.steps → job.steps. The emitted form has those keys directly
/// on the task, seeded identically for a symmetric round-trip.
fn seed_tekton_tasks(
    graph: &mut TypedGraph,
    tasks_entry_id: GhostId,
    list: &Node,
    finally: Option<&Node>,
    source: &str,
    anchors: &AnchorTable<'_>,
) {
    let seq_attrs = make_attrs(&[
        (cst_attr::SPAN_START, &list.span.start.to_string()),
        (cst_attr::SPAN_END, &list.span.end.to_string()),
    ]);
    let seq_id = add_child(graph, tasks_entry_id, CST_VALUE_OF, CST_SEQUENCE, seq_attrs);
    // `finally:` tasks are also jobs (cleanup tasks with their own taskSpec).
    // Merge them into the same job list — the finally/tasks split is a tekton
    // surface distinction the flat hub:pipeline.jobs doesn't carry, so it
    // round-trips as one task list (hub-equal, the jobs themselves preserved).
    let items = list
        .children
        .iter()
        .chain(finally.into_iter().flat_map(|f| f.children.iter()));
    for (i, item) in items.enumerate() {
        let NodeKind::SequenceItem = &item.kind else {
            continue;
        };
        let Some(task) = item.children.first() else {
            continue;
        };
        if !matches!(task.kind, NodeKind::Mapping) {
            continue;
        }
        let item_attrs = make_attrs(&[
            ("index", &i.to_string()),
            (cst_attr::SPAN_START, &item.span.start.to_string()),
            (cst_attr::SPAN_END, &item.span.end.to_string()),
        ]);
        let item_id = add_child(graph, seq_id, CST_HAS_CHILD, CST_SEQUENCE_ITEM, item_attrs);
        let job_attrs = make_attrs(&[
            (cst_attr::SPAN_START, &task.span.start.to_string()),
            (cst_attr::SPAN_END, &task.span.end.to_string()),
        ]);
        let job_id = add_child(graph, item_id, CST_VALUE_OF, CST_MAPPING, job_attrs);
        graph.set_node_attr(&job_id, cst_attr::CONSTRUCT, "job");
        for tentry in &task.children {
            let NodeKind::MappingEntry { key_text } = &tentry.kind else {
                continue;
            };
            if tentry.children.len() < 2 {
                continue;
            }
            let tval = &tentry.children[1];
            if key_text == "taskSpec" && matches!(tval.kind, NodeKind::Mapping) {
                // Hoist ONLY taskSpec.steps onto the job. taskSpec's other keys
                // (workspaces DECLARATIONS, params, results) would collide with
                // the task-level workspace BINDINGS / params — leave them under
                // the (unmapped) taskSpec so they drop symmetrically.
                for sentry in &tval.children {
                    let NodeKind::MappingEntry { key_text: skey } = &sentry.kind else {
                        continue;
                    };
                    if skey != "steps" || sentry.children.len() < 2 {
                        continue;
                    }
                    seed_body_entry(
                        graph,
                        job_id,
                        skey,
                        &sentry.children[1],
                        sentry,
                        source,
                        anchors,
                    );
                }
            } else {
                seed_body_entry(graph, job_id, key_text, tval, tentry, source, anchors);
            }
        }
        if let Some(n) = name_of(task, source) {
            synthesize_name_carrier(graph, job_id, "job", n, task.span.start, task.span.end);
        }
    }
}

#[must_use]
#[allow(
    clippy::too_many_lines,
    reason = "tekton task/taskSpec seeding is one cohesive pass"
)]
pub fn seed_from_document(doc: &Document, source_file: &str) -> SeededGraph {
    let Some(mut seed) = open_pipeline(doc, source_file) else {
        let source = doc.source();
        let (graph, doc_id) = crate::make_document(source_file, source);
        return SeededGraph { graph, doc_id };
    };
    let source = doc.source();
    let outer_map_id = seed.map_id;

    for entry in &seed.top_mapping.children {
        if let NodeKind::MappingEntry { key_text } = &entry.kind {
            if entry.children.len() < 2 {
                continue;
            }
            let value = &entry.children[1];
            if key_text == "spec" && matches!(value.kind, NodeKind::Mapping) {
                // Splice the spec body's entries directly into the
                // outer pipeline mapping. Each spec child becomes
                // a top-level pipeline entry (tasks, finally,
                // params, …) and walks the normal classify-driven
                // path from there.
                for spec_entry in &value.children {
                    if let NodeKind::MappingEntry {
                        key_text: inner_key,
                    } = &spec_entry.kind
                    {
                        if spec_entry.children.len() < 2 {
                            continue;
                        }
                        let inner_value = &spec_entry.children[1];
                        let role = format!("meta:{inner_key}");
                        let attrs = make_attrs(&[
                            (cst_attr::KEY, inner_key),
                            ("entry_role", &role),
                            (cst_attr::FROM_MERGE, "false"),
                            (cst_attr::SPAN_START, &spec_entry.span.start.to_string()),
                            (cst_attr::SPAN_END, &spec_entry.span.end.to_string()),
                        ]);
                        let entry_id = crate::add_child(
                            &mut seed.graph,
                            outer_map_id,
                            CST_HAS_CHILD,
                            CST_MAPPING_ENTRY,
                            attrs,
                        );
                        if inner_key == "finally" {
                            // Merged into `tasks` (see seed_tekton_tasks).
                            continue;
                        }
                        if inner_key == "tasks" && matches!(inner_value.kind, NodeKind::Sequence) {
                            // Custom path: tasks (+ finally) → jobs, taskSpec
                            // hoisted (see seed_tekton_tasks).
                            let finally = value.children.iter().find_map(|e| {
                                let NodeKind::MappingEntry { key_text } = &e.kind else {
                                    return None;
                                };
                                (key_text == "finally")
                                    .then(|| e.children.get(1))
                                    .flatten()
                                    .filter(|v| matches!(v.kind, NodeKind::Sequence))
                            });
                            seed_tekton_tasks(
                                &mut seed.graph,
                                entry_id,
                                inner_value,
                                finally,
                                source,
                                &seed.anchors,
                            );
                        } else {
                            seed_value(
                                &mut seed.graph,
                                entry_id,
                                inner_key,
                                spec_entry.anchor.as_deref(),
                                inner_value,
                                source,
                                &seed.anchors,
                                CONSTRUCT_KEYS,
                            );
                        }
                    }
                }
                continue;
            }
            // The EMITTED form is spec-hoisted — a top-level `tasks:` must take
            // the SAME custom path or the taskSpec hoist only happens on the
            // first forward (asymmetric).
            if key_text == "finally" {
                continue;
            }
            if key_text == "tasks" && matches!(value.kind, NodeKind::Sequence) {
                let attrs = make_attrs(&[
                    (cst_attr::KEY, "tasks"),
                    ("entry_role", "meta:tasks"),
                    (cst_attr::FROM_MERGE, "false"),
                    (cst_attr::SPAN_START, &entry.span.start.to_string()),
                    (cst_attr::SPAN_END, &entry.span.end.to_string()),
                ]);
                let entry_id = add_child(
                    &mut seed.graph,
                    outer_map_id,
                    CST_HAS_CHILD,
                    CST_MAPPING_ENTRY,
                    attrs,
                );
                let finally = seed.top_mapping.children.iter().find_map(|e| {
                    let NodeKind::MappingEntry { key_text } = &e.kind else {
                        return None;
                    };
                    (key_text == "finally")
                        .then(|| e.children.get(1))
                        .flatten()
                        .filter(|v| matches!(v.kind, NodeKind::Sequence))
                });
                seed_tekton_tasks(
                    &mut seed.graph,
                    entry_id,
                    value,
                    finally,
                    source,
                    &seed.anchors,
                );
                continue;
            }
            // Other top-level entries (apiVersion, kind, metadata)
            // attach to the outer pipeline mapping verbatim.
            let role = format!("meta:{key_text}");
            let attrs = make_attrs(&[
                (cst_attr::KEY, key_text),
                ("entry_role", &role),
                (cst_attr::FROM_MERGE, "false"),
                (cst_attr::SPAN_START, &entry.span.start.to_string()),
                (cst_attr::SPAN_END, &entry.span.end.to_string()),
            ]);
            let entry_id = crate::add_child(
                &mut seed.graph,
                outer_map_id,
                CST_HAS_CHILD,
                CST_MAPPING_ENTRY,
                attrs,
            );
            seed_value(
                &mut seed.graph,
                entry_id,
                key_text,
                entry.anchor.as_deref(),
                value,
                source,
                &seed.anchors,
                CONSTRUCT_KEYS,
            );
        }
    }
    // Confirm the outer mapping carries the pipeline tag (open_pipeline
    // sets it; defensive in case of malformed input).
    if seed
        .graph
        .get_node(&outer_map_id)
        .is_some_and(|n| n.type_id == CST_MAPPING)
    {
        seed.graph
            .set_node_attr(&outer_map_id, cst_attr::CONSTRUCT, "pipeline");
    }

    SeededGraph {
        graph: seed.graph,
        doc_id: seed.doc_id,
    }
}
