//! AWS CodePipeline: the entire pipeline body is wrapped under a
//! top-level `pipeline:` key (the spec lives under
//! `{pipeline: {name, stages, ...}}`). The seeder flattens that
//! wrapper — the children of `pipeline:` become direct entries on
//! the outer mapping — and version metadata stays at the outer
//! level. Same shape pattern as the K8s envelope on tekton/argo.

use crate::{
    classify::aws_codepipeline::CONSTRUCT_KEYS, cst_attr, make_attrs, open_pipeline, seed_value,
    SeededGraph, CST_HAS_CHILD, CST_MAPPING, CST_MAPPING_ENTRY,
};
use pipeline_cst::{Document, NodeKind};

#[must_use]
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
            if key_text == "pipeline" && matches!(value.kind, NodeKind::Mapping) {
                // Splice the inner pipeline body's entries directly
                // onto the outer mapping.
                for inner_entry in &value.children {
                    if let NodeKind::MappingEntry {
                        key_text: inner_key,
                    } = &inner_entry.kind
                    {
                        if inner_entry.children.len() < 2 {
                            continue;
                        }
                        let inner_value = &inner_entry.children[1];
                        let role = format!("meta:{inner_key}");
                        let attrs = make_attrs(&[
                            (cst_attr::KEY, inner_key),
                            ("entry_role", &role),
                            (cst_attr::FROM_MERGE, "false"),
                            (cst_attr::SPAN_START, &inner_entry.span.start.to_string()),
                            (cst_attr::SPAN_END, &inner_entry.span.end.to_string()),
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
                            inner_key,
                            inner_entry.anchor.as_deref(),
                            inner_value,
                            source,
                            &seed.anchors,
                            CONSTRUCT_KEYS,
                        );
                    }
                }
                continue;
            }
            // Other top-level entries (very rare for codepipeline,
            // mostly never present at root) attach verbatim.
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
