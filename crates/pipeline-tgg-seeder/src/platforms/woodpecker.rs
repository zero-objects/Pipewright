//! Woodpecker: `steps:` is either a LIST or a MAPPING of step
//! definitions (image, commands, …). Woodpecker steps are
//! containerised units modelled as hub:step (bijective; the
//! job-nesting is a cross-platform translation concern). The normal
//! classify-driven walk tags them construct=step in both forms:
//! a list via the sequence-item path, a map via is_map_construct
//! (the `steps:` key is `map:step` in the classify table).

use crate::{
    classify::woodpecker::CONSTRUCT_KEYS, seed_top_entry_as_meta, seed_top_level, SeededGraph,
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
