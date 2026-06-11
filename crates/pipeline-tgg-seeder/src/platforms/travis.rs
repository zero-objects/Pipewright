//! Travis CI: largely flat — `script`, `before_script`, `env`,
//! `services`, … are pipeline-level. Multi-job builds use
//! `jobs.include` (a sequence). Treat top-level entries as meta;
//! the classify table tags `script`/`stages` etc.; the
//! implicit-containment rule materialises `pipeline → step`.

use crate::{
    classify::travis::CONSTRUCT_KEYS, seed_top_entry_as_job_block, seed_top_entry_as_meta,
    seed_top_level, SeededGraph,
};
use pipeline_cst::Document;

#[must_use]
pub fn seed_from_document(doc: &Document, source_file: &str) -> SeededGraph {
    seed_top_level(
        doc,
        source_file,
        CONSTRUCT_KEYS,
        // `jobs:` is a MAP of jobs (jobs.<name>: {...}), not a
        // single job mapping. The classify table says `("jobs",
        // "job")` so without this dispatch the outer container
        // mapping gets tagged `construct=job` and the inner job
        // entries don't — exactly mirroring github's job-block
        // shape. Re-using github's helper strips the mistagged
        // outer tag and tags each child entry value as construct=job.
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
