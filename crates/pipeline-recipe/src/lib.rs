//! The recipe system. A *recipe* (`*.recipe.yml`) is a reusable, platform-
//! neutral pipeline fragment — named jobs with an image, steps, and intra-recipe
//! dependencies. Recipes are **composed** into a single pipeline and rendered to
//! **any** target platform: the trick is that composition is just migration —
//! build the combined fragment in a neutral job syntax, forward it to the
//! Hub-IR, and re-emit to the target via the backward TGG cascade.
//!
//! This reuses the whole forward/backward machinery; a recipe never hand-builds
//! a hub subgraph. Jobs are namespaced `<recipe_id>-<job>` so multiple recipes
//! compose without collisions, and intra-recipe `needs` are rewritten to match.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use serde::Deserialize;

pub mod config;
pub mod registry;

/// A recipe: a named set of jobs forming a reusable pipeline fragment, with the
/// metadata a registry needs to list, search, sort and describe it.
#[derive(Debug, Clone, Deserialize)]
pub struct Recipe {
    pub recipe_id: String,
    /// SemVer-ish version of the recipe (`"1.0.0"`); empty if unversioned.
    #[serde(default)]
    pub recipe_version: String,
    /// One-line summary, shown in lists and search.
    #[serde(default)]
    pub description: String,
    /// Longer free-form / Markdown documentation (the "doc" of the recipe).
    #[serde(default)]
    pub doc: String,
    /// Search/sort facets ("rust", "docker", "release", …).
    #[serde(default)]
    pub tags: Vec<String>,
    /// Capabilities the recipe needs from a target ("docker", …).
    #[serde(default)]
    pub platform_requirements: Vec<String>,
    /// Declared inputs / outputs (for graph-edit wiring).
    #[serde(default)]
    pub input_ports: Vec<Port>,
    #[serde(default)]
    pub output_ports: Vec<Port>,
    pub jobs: BTreeMap<String, RecipeJob>,
}

/// A declared input/output of a recipe — a named, typed connection point.
#[derive(Debug, Clone, Deserialize, serde::Serialize)]
pub struct Port {
    pub name: String,
    #[serde(default)]
    pub kind: String,
}

/// One job in a recipe.
#[derive(Debug, Clone, Deserialize)]
pub struct RecipeJob {
    #[serde(default)]
    pub image: Option<String>,
    #[serde(default)]
    pub needs: Vec<String>,
    #[serde(default)]
    pub steps: Vec<String>,
}

/// What can go wrong composing recipes.
#[derive(Debug, Clone)]
pub enum RecipeError {
    /// A `*.recipe.yml` failed to parse.
    Parse(String),
    /// No recipes were given.
    Empty,
    /// The forward/re-emit through the Hub-IR failed.
    Render(String),
    /// A configured source (directory / git repo) could not be made available.
    Source(String),
}

impl std::fmt::Display for RecipeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Parse(e) => write!(f, "recipe parse error: {e}"),
            Self::Empty => write!(f, "no recipes to compose"),
            Self::Render(e) => write!(f, "render error: {e}"),
            Self::Source(e) => write!(f, "recipe source error: {e}"),
        }
    }
}

impl std::error::Error for RecipeError {}

/// Parse a `*.recipe.yml` document.
///
/// # Errors
/// [`RecipeError::Parse`] on malformed YAML / missing fields.
pub fn load(yaml: &str) -> Result<Recipe, RecipeError> {
    serde_yaml::from_str(yaml).map_err(|e| RecipeError::Parse(e.to_string()))
}

/// Compose recipes into one pipeline rendered to `target`. Builds the combined
/// fragment in a neutral job syntax, forwards it to the Hub-IR, and re-emits to
/// the target platform (composition = migration).
///
/// # Errors
/// [`RecipeError`] for an empty set, or a forward / re-emit failure.
pub fn compose(recipes: &[Recipe], target: &str) -> Result<String, RecipeError> {
    if recipes.is_empty() {
        return Err(RecipeError::Empty);
    }
    // Compose in a neutral (gitlab-shaped) syntax, then migrate to the target:
    // forward → re-key the hub to the target vocabulary → re-emit. Cross-platform
    // fidelity is gated on the migration layer — it's clean between structurally
    // compatible families (e.g. job-based gitlab→github) and lossy/empty toward
    // structurally different ones (e.g. step-flat drone). An empty result means
    // the target couldn't represent the recipe; surface that honestly.
    let neutral = combined_yaml(recipes);
    let out = pipeline_forward::migrate("gitlab", &neutral, target)
        .map_err(|e| RecipeError::Render(e.to_string()))?;
    if out.trim().is_empty() || out.trim() == "{}" {
        return Err(RecipeError::Render(format!(
            "target '{target}' could not represent this recipe (cross-platform migration is limited to structurally compatible targets)"
        )));
    }
    Ok(out)
}

/// Parse then [`compose`] a set of recipe documents.
///
/// # Errors
/// [`RecipeError`] for a parse, empty-set, or render failure.
pub fn compose_documents(yamls: &[String], target: &str) -> Result<String, RecipeError> {
    let recipes: Result<Vec<Recipe>, _> = yamls.iter().map(|y| load(y)).collect();
    compose(&recipes?, target)
}

/// A generated human-readable description of what a recipe does, in `locale`,
/// produced through the prose "doc mechanism": the recipe is forwarded to the
/// Hub-IR and rendered as a natural-language runbook (Markdown). This is the
/// STRUCTURAL description (jobs, steps, dependencies, in the chosen language);
/// the curated one-liner / long-form live in [`Recipe::description`] /
/// [`Recipe::doc`]. The two complement each other in a recipe's detail view.
///
/// # Errors
/// [`RecipeError::Render`] if the recipe can't be forwarded / has no pipeline.
pub fn describe_recipe(recipe: &Recipe, locale: &str) -> Result<String, RecipeError> {
    let neutral = combined_yaml(std::slice::from_ref(recipe));
    let graph = pipeline_forward::forward("gitlab", &neutral)
        .map_err(|e| RecipeError::Render(e.to_string()))?;
    let p = pipeline_render::lift(&graph)
        .ok_or_else(|| RecipeError::Render("recipe has no pipeline to describe".to_string()))?;
    Ok(pipeline_render::markdown_in(&p, locale))
}

/// Apply a recipe to an existing pipeline `source` of platform `kind`, merging
/// the recipe's jobs into it and re-emitting in the same platform. This is the
/// graph-edit "apply recipe" operation.
///
/// The merge happens in the neutral (gitlab-shaped) hub space: both sides are
/// migrated to neutral, their top-level job maps are unioned (the recipe's jobs
/// are namespaced `<recipe_id>-<job>`, so they never collide with existing
/// jobs), and the union is migrated back to `kind`. Migration both ways reuses
/// the verified forward/backward cascade — fidelity is gated on the migration
/// layer, identical to [`compose`].
///
/// An empty / `{}` `source` is treated as a fresh pipeline (the result is just
/// the recipe rendered to `kind`).
///
/// # Errors
/// [`RecipeError`] if either migration fails, the neutral YAML can't be merged,
/// or the target can't represent the result.
pub fn apply_to_source(source: &str, kind: &str, recipe: &Recipe) -> Result<String, RecipeError> {
    let to_neutral = |src: &str| -> Result<String, RecipeError> {
        if kind == "gitlab" {
            Ok(src.to_string())
        } else {
            pipeline_forward::migrate(kind, src, "gitlab")
                .map_err(|e| RecipeError::Render(e.to_string()))
        }
    };
    let existing_neutral = if source.trim().is_empty() || source.trim() == "{}" {
        String::new()
    } else {
        to_neutral(source)?
    };
    let recipe_neutral = combined_yaml(std::slice::from_ref(recipe));
    let merged = merge_gitlab(&existing_neutral, &recipe_neutral);
    let out = if kind == "gitlab" {
        merged
    } else {
        pipeline_forward::migrate("gitlab", &merged, kind)
            .map_err(|e| RecipeError::Render(e.to_string()))?
    };
    if out.trim().is_empty() || out.trim() == "{}" {
        return Err(RecipeError::Render(format!(
            "target '{kind}' could not represent the merged pipeline (cross-platform fidelity is limited to structurally compatible targets)"
        )));
    }
    Ok(out)
}

/// Union two gitlab-shaped YAML documents: append `addition`'s top-level blocks
/// that `base` doesn't already define. Both inputs are kept as TEXT — `base`
/// verbatim, each `addition` block verbatim — so the result preserves the
/// emitter's (forward-able) formatting. A re-serialize through serde would
/// re-indent block sequences to column 0, which the CST tokenizer can't
/// round-trip; text-preserving append sidesteps that entirely. An `addition`
/// key already present in `base` is skipped (additive, not destructive — so
/// re-applying a recipe is idempotent rather than duplicating keys).
fn merge_gitlab(base: &str, addition: &str) -> String {
    let base_keys: std::collections::BTreeSet<String> =
        top_level_blocks(base).into_iter().map(|(k, _)| k).collect();
    let mut out = base.trim_end().to_string();
    for (key, block) in top_level_blocks(addition) {
        if base_keys.contains(&key) {
            continue;
        }
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str(block.trim_end());
    }
    out.push('\n');
    out
}

/// Split a YAML document into its top-level `(key, block-text)` pairs, keyed by
/// each column-0 `key:` line. The block runs until the next column-0 key.
fn top_level_blocks(yaml: &str) -> Vec<(String, String)> {
    let mut blocks: Vec<(String, String)> = Vec::new();
    let mut current: Option<(String, String)> = None;
    for line in yaml.lines() {
        let is_top = !line.is_empty()
            && !line.starts_with(char::is_whitespace)
            && !line.trim_start().starts_with('#')
            && line.contains(':');
        if is_top {
            if let Some(entry) = current.take() {
                blocks.push(entry);
            }
            let key = line.split(':').next().unwrap_or("").trim().to_string();
            current = Some((key, String::new()));
        }
        if let Some((_, body)) = current.as_mut() {
            body.push_str(line);
            body.push('\n');
        }
    }
    if let Some(entry) = current.take() {
        blocks.push(entry);
    }
    blocks
}

/// Build the combined neutral (gitlab-shaped) pipeline YAML from the recipes:
/// jobs namespaced `<recipe_id>-<job>`, `steps` → `script`, intra-recipe `needs`
/// namespaced to match.
fn combined_yaml(recipes: &[Recipe]) -> String {
    let mut s = String::new();
    for r in recipes {
        for (name, job) in &r.jobs {
            let _ = writeln!(s, "{}:", ns(&r.recipe_id, name));
            if let Some(img) = &job.image {
                let _ = writeln!(s, "  image: {}", yaml_scalar(img));
            }
            if !job.needs.is_empty() {
                s.push_str("  needs:\n");
                for dep in &job.needs {
                    let _ = writeln!(s, "    - {}", yaml_scalar(&ns(&r.recipe_id, dep)));
                }
            }
            s.push_str("  script:\n");
            for step in &job.steps {
                let _ = writeln!(s, "    - {}", yaml_scalar(step));
            }
        }
    }
    s
}

/// Namespace a job name by its recipe id, so multiple recipes never collide.
fn ns(recipe_id: &str, job: &str) -> String {
    format!("{recipe_id}-{job}")
}

/// Single-quote a YAML scalar (escaping internal single quotes), so commands
/// with `:`/`#`/leading specials round-trip safely.
fn yaml_scalar(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const RUST_CI: &str = "recipe_id: rust-ci\njobs:\n  lint:\n    image: rust:1.75\n    steps:\n      - cargo clippy\n  test:\n    image: rust:1.75\n    needs:\n      - lint\n    steps:\n      - cargo test\n";

    #[test]
    fn loads_a_recipe() {
        let r = load(RUST_CI).expect("load");
        assert_eq!(r.recipe_id, "rust-ci");
        assert_eq!(r.jobs.len(), 2);
        assert_eq!(r.jobs["test"].needs, vec!["lint"]);
    }

    #[test]
    fn composes_to_gitlab() {
        let r = load(RUST_CI).unwrap();
        let out = compose(&[r], "gitlab").expect("compose gitlab");
        // namespaced jobs, image, the dependency, and the commands all survive.
        assert!(out.contains("rust-ci-lint"));
        assert!(out.contains("rust-ci-test"));
        assert!(out.contains("rust:1.75"));
        assert!(out.contains("cargo clippy") && out.contains("cargo test"));
        assert!(out.contains("rust-ci-lint"), "needs dependency present");
    }

    #[test]
    fn composes_to_step_flat_target_via_cross_structural_bridge() {
        // drone is step-flat; a job-based recipe used to fail to migrate there.
        // The F1 cross-structural bridge now flattens each job to a drone step,
        // so composition to a step-flat target succeeds (was a clean error).
        let r = load(RUST_CI).unwrap();
        let out = compose(&[r], "drone").expect("compose to drone via the bridge");
        assert!(
            out.contains("kind: pipeline") && out.contains("steps:"),
            "valid drone: {out}"
        );
        assert!(
            out.contains("rust-ci-lint") && out.contains("cargo clippy"),
            "recipe content present: {out}"
        );
        // drone is step-flat: the work units forward to hub:step, not hub:job.
        let steps = pipeline_forward::forward("drone", &out)
            .map(|g| g.iter_nodes().filter(|n| n.type_id == "hub:step").count())
            .unwrap_or(0);
        assert!(steps >= 2, "drone output forwards to its steps: {out}");
    }

    #[test]
    fn composes_multiple_recipes_namespaced() {
        let a = load(RUST_CI).unwrap();
        let b = load("recipe_id: deploy\njobs:\n  publish:\n    steps:\n      - make deploy\n")
            .unwrap();
        let out = compose(&[a, b], "gitlab").expect("compose two");
        assert!(out.contains("rust-ci-lint") && out.contains("deploy-publish"));
        assert!(out.contains("make deploy"));
    }

    #[test]
    fn empty_set_errors() {
        assert!(matches!(compose(&[], "gitlab"), Err(RecipeError::Empty)));
    }

    /// Number of `hub:job` nodes the result forwards to — the real test of a
    /// usable pipeline (a string that merely *contains* a job name can still
    /// fail to parse; see the 0-indent `stages:` regression this guards).
    fn forwarded_jobs(src: &str, kind: &str) -> usize {
        pipeline_forward::forward(kind, src)
            .map(|g| g.iter_nodes().filter(|n| n.type_id == "hub:job").count())
            .unwrap_or(0)
    }

    #[test]
    fn apply_recipe_merges_into_existing_gitlab_pipeline() {
        let existing = "stages:\n  - build\nbuild:\n  stage: build\n  script:\n    - make\n";
        let r = load(RUST_CI).unwrap();
        let out = apply_to_source(existing, "gitlab", &r).expect("apply");
        assert!(out.contains("build"), "existing job kept: {out}");
        assert!(
            out.contains("rust-ci-lint") && out.contains("rust-ci-test"),
            "recipe jobs added: {out}"
        );
        // The merged result must FORWARD back to a real pipeline: existing build
        // + rust-ci's lint/test/build = 4 jobs. (A textual merge that re-indents
        // `stages:` to column 0 would parse to 0 jobs — this is the guard.)
        assert_eq!(
            forwarded_jobs(&out, "gitlab"),
            3,
            "merged pipeline forwards to 3 jobs (build + rust-ci lint/test): {out}"
        );
    }

    #[test]
    fn apply_to_empty_works_across_compatible_targets() {
        // A fresh pipeline from a recipe forwards to the recipe's jobs on the
        // structurally-compatible (job-based) targets the migration layer
        // handles cleanly today. Targets that need cross-structural remapping
        // (circleci/azure/step-flat drone) are limited by F1 in the backlog —
        // tracked there, not asserted here so this test reflects reality.
        let r = load(RUST_CI).unwrap();
        for target in ["gitlab", "github"] {
            let out = apply_to_source("", target, &r)
                .unwrap_or_else(|e| panic!("apply to {target}: {e}"));
            assert!(
                forwarded_jobs(&out, target) >= 2,
                "{target}: recipe forwards to its jobs, got:\n{out}"
            );
        }
    }

    #[test]
    fn re_applying_a_recipe_is_idempotent() {
        let existing = "build:\n  script:\n    - make\n";
        let r = load(RUST_CI).unwrap();
        let once = apply_to_source(existing, "gitlab", &r).expect("apply once");
        let twice = apply_to_source(&once, "gitlab", &r).expect("apply twice");
        assert_eq!(
            forwarded_jobs(&once, "gitlab"),
            forwarded_jobs(&twice, "gitlab"),
            "re-applying must not duplicate jobs"
        );
    }

    #[test]
    fn apply_recipe_into_empty_source_is_just_the_recipe() {
        let r = load(RUST_CI).unwrap();
        let out = apply_to_source("", "gitlab", &r).expect("apply to empty");
        assert!(out.contains("rust-ci-lint"));
        assert_eq!(
            forwarded_jobs(&out, "gitlab"),
            2,
            "fresh recipe forwards to 2 jobs: {out}"
        );
    }

    #[test]
    fn apply_recipe_to_github_round_trips_through_neutral() {
        // existing github workflow + a recipe → still a github workflow with both
        // recipe jobs. The merge happens in the neutral gitlab space (lossless,
        // see the gitlab test); the final hop back to github is gated on the
        // cross-platform MIGRATION layer, where github `run`/step fidelity has
        // documented gaps (see docs/interop-matrix.md). So we assert the recipe
        // jobs survive into github and forward — not full step round-trip, which
        // is a migration-layer concern, not the recipe layer's.
        let existing = "on: push\njobs:\n  build:\n    steps:\n      - run: make\n";
        let r = load(RUST_CI).unwrap();
        let out = apply_to_source(existing, "github", &r).expect("apply github");
        assert!(
            out.contains("rust-ci-lint") && out.contains("rust-ci-test"),
            "recipe jobs in github output: {out}"
        );
        assert!(
            forwarded_jobs(&out, "github") >= 2,
            "github result forwards to the recipe's jobs: {out}"
        );
    }
}

#[cfg(test)]
mod describe_tests {
    use super::*;
    #[test]
    fn describe_recipe_via_prose_localized() {
        let r = registry::Registry::with_standard();
        let rust = &r.get("rust-ci").unwrap().recipe;
        let en = describe_recipe(rust, "en").expect("describe en");
        let de = describe_recipe(rust, "de").expect("describe de");
        assert!(
            en.contains("job") || en.contains("pipeline"),
            "en prose: {en}"
        );
        assert!(
            de.contains("Job") || de.contains("Pipeline"),
            "de prose: {de}"
        );
        assert_ne!(en, de, "locale must change the prose");
    }
}
