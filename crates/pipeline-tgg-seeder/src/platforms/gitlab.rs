//! GitLab: every non-meta top-level mapping-entry is a job.

use crate::{
    classify::gitlab::{CONSTRUCT_KEYS, LIST_CONSTRUCT_KEYS},
    seed_top_entry_as_job, seed_top_entry_as_meta, seed_top_level_with_list_fields, SeededGraph,
};
use pipeline_cst::{Document, NodeKind};

/// `.gitlab-ci.yml` top-level keys reserved by the platform — NOT
/// job names.
pub const META_KEYS: &[&str] = &[
    "stages",
    "default",
    "variables",
    "workflow",
    "include",
    // `spec:` is the pipeline-inputs header (maps to pipeline.parameters),
    // a reserved top-level keyword — never a job name.
    "spec",
    "image",
    "services",
    "cache",
    "before_script",
    "after_script",
    "interruptible",
    "retry",
    "timeout",
];

#[must_use]
pub fn seed_from_document(doc: &Document, source_file: &str) -> SeededGraph {
    seed_top_level_with_list_fields(
        doc,
        source_file,
        CONSTRUCT_KEYS,
        LIST_CONSTRUCT_KEYS,
        |graph, parent_map, entry_node, key, value, source, anchors, classify| {
            if key == "default" && matches!(value.kind, NodeKind::Mapping) {
                // `default:` is a TRANSPARENT defaults wrapper — its children
                // (image / cache / before_script / services / timeout / …) are
                // pipeline-level defaults. HOIST them onto the pipeline (like
                // argo's spec/arguments) so each lands on its real field
                // (pipeline.image/cache/hooks/…); the old map_nodes →
                // pipeline.defaults captured only the flat scalars and dropped
                // the nested cache + before_script lists.
                for child in &value.children {
                    if let NodeKind::MappingEntry { key_text } = &child.kind {
                        if child.children.len() < 2 {
                            continue;
                        }
                        seed_top_entry_as_meta(
                            graph,
                            parent_map,
                            child,
                            key_text,
                            &child.children[1],
                            source,
                            anchors,
                            classify,
                        );
                    }
                }
            } else if META_KEYS.contains(&key) {
                seed_top_entry_as_meta(
                    graph, parent_map, entry_node, key, value, source, anchors, classify,
                );
            } else {
                seed_top_entry_as_job(
                    graph, parent_map, entry_node, key, value, source, anchors, classify,
                );
            }
        },
    )
    // No lift: the unified hub:value scalar_attr rules read the CST scalar
    // directly (key-gated, bijective). The earlier first-class form needed
    // the lift to place MC.<key>; that form is gone.
}
