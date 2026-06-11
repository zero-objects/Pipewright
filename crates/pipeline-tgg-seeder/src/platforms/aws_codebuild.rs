//! aws_codebuild: a buildspec's `phases:` map (install / pre_build / build /
//! post_build) is a CONTAINER of jobs — each phase is a job, its `commands:`
//! list its steps. Every other top-level key is meta; the classify table tags
//! the remaining nested constructs (artifacts / cache).

use crate::{
    classify::aws_codebuild::CONSTRUCT_KEYS, seed_top_entry_as_job_block, seed_top_entry_as_meta,
    seed_top_level, SeededGraph,
};
use pipeline_cst::{Document, NodeKind};

#[must_use]
pub fn seed_from_document(doc: &Document, source_file: &str) -> SeededGraph {
    seed_top_level(
        doc,
        source_file,
        CONSTRUCT_KEYS,
        |graph, parent_map, entry_node, key, value, source, anchors, classify| {
            // `phases:` is a job container: each child entry (install / build /
            // …) is a job whose `commands:` become its steps. Route it through
            // the block-job seeder, which tags each child entry construct=job.
            if (key == "phases" || key == "phase") && matches!(value.kind, NodeKind::Mapping) {
                seed_top_entry_as_job_block(
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
