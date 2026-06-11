//! Dagger module manifest (`dagger.json`). JSON is a strict
//! subset of YAML so `pipeline_cst::parse` handles it directly;
//! the classify table is tiny because Dagger's build LOGIC lives
//! in SDK code (Go / Python / TypeScript) which is not in scope
//! for a static-config translator.

use crate::{
    classify::dagger::CONSTRUCT_KEYS, seed_top_entry_as_meta, seed_top_level, SeededGraph,
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
