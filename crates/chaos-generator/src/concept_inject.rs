//! Concept-path injection — guarantees the chaos generator
//! exercises every concept rule declared in `catalog/concepts.toml`.
//!
//! ## Why
//!
//! `walker.rs` builds YAML from random walks over the platform
//! catalog. That picks fields with `optional_prob` (default 30%)
//! and emits enum-without-options fields as the literal string
//! `"enum"`. Result: github's `on: { … }` is generated as `on: {}`
//! or `on: - enum`, neither of which the seeder recognises, and
//! every cross-platform pair targeting github is "A IR empty".
//!
//! Concept rules are the convergence points the IR cares about
//! (`trigger_branch`, `step_image`, `trigger_cron`, …). Each rule
//! anchors a path in CST and binds it to a hub:attr. If we walk
//! a fresh YAML and *override* each declared concept path with a
//! plausible value, chaos exercises the concept rule on every
//! run — no more reliance on the walker rolling a 30% optional
//! into a 30% optional into a 30% optional.
//!
//! ## How
//!
//! `inject(yaml, platform, rng)` reads `catalog/concepts.toml`
//! and for each `(concept, platform_path)` entry whose
//! `platform_path[platform]` is set, navigates `yaml` to the
//! parent of the leaf, creating intermediate mappings as needed,
//! and writes a deterministic sample value at the leaf.
//!
//! Path grammar matches `gen_ruleset.py::concept_rule`:
//!   * `key`     — descend into a map under that key
//!   * `key[]`   — descend into a list under `key`; force one item
//!   * `<>`      — anonymous map key (just synthesise some key)
//!
//! Sample values are hard-coded in `sample_for` — keeping them
//! plausible (real branch names, real cron expressions, real
//! image refs) keeps the seeded YAML readable in failure logs.

use crate::catalog_path;
use rand::seq::SliceRandom;
use rand::Rng;
use serde_yaml::{Mapping, Sequence, Value};
use std::collections::BTreeMap;
use std::fs;

#[derive(Debug, Clone)]
struct ConceptSpec {
    name: String,
    /// `"list"` (the path's leaf is a `cst:Sequence` of scalars)
    /// or `"scalar"` (single scalar). Mirrors the field in
    /// concepts.toml.
    leaf_shape: String,
    /// Per-platform path string, as found in
    /// `[concept.<name>.platform_path]`.
    platform_paths: BTreeMap<String, String>,
}

fn load_concepts() -> Vec<ConceptSpec> {
    // `catalog_path("concepts")` resolves to the repo's
    // `catalog/concepts.toml`. We tolerate the file going missing
    // (returns empty) so older trees without concept declarations
    // still pass through the walker untouched.
    let path = catalog_path("concepts");
    let Ok(text) = fs::read_to_string(&path) else {
        return Vec::new();
    };
    let Ok(doc): Result<toml::Value, _> = toml::from_str(&text) else {
        return Vec::new();
    };
    let Some(concepts_tbl) = doc.get("concept").and_then(|v| v.as_table()) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for (name, body) in concepts_tbl {
        let body = match body.as_table() {
            Some(t) => t,
            None => continue,
        };
        let leaf_shape = body
            .get("leaf_shape")
            .and_then(|v| v.as_str())
            .unwrap_or("list")
            .to_string();
        let mut platform_paths = BTreeMap::new();
        if let Some(pp) = body.get("platform_path").and_then(|v| v.as_table()) {
            for (k, v) in pp {
                if let Some(s) = v.as_str() {
                    platform_paths.insert(k.clone(), s.to_string());
                }
            }
        }
        out.push(ConceptSpec {
            name: name.clone(),
            leaf_shape,
            platform_paths,
        });
    }
    out
}

/// Inject every concept path declared for `platform` into `yaml`.
/// Mutates `yaml` in place. Intended to be called AFTER the
/// random walker has produced the initial document.
pub fn inject(yaml: &mut Value, platform: &str, rng: &mut impl Rng) {
    let concepts = load_concepts();
    for c in concepts {
        if let Some(path) = c.platform_paths.get(platform).cloned() {
            let leaf_value = sample_for(&c.name, &c.leaf_shape, rng);
            write_path(yaml, &path, leaf_value, rng);
        }
    }
}

/// Deterministic sample value for a concept's leaf. Returning
/// real-looking values (not "foo", not "x") helps when a chaos
/// failure dumps YAML — you can read the cron expression and
/// know which rule was being exercised.
fn sample_for(concept_name: &str, leaf_shape: &str, rng: &mut impl Rng) -> Value {
    // The pool is intentionally small (2–3 values per concept)
    // so proptest shrinks reliably and the YAML stays compact.
    let pool: &[&str] = match concept_name {
        "trigger_branch_match" => &["main", "develop"],
        "trigger_tag_match" => &["v*", "release/*"],
        "trigger_path_match" => &["src/**", "docs/**"],
        "trigger_cron" => &["0 2 * * *"],
        "pipeline_default_image" => &["alpine:3.18", "node:20"],
        "step_image" => &["alpine:3.18", "rust:1.75"],
        "step_working_dir" => &["/workspace", "/app"],
        "job_runs_on" => &["ubuntu-latest", "macos-latest"],
        "pipeline_service_account" => &["ci-runner@example.iam.gserviceaccount.com"],
        "pipeline_timeout" => &["600s"],
        "pipeline_env_var_value" => &["debug", "info"],
        "module_sdk" => &["go"],
        "module_engine_version" => &["v0.12.0"],
        _ => &["concept-value"],
    };
    let pick = pool[rng.gen_range(0..pool.len())];
    if leaf_shape == "list" {
        // The L-pattern terminates in cst:Sequence -> Item ->
        // Scalar. Force at least one item, occasionally two —
        // small enough to keep shrinker happy.
        let n = rng.gen_range(1..=2);
        let mut seq = Sequence::new();
        for _ in 0..n {
            let one = pool.choose(rng).copied().unwrap_or(pick);
            seq.push(Value::String(one.to_string()));
        }
        Value::Sequence(seq)
    } else {
        Value::String(pick.to_string())
    }
}

/// Navigate `yaml` along `path`, creating intermediate mappings
/// / sequences as needed, and set the leaf to `leaf`.
///
/// Path tokens:
/// - `key`   — step into the value of `key`, creating an empty
///   mapping if `key` is absent or non-mapping.
/// - `key[]` — `key`'s value is a sequence; ensure at least one item
///   exists; descend into the FIRST item (also forcing it to be a mapping).
/// - `<>`    — anonymous map entry — write under a fresh key
///   (`concept_inject` to avoid clashes with the random walker's own keys).
fn write_path(yaml: &mut Value, path: &str, leaf: Value, rng: &mut impl Rng) {
    let segments: Vec<&str> = path.split('.').collect();
    if segments.is_empty() {
        return;
    }
    let (last, mids) = segments.split_last().expect("non-empty");
    let mut cursor: &mut Value = yaml;
    for seg in mids {
        cursor = descend(cursor, seg, rng);
    }
    write_leaf(cursor, last, leaf);
}

fn descend<'v>(cursor: &'v mut Value, seg: &str, rng: &mut impl Rng) -> &'v mut Value {
    if seg == "<>" {
        // Anonymous map iteration — write under a synthetic key.
        // Reusing the same key across calls is intentional: a
        // concept that fires once per map entry will fire once
        // on this fixed key, which is enough for testing.
        ensure_mapping(cursor);
        let m = cursor.as_mapping_mut().expect("ensured mapping");
        let k = Value::String("__concept_inject".to_string());
        m.entry(k.clone())
            .or_insert_with(|| Value::Mapping(Mapping::new()));
        return m.get_mut(&k).expect("just inserted");
    }
    if let Some(key_part) = seg.strip_suffix("[]") {
        ensure_mapping(cursor);
        let m = cursor.as_mapping_mut().expect("ensured mapping");
        let k = Value::String(key_part.to_string());
        // Materialise as a non-empty sequence of mappings.
        let entry = m
            .entry(k.clone())
            .or_insert_with(|| Value::Sequence(Sequence::new()));
        if !matches!(entry, Value::Sequence(_)) {
            *entry = Value::Sequence(Sequence::new());
        }
        let seq = entry.as_sequence_mut().expect("sequence");
        if seq.is_empty() {
            seq.push(Value::Mapping(Mapping::new()));
        }
        // Descend into the first item, ensuring it's a mapping.
        if !matches!(seq[0], Value::Mapping(_)) {
            seq[0] = Value::Mapping(Mapping::new());
        }
        let _ = rng;
        return &mut seq[0];
    }
    ensure_mapping(cursor);
    let m = cursor.as_mapping_mut().expect("ensured mapping");
    let k = Value::String(seg.to_string());
    let entry = m
        .entry(k.clone())
        .or_insert_with(|| Value::Mapping(Mapping::new()));
    if !matches!(entry, Value::Mapping(_)) {
        *entry = Value::Mapping(Mapping::new());
    }
    m.get_mut(&k).expect("just inserted")
}

fn write_leaf(cursor: &mut Value, seg: &str, leaf: Value) {
    if seg == "<>" {
        // Anonymous leaf: write a single fresh key carrying the
        // leaf value. The `<>` path semantics says "for each
        // entry, the value is the leaf"; injecting one entry is
        // enough for the concept rule to fire.
        ensure_mapping(cursor);
        let m = cursor.as_mapping_mut().expect("ensured mapping");
        m.insert(Value::String("__concept_inject_leaf".to_string()), leaf);
        return;
    }
    if let Some(key_part) = seg.strip_suffix("[]") {
        // Leaf ends in `[]` — concept_rule rejects this case;
        // injector mirrors that by writing the leaf as a one-item
        // sequence under the key.
        ensure_mapping(cursor);
        let m = cursor.as_mapping_mut().expect("ensured mapping");
        let seq = Sequence::from(vec![leaf]);
        m.insert(Value::String(key_part.to_string()), Value::Sequence(seq));
        return;
    }
    ensure_mapping(cursor);
    let m = cursor.as_mapping_mut().expect("ensured mapping");
    m.insert(Value::String(seg.to_string()), leaf);
}

fn ensure_mapping(v: &mut Value) {
    if !matches!(v, Value::Mapping(_)) {
        *v = Value::Mapping(Mapping::new());
    }
}
