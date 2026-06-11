//! GitHub Actions: jobs nest inside the top-level `jobs:` block.

use crate::{
    classify::github::CONSTRUCT_KEYS, seed_top_entry_as_job_block, seed_top_entry_as_meta,
    seed_top_level, SeededGraph,
};
use pipeline_cst::Document;

#[must_use]
pub fn seed_from_document(doc: &Document, source_file: &str) -> SeededGraph {
    seed_top_level(
        doc,
        source_file,
        CONSTRUCT_KEYS,
        |graph, parent_map, entry_node, key, value, source, anchors, classify| {
            if key == "jobs" {
                let _ = seed_top_entry_as_job_block(
                    graph, parent_map, entry_node, key, value, source, anchors, classify,
                );
            } else {
                seed_top_entry_as_meta(
                    graph, parent_map, entry_node, key, value, source, anchors, classify,
                );
            }
        },
    )
}
