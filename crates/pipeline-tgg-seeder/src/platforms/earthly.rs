//! Earthly: a target (`name:` whose value is a recipe — after the CST reshape,
//! a `cst:Mapping` of `VERB args` command entries) is a JOB; the recipe's
//! commands become the job's steps / image / artifacts via the ruleset (it
//! matches `RUN`/`COPY`/`FROM`/`SAVE ARTIFACT`/… inside a `construct=job`
//! mapping). Top-level verb lines (`VERSION`, `FROM`, `ARG`, …) whose value is a
//! scalar are pipeline-level meta.

use crate::{
    classify::earthly::CONSTRUCT_KEYS, seed_top_entry_as_job, seed_top_entry_as_meta,
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
            // A target's value is a recipe mapping WITH at least one command →
            // it's a job; the recipe's commands become its steps/image/artifacts.
            // A top-level verb's value is a scalar → pipeline meta. An EMPTY
            // recipe (no commands) is NOT seeded as a job: other keyless
            // platforms (gitlab) don't seed a content-less top-level key as a
            // job, so seeding it here would make a degenerate empty job
            // round-trip inconsistently (present via earthly, absent via gitlab →
            // interop drift). Treat it as meta, matching gitlab.
            let nonempty_recipe = matches!(value.kind, NodeKind::Mapping)
                && value
                    .children
                    .iter()
                    .any(|c| matches!(c.kind, NodeKind::MappingEntry { .. }));
            if nonempty_recipe {
                seed_top_entry_as_job(
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
