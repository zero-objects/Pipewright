//! Rule-coverage breakdown by category.
//!
//! The flat "3.4% rules fired" number doesn't say WHICH rules are
//! dead. This test categorises every rule in every platform's
//! ruleset and reports per-category coverage so we can tell
//! whether the gap is "carrier comments never fire in chaos" or
//! "the generator never hits this whole rule family".
//!
//! Categories (matched against rule name suffixes / patterns):
//!   * `construct` — `R_<plat>_<C>` (rank 90, ties a tagged
//!     cst:Mapping to its hub:<C>)
//!   * `user_comment` — preserves `# foo` comments
//!   * `attr_carrier` — `# @hub:<C>.<F>=<v>` for scalar field
//!   * `ref_attr_carrier` — depth-2 carriers
//!   * `implicit` — implicit-containment rules
//!   * `field` — native field rules (scalar_attr, seq_*, map_*,
//!     mapping_node, block_attr, …)

use chaos_generator::{coverage::run_and_record, generate_yaml, walker::Budget};
use pipeline_cst::parse as parse_yaml;
use pipeline_earthfile_cst::parse as parse_earthfile;
use pipeline_jenkinsfile_cst::parse as parse_jenkinsfile;
use seesaw_core::graph::TypedGraph;
use seesaw_core::rule::spec::{RuleSetSpec, RuleSpec};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

fn parse_for(platform: &str, src: &str) -> Result<pipeline_cst::Document, String> {
    match platform {
        "earthly" => parse_earthfile(src).map_err(|e| format!("{e:?}")),
        "jenkins" => parse_jenkinsfile(src).map_err(|e| format!("{e:?}")),
        _ => parse_yaml(src).map_err(|e| format!("{e:?}")),
    }
}

fn ruleset(platform: &str) -> RuleSetSpec {
    let p = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../catalog/rules")
        .join(format!("{platform}.ruleset.json"));
    serde_json::from_str(&std::fs::read_to_string(&p).unwrap()).unwrap()
}

fn seed_for(platform: &str, doc: &pipeline_cst::Document, source: &str) -> TypedGraph {
    use pipeline_tgg_seeder::platforms as p;
    match platform {
        "drone" => p::drone::seed_from_document(doc, source).graph,
        "woodpecker" => p::woodpecker::seed_from_document(doc, source).graph,
        "buildkite" => p::buildkite::seed_from_document(doc, source).graph,
        "tekton" => p::tekton::seed_from_document(doc, source).graph,
        "argo" => p::argo::seed_from_document(doc, source).graph,
        "google_cloudbuild" => p::google_cloudbuild::seed_from_document(doc, source).graph,
        "gitlab" => p::gitlab::seed_from_document(doc, source).graph,
        "github" => p::github::seed_from_document(doc, source).graph,
        "circleci" => p::circleci::seed_from_document(doc, source).graph,
        "azure" => p::azure::seed_from_document(doc, source).graph,
        "travis" => p::travis::seed_from_document(doc, source).graph,
        "bitbucket" => p::bitbucket::seed_from_document(doc, source).graph,
        "aws_codebuild" => p::aws_codebuild::seed_from_document(doc, source).graph,
        "aws_codepipeline" => p::aws_codepipeline::seed_from_document(doc, source).graph,
        "dagger" => p::dagger::seed_from_document(doc, source).graph,
        "earthly" => p::earthly::seed_from_document(doc, source).graph,
        "jenkins" => p::jenkins::seed_from_document(doc, source).graph,
        _ => panic!(),
    }
}

const PLATFORMS: &[&str] = &[
    "drone",
    "woodpecker",
    "buildkite",
    "tekton",
    "argo",
    "google_cloudbuild",
    "github",
    "azure",
    "aws_codebuild",
    "aws_codepipeline",
    "travis",
    "gitlab",
    "bitbucket",
    "circleci",
    "dagger",
    "earthly",
    "jenkins",
];

#[derive(Debug, Default, Clone)]
struct Counts {
    total: usize,
    fired: usize,
}

fn categorise(rule: &RuleSpec) -> &'static str {
    let name = &rule.name;
    let doc = rule.documentation.as_deref().unwrap_or("");
    if doc.contains("tagged construct=") {
        return "construct";
    }
    if doc.contains("user `# foo` comment") {
        return "user_comment";
    }
    if name.ends_with("_ref_carrier") {
        return "ref_attr_carrier";
    }
    if name.ends_with("_carrier") {
        return "attr_carrier";
    }
    if name.ends_with("_implicit") {
        return "implicit";
    }
    "field"
}

#[test]
#[ignore = "diagnostic — run with `cargo test rule_coverage_breakdown -- --ignored --nocapture`"]
fn rule_coverage_breakdown() {
    // Seeds tunable via env var so push-for-95%-coverage runs
    // don't need a recompile. Default 20 is the quick diagnostic;
    // 200+ is the "real" coverage measurement (~5-10 min); 2000+
    // is the nightly stress-coverage target.
    let n_seeds: u64 = std::env::var("COVERAGE_SEEDS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(20);
    // Shallow + deep budgets — deep produces richer fixtures that
    // exercise nested optionals and union arms the shallow walker
    // never reaches. Add only when COVERAGE_SEEDS is set above the
    // default (otherwise the diagnostic stays runnable in seconds).
    let budgets: Vec<Budget> = if n_seeds > 50 {
        vec![Budget::shallow(), Budget::deep()]
    } else {
        vec![Budget::shallow()]
    };

    println!(
        "\n── Rule coverage by category (×{n_seeds} seeds × {} budgets) ──",
        budgets.len()
    );
    println!(
        "{:>20} {:>14} {:>14} {:>14} {:>14} {:>14} {:>14}",
        "platform",
        "construct",
        "user_comment",
        "attr_carrier",
        "ref_attr_carrier",
        "implicit",
        "field"
    );

    let mut category_grand: BTreeMap<&'static str, Counts> = BTreeMap::new();
    let mut platform_dead_field: BTreeMap<&'static str, Vec<String>> = BTreeMap::new();

    for &platform in PLATFORMS {
        let rs = ruleset(platform);
        let mut fired: BTreeSet<String> = BTreeSet::new();
        for seed in 0..n_seeds {
            for budget in &budgets {
                let Ok(yaml) = generate_yaml(platform, seed, budget) else {
                    continue;
                };
                let Ok(doc) = parse_for(platform, &yaml) else {
                    continue;
                };
                let mut g = seed_for(platform, &doc, &yaml);
                let seen = run_and_record(&mut g, &rs, 5_000);
                fired.extend(seen);
            }
        }

        let mut per_cat: BTreeMap<&'static str, Counts> = BTreeMap::new();
        for r in &rs.rules {
            let c = categorise(r);
            let cnts = per_cat.entry(c).or_default();
            cnts.total += 1;
            if fired.contains(&r.name) {
                cnts.fired += 1;
            }
            let g = category_grand.entry(c).or_default();
            g.total += 1;
            if fired.contains(&r.name) {
                g.fired += 1;
            }
            if c == "field" && !fired.contains(&r.name) {
                platform_dead_field
                    .entry(platform)
                    .or_default()
                    .push(r.name.clone());
            }
        }
        let fmt = |cat: &str| -> String {
            let c = per_cat.get(cat).cloned().unwrap_or_default();
            if c.total == 0 {
                "-".into()
            } else {
                format!("{}/{}", c.fired, c.total)
            }
        };
        println!(
            "{:>20} {:>14} {:>14} {:>14} {:>14} {:>14} {:>14}",
            platform,
            fmt("construct"),
            fmt("user_comment"),
            fmt("attr_carrier"),
            fmt("ref_attr_carrier"),
            fmt("implicit"),
            fmt("field"),
        );
    }

    println!("\n── Grand total per category ──");
    for (cat, cnts) in &category_grand {
        let pct = if cnts.total == 0 {
            100.0
        } else {
            (cnts.fired as f64 / cnts.total as f64) * 100.0
        };
        println!(
            "  {:<20} {:>5}/{:<5}  {:>5.1}%",
            cat, cnts.fired, cnts.total, pct
        );
    }

    println!("\n── Why are construct rules barely firing? ──");
    // Construct rules fire when CST has cst:Mapping[construct=<C>].
    // List all construct rules per platform and whether they fire.
    for &platform in PLATFORMS {
        let rs = ruleset(platform);
        let mut fired: BTreeSet<String> = BTreeSet::new();
        for seed in 0..n_seeds {
            for budget in &budgets {
                let Ok(yaml) = generate_yaml(platform, seed, budget) else {
                    continue;
                };
                let Ok(doc) = parse_for(platform, &yaml) else {
                    continue;
                };
                let mut g = seed_for(platform, &doc, &yaml);
                let seen = run_and_record(&mut g, &rs, 5_000);
                fired.extend(seen);
            }
        }
        let construct_rules: Vec<_> = rs
            .rules
            .iter()
            .filter(|r| categorise(r) == "construct")
            .collect();
        let unfired: Vec<&str> = construct_rules
            .iter()
            .filter(|r| !fired.contains(&r.name))
            .map(|r| r.name.as_str())
            .collect();
        if unfired.is_empty() {
            continue;
        }
        println!(
            "  {platform}: dead construct rules ({} of {}):",
            unfired.len(),
            construct_rules.len()
        );
        for n in unfired.iter().take(8) {
            println!("    - {n}");
        }
        if unfired.len() > 8 {
            println!("    ... ({} more)", unfired.len() - 8);
        }
    }
}
