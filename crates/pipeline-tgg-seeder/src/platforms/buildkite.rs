//! Buildkite: flat pipeline — `steps:` is a sequence of step
//! objects. Classify-driven recursive walk handles the steps.

use crate::{
    classify::buildkite::CONSTRUCT_KEYS, seed_top_entry_as_meta, seed_top_level, SeededGraph,
};
use pipeline_cst::Document;

#[must_use]
pub fn seed_from_document(doc: &Document, source_file: &str) -> SeededGraph {
    seed_top_level(
        doc,
        source_file,
        CONSTRUCT_KEYS,
        |graph, parent_map, entry_node, key, value, source, anchors, classify| {
            seed_top_entry_as_meta(
                graph, parent_map, entry_node, key, value, source, anchors, classify,
            );
        },
    )
}
