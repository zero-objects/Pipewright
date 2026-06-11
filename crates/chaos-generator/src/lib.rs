//! Chaos generator for CI/CD pipeline fixtures.
//!
//! Takes a `catalog/<platform>.toml` schema and a seed, emits a
//! syntactically valid platform document (YAML for the 14 tier-1/2/3
//! targets; native syntax for tier-4 still TBD). Used to drive
//! property-based roundtrip tests: any seed should survive
//! `parse → seed → forward → reverse → emit → parse → forward`
//! with the IR semantically equivalent on both ends.
//!
//! Why not write fixtures by hand: the catalog already encodes every
//! valid construct + type + required-flag, so the chaos generator
//! is the *only* way to systematically exercise the long tail
//! (union arms that rarely appear, deeply nested optional sections,
//! …). Hand-written fixtures undertest by definition.

pub mod concept_inject;
pub mod coverage;
pub mod render;
pub mod spec;
pub mod walker;

use rand_chacha::ChaCha8Rng;
use std::path::Path;

/// Resolve the canonical catalog root for `<platform>.toml`.
pub fn catalog_path(platform: &str) -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../catalog")
        .join(format!("{platform}.toml"))
}

/// Section name that the platform's root document maps to. Most
/// YAML platforms use the platform name itself, drone uses
/// `kind_pipeline` because its top-level switches on `kind:`.
pub fn root_section(platform: &str) -> &'static str {
    // The section name a top-level document maps to. Cross-checked
    // against `grep "^\[" catalog/<platform>.toml`; the upstream
    // schemas vary in casing/naming, so this is platform-specific.
    match platform {
        "drone" => "kind_pipeline",
        "woodpecker" => "pipeline",
        "github" => "pipeline",
        "gitlab" => "pipeline",
        "circleci" => "pipeline",
        "azure" => "pipelineBase", // azure.toml has no [pipeline]; pipelineBase is the parameterised root
        "travis" => "pipeline",
        "bitbucket" => "pipeline",
        "buildkite" => "pipeline",
        "tekton" => "pipeline",
        "argo" => "workflow",
        "aws_codebuild" => "buildspec",
        "aws_codepipeline" => "pipeline",
        "google_cloudbuild" => "pipeline",
        "dagger" => "module",
        "earthly" => "earthfile",
        _ => "pipeline",
    }
}

/// One-shot: generate a YAML document for `platform` with the given
/// seed and budget. Returns the serialised string ready to feed
/// the seeder.
pub fn generate_yaml(
    platform: &str,
    seed: u64,
    budget: &walker::Budget,
) -> Result<String, GenError> {
    generate_yaml_inner(platform, seed, budget, false)
}

/// Like `generate_yaml` but ALSO injects every concept path
/// declared for `platform` in `catalog/concepts.toml` with a
/// plausible sample value. Forces concept rules to fire on every
/// generated fixture — the cross-platform matrix uses this so
/// convergence rules actually get exercised. Chaos self-roundtrip
/// stays on the unaugmented form so it continues testing the
/// random walker's output rather than the injection's.
pub fn generate_yaml_with_concepts(
    platform: &str,
    seed: u64,
    budget: &walker::Budget,
) -> Result<String, GenError> {
    generate_yaml_inner(platform, seed, budget, true)
}

fn generate_yaml_inner(
    platform: &str,
    seed: u64,
    budget: &walker::Budget,
    inject_concepts: bool,
) -> Result<String, GenError> {
    use rand::SeedableRng;
    let spec = spec::load(&catalog_path(platform)).map_err(GenError::Spec)?;
    let root = root_section(platform);
    let mut rng = ChaCha8Rng::seed_from_u64(seed);
    let mut value = walker::gen_section(&spec, root, &mut rng, budget);
    if inject_concepts {
        // Ensure every concept rule has a path to match against.
        // The walker alone misses concept paths because most
        // intermediate keys are optional fields in the catalog
        // (the github `on:` field is an enum-without-options, the
        // walker fills it with the literal string "enum" — concept
        // rules looking for `on.push.branches` never fire).
        concept_inject::inject(&mut value, platform, &mut rng);
    }
    let wrapped = wrap_for_platform(platform, value);
    match platform {
        "earthly" => render::earthfile::render(&wrapped).map_err(GenError::Render),
        "jenkins" => render::jenkinsfile::render(&wrapped).map_err(GenError::Render),
        _ => render::yaml::render(&wrapped).map_err(GenError::Render),
    }
}

/// Some platforms expect a Kubernetes-resource envelope
/// (`apiVersion`/`kind`/`metadata`/`spec`) the bare spec walker
/// can't construct on its own — it would have to know that
/// `spec:` is special. Apply the wrap here.
fn wrap_for_platform(platform: &str, body: serde_yaml::Value) -> serde_yaml::Value {
    use serde_yaml::{Mapping, Value};
    let (api, kind) = match platform {
        "tekton" => ("tekton.dev/v1", "Pipeline"),
        "argo" => ("argoproj.io/v1alpha1", "Workflow"),
        _ => return body,
    };
    // Strip envelope keys from the inner body — tekton/argo
    // pipeline catalogs list apiVersion/kind/metadata as fields
    // because the seeder hoists them to top-level for roundtrip,
    // but the chaos generator shouldn't emit them inside `spec:`
    // alongside the envelope-set ones (the dual values then mix
    // in the forward → reverse path, A vs B's satellite snapshot
    // diverges, and the test fails on the version string).
    let mut inner = if let Value::Mapping(m) = body {
        m
    } else {
        Mapping::new()
    };
    inner.remove(Value::String("apiVersion".into()));
    inner.remove(Value::String("kind".into()));
    inner.remove(Value::String("metadata".into()));

    let mut m = Mapping::new();
    m.insert(
        Value::String("apiVersion".into()),
        Value::String(api.into()),
    );
    m.insert(Value::String("kind".into()), Value::String(kind.into()));
    let mut meta = Mapping::new();
    meta.insert(Value::String("name".into()), Value::String("chaos".into()));
    m.insert(Value::String("metadata".into()), Value::Mapping(meta));
    m.insert(Value::String("spec".into()), Value::Mapping(inner));
    Value::Mapping(m)
}

#[derive(Debug, thiserror::Error)]
pub enum GenError {
    #[error(transparent)]
    Spec(#[from] spec::SpecError),
    #[error("render: {0}")]
    Render(String),
}
