//! Argo Workflows: same Kubernetes-resource shape as Tekton —
//! apiVersion / kind / metadata at the top, real workflow body
//! under `spec:`. The seeder flattens spec children into the
//! outer pipeline mapping so the catalog-driven field rules can
//! anchor on them without needing a `spec:` wrapper in the IR.
//!
//! `spec.templates[]` are seeded GENERICALLY: each template is a
//! polymorphism-helper `construct=job` carrying optional body fields
//! (container/script/steps/dag/resource/suspend/inputs/outputs/…),
//! every one of which the catalog `job.field.*` rules map to a hub
//! job attribute. No body is flattened — the template body keys are
//! recursively seeded as-is so each variant round-trips losslessly.

use crate::{
    add_child, classify::argo::CONSTRUCT_KEYS, cst_attr, make_attrs, open_pipeline, seed_value,
    synthesize_name_carrier, AnchorTable, SeededGraph, CST_HAS_CHILD, CST_MAPPING,
    CST_MAPPING_ENTRY, CST_SEQUENCE, CST_SEQUENCE_ITEM, CST_VALUE_OF,
};
use pipeline_cst::{Document, Node, NodeKind};
use seesaw_core::graph::{GhostId, TypedGraph};

/// Create a `cst:MappingEntry` for `key` on `parent` and seed its value via the
/// generic classify-driven walk (used for every template/task body key except
/// the ones we hoist).
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

/// Name carrier from a mapping's `name:` entry, if it is a scalar.
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

/// Seed `spec.templates[]` as the pipeline's JOB list, with the `dag:` body
/// HOISTED: a dag template is a job whose `dag.tasks[]` ARE its dependency
/// edges (job.needs). Looking one level higher than the `dag` wrapper, each
/// task → a `construct=dependency_edge` under a `tasks` entry on the job (so
/// job.needs ← tasks links them), and each task's transparent `arguments:`
/// wrapper is hoisted so its `parameters` round-trip as the edge's parameters.
/// Non-dag bodies (container/script/steps/resource/…) seed generically.
fn seed_argo_templates(
    graph: &mut TypedGraph,
    templates_entry_id: GhostId,
    list: &Node,
    source: &str,
    anchors: &AnchorTable<'_>,
) {
    let seq_attrs = make_attrs(&[
        (cst_attr::SPAN_START, &list.span.start.to_string()),
        (cst_attr::SPAN_END, &list.span.end.to_string()),
    ]);
    let seq_id = add_child(
        graph,
        templates_entry_id,
        CST_VALUE_OF,
        CST_SEQUENCE,
        seq_attrs,
    );
    for (i, item) in list.children.iter().enumerate() {
        let NodeKind::SequenceItem = &item.kind else {
            continue;
        };
        let Some(tmpl) = item.children.first() else {
            continue;
        };
        if !matches!(tmpl.kind, NodeKind::Mapping) {
            continue;
        }
        let item_attrs = make_attrs(&[
            ("index", &i.to_string()),
            (cst_attr::SPAN_START, &item.span.start.to_string()),
            (cst_attr::SPAN_END, &item.span.end.to_string()),
        ]);
        let item_id = add_child(graph, seq_id, CST_HAS_CHILD, CST_SEQUENCE_ITEM, item_attrs);
        let job_attrs = make_attrs(&[
            (cst_attr::SPAN_START, &tmpl.span.start.to_string()),
            (cst_attr::SPAN_END, &tmpl.span.end.to_string()),
        ]);
        let job_id = add_child(graph, item_id, CST_VALUE_OF, CST_MAPPING, job_attrs);
        graph.set_node_attr(&job_id, cst_attr::CONSTRUCT, "job");
        for tentry in &tmpl.children {
            let NodeKind::MappingEntry { key_text } = &tentry.kind else {
                continue;
            };
            if tentry.children.len() < 2 {
                continue;
            }
            let tval = &tentry.children[1];
            if key_text == "dag" && matches!(tval.kind, NodeKind::Mapping) {
                // `dag: {tasks: […]}` — hoist its tasks onto the job.
                if let Some(tasks) = tval.children.iter().find_map(|e| {
                    let NodeKind::MappingEntry { key_text } = &e.kind else {
                        return None;
                    };
                    (key_text == "tasks")
                        .then(|| e.children.get(1))
                        .flatten()
                        .filter(|v| matches!(v.kind, NodeKind::Sequence))
                }) {
                    seed_argo_tasks(graph, job_id, tasks, source, anchors);
                }
            } else if key_text == "tasks" && matches!(tval.kind, NodeKind::Sequence) {
                // The emitted (hoisted) form — `tasks:` directly on the job;
                // seed identically to dag.tasks so the round-trip is symmetric.
                seed_argo_tasks(graph, job_id, tval, source, anchors);
            } else if matches!(key_text.as_str(), "inputs" | "outputs")
                && matches!(tval.kind, NodeKind::Mapping)
            {
                // `inputs:`/`outputs:` are transparent wrappers holding
                // `parameters:`/`artifacts:` lists — hoist their children onto
                // the job so job.parameters ← parameters / job.artifacts ←
                // artifacts anchor on them (a mapping_node onto the wrapper
                // would leave the inner lists orphaned).
                for wentry in &tval.children {
                    let NodeKind::MappingEntry { key_text: wkey } = &wentry.kind else {
                        continue;
                    };
                    if wentry.children.len() < 2 {
                        continue;
                    }
                    seed_body_entry(
                        graph,
                        job_id,
                        wkey,
                        &wentry.children[1],
                        wentry,
                        source,
                        anchors,
                    );
                }
            } else {
                seed_body_entry(graph, job_id, key_text, tval, tentry, source, anchors);
            }
        }
        if let Some(n) = name_of(tmpl, source) {
            synthesize_name_carrier(graph, job_id, "job", n, tmpl.span.start, tmpl.span.end);
        }
    }
}

/// Seed a `tasks` sequence (from `dag.tasks` OR the emitted flat `tasks:`) as
/// the job's dependency edges: a `tasks` entry whose items are
/// `construct=dependency_edge` (job.needs ← tasks). Each task's `arguments:`
/// wrapper is hoisted so its `parameters` land on the edge.
fn seed_argo_tasks(
    graph: &mut TypedGraph,
    job_id: GhostId,
    tasks: &Node,
    source: &str,
    anchors: &AnchorTable<'_>,
) {
    let tasks_entry_attrs = make_attrs(&[
        (cst_attr::KEY, "tasks"),
        ("entry_role", "annotation"),
        (cst_attr::FROM_MERGE, "false"),
        (cst_attr::SPAN_START, &tasks.span.start.to_string()),
        (cst_attr::SPAN_END, &tasks.span.end.to_string()),
    ]);
    let tasks_entry_id = add_child(
        graph,
        job_id,
        CST_HAS_CHILD,
        CST_MAPPING_ENTRY,
        tasks_entry_attrs,
    );
    let seq_attrs = make_attrs(&[
        (cst_attr::SPAN_START, &tasks.span.start.to_string()),
        (cst_attr::SPAN_END, &tasks.span.end.to_string()),
    ]);
    let seq_id = add_child(graph, tasks_entry_id, CST_VALUE_OF, CST_SEQUENCE, seq_attrs);
    for (i, item) in tasks.children.iter().enumerate() {
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
        let it_id = add_child(graph, seq_id, CST_HAS_CHILD, CST_SEQUENCE_ITEM, item_attrs);
        let edge_attrs = make_attrs(&[
            (cst_attr::SPAN_START, &task.span.start.to_string()),
            (cst_attr::SPAN_END, &task.span.end.to_string()),
        ]);
        let edge_id = add_child(graph, it_id, CST_VALUE_OF, CST_MAPPING, edge_attrs);
        graph.set_node_attr(&edge_id, cst_attr::CONSTRUCT, "dependency_edge");
        for sentry in &task.children {
            let NodeKind::MappingEntry { key_text } = &sentry.kind else {
                continue;
            };
            if sentry.children.len() < 2 {
                continue;
            }
            let sval = &sentry.children[1];
            // `arguments:` is a transparent wrapper — hoist its parameters
            // onto the edge (parameters → dependency_edge.parameters).
            if key_text == "arguments" && matches!(sval.kind, NodeKind::Mapping) {
                for aentry in &sval.children {
                    let NodeKind::MappingEntry { key_text: akey } = &aentry.kind else {
                        continue;
                    };
                    if aentry.children.len() < 2 {
                        continue;
                    }
                    seed_body_entry(
                        graph,
                        edge_id,
                        akey,
                        &aentry.children[1],
                        aentry,
                        source,
                        anchors,
                    );
                }
            } else {
                seed_body_entry(graph, edge_id, key_text, sval, sentry, source, anchors);
            }
        }
        if let Some(n) = name_of(task, source) {
            synthesize_name_carrier(
                graph,
                edge_id,
                "dependency_edge",
                n,
                task.span.start,
                task.span.end,
            );
        }
    }
}

/// Seed one entry as a direct child of the pipeline mapping `outer_map_id`:
/// create its `cst:MappingEntry` (role `meta:<key>`) and seed its value. Used
/// for the genuine top-level/spec entries AND for the children hoisted out of
/// argo's transparent `arguments:` wrapper.
#[allow(
    clippy::too_many_arguments,
    reason = "load-bearing seeding context; a struct would just rename the fields"
)]
fn seed_pipeline_entry(
    graph: &mut TypedGraph,
    outer_map_id: GhostId,
    key: &str,
    anchor: Option<&str>,
    value: &Node,
    entry_span: (usize, usize),
    source: &str,
    anchors: &crate::AnchorTable<'_>,
) {
    let role = format!("meta:{key}");
    let attrs = make_attrs(&[
        (cst_attr::KEY, key),
        ("entry_role", &role),
        (cst_attr::FROM_MERGE, "false"),
        (cst_attr::SPAN_START, &entry_span.0.to_string()),
        (cst_attr::SPAN_END, &entry_span.1.to_string()),
    ]);
    let entry_id = crate::add_child(graph, outer_map_id, CST_HAS_CHILD, CST_MAPPING_ENTRY, attrs);
    seed_value(
        graph,
        entry_id,
        key,
        anchor,
        value,
        source,
        anchors,
        CONSTRUCT_KEYS,
    );
}

#[must_use]
#[allow(
    clippy::too_many_lines,
    reason = "argo spec/templates flattening is one cohesive seeding pass"
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
                for spec_entry in &value.children {
                    if let NodeKind::MappingEntry {
                        key_text: inner_key,
                    } = &spec_entry.kind
                    {
                        if spec_entry.children.len() < 2 {
                            continue;
                        }
                        let inner_value = &spec_entry.children[1];
                        // `arguments:` is a transparent wrapper holding
                        // `parameters:` / `artifacts:` lists. Hoist its children
                        // to the pipeline level so the pipeline.parameters /
                        // pipeline.artifacts seq_mapping_nodes rules anchor on
                        // them directly — the wrapper itself carries no hub
                        // construct, and a mapping_node onto it would leave the
                        // inner lists orphaned (dropped on emit).
                        if inner_key == "arguments" && matches!(inner_value.kind, NodeKind::Mapping)
                        {
                            for arg_entry in &inner_value.children {
                                if let NodeKind::MappingEntry { key_text: arg_key } =
                                    &arg_entry.kind
                                {
                                    if arg_entry.children.len() < 2 {
                                        continue;
                                    }
                                    seed_pipeline_entry(
                                        &mut seed.graph,
                                        outer_map_id,
                                        arg_key,
                                        arg_entry.anchor.as_deref(),
                                        &arg_entry.children[1],
                                        (arg_entry.span.start, arg_entry.span.end),
                                        source,
                                        &seed.anchors,
                                    );
                                }
                            }
                            continue;
                        }
                        if inner_key == "templates"
                            && matches!(inner_value.kind, NodeKind::Sequence)
                        {
                            // Custom path: templates → jobs, with the `dag:`
                            // body hoisted to job.needs (see seed_argo_templates).
                            let attrs = make_attrs(&[
                                (cst_attr::KEY, "templates"),
                                ("entry_role", "meta:templates"),
                                (cst_attr::FROM_MERGE, "false"),
                                (cst_attr::SPAN_START, &spec_entry.span.start.to_string()),
                                (cst_attr::SPAN_END, &spec_entry.span.end.to_string()),
                            ]);
                            let entry_id = add_child(
                                &mut seed.graph,
                                outer_map_id,
                                CST_HAS_CHILD,
                                CST_MAPPING_ENTRY,
                                attrs,
                            );
                            seed_argo_templates(
                                &mut seed.graph,
                                entry_id,
                                inner_value,
                                source,
                                &seed.anchors,
                            );
                            continue;
                        }
                        seed_pipeline_entry(
                            &mut seed.graph,
                            outer_map_id,
                            inner_key,
                            spec_entry.anchor.as_deref(),
                            inner_value,
                            (spec_entry.span.start, spec_entry.span.end),
                            source,
                            &seed.anchors,
                        );
                    }
                }
                continue;
            }
            // The EMITTED form is spec-hoisted (templates/parameters appear at
            // the top level, no `spec:` wrapper), so route a top-level
            // `templates:` through the SAME custom path as the spec one — else
            // the dag→needs hoist only happens on the first forward and the
            // round-trip is asymmetric.
            if key_text == "templates" && matches!(value.kind, NodeKind::Sequence) {
                let attrs = make_attrs(&[
                    (cst_attr::KEY, "templates"),
                    ("entry_role", "meta:templates"),
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
                seed_argo_templates(&mut seed.graph, entry_id, value, source, &seed.anchors);
                continue;
            }
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
