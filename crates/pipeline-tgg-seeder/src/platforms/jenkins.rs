//! Jenkins declarative pipelines — Jenkinsfile DSL parsed by
//! `pipeline-jenkinsfile-cst` into `pipeline_cst::Document`. The
//! DSL wraps the whole config in a `pipeline { ... }` block, so the
//! parsed document looks like `{pipeline: {agent, stages, ...}}`.
//! The seeder retags the inner mapping (the body of the
//! `pipeline {}` block) as construct=pipeline so the rest of the
//! catalog-driven walker sees the pipeline structure where it
//! expects to. Other top-level keys (rare in Jenkinsfile) get the
//! generic meta treatment.

use crate::{
    classify::jenkins::CONSTRUCT_KEYS, seed_top_entry_as_meta, seed_top_level, SeededGraph,
};
use pipeline_cst::{Document, NodeKind};

#[must_use]
pub fn seed_from_document(doc: &Document, source_file: &str) -> SeededGraph {
    seed_top_level(
        doc,
        source_file,
        CONSTRUCT_KEYS,
        |graph, parent_map, entry_node, key, value, source, anchors, classify| {
            if key == "pipeline" && matches!(value.kind, NodeKind::Mapping) {
                // The Jenkinsfile `pipeline { … }` block is a TRANSPARENT
                // wrapper (exactly like argo's `spec:`): HOIST its body entries
                // onto the one pipeline mapping the outer shell already carries
                // (open_pipeline tagged it construct=pipeline). Tagging the
                // inner body as a SECOND construct=pipeline split the hub into
                // two hub:pipeline nodes — the content on one, an empty shell on
                // the other — and pick_pipeline_root could land on the empty
                // shell, emitting a bare `pipeline {}`.
                for body_entry in &value.children {
                    if let NodeKind::MappingEntry { key_text } = &body_entry.kind {
                        if body_entry.children.len() < 2 {
                            continue;
                        }
                        seed_top_entry_as_meta(
                            graph,
                            parent_map,
                            body_entry,
                            key_text,
                            &body_entry.children[1],
                            source,
                            anchors,
                            classify,
                        );
                    }
                }
            } else {
                seed_top_entry_as_meta(
                    graph, parent_map, entry_node, key, value, source, anchors, classify,
                );
            }
        },
    )
}
