//! Rule-coverage report — opt-in (`cargo test rule_coverage_report -- --ignored --nocapture`).
//!
//! For every platform with a passing chaos suite, run N seeds
//! forward through the cascade, collect the set of rule names
//! that fired, and diff against the full ruleset. Output lists
//! the rules that never fired across the entire sample — those
//! are the under-tested (or dead) corners of the platform's TGG
//! ruleset.
//!
//! Not part of CI: it's a diagnostic, not a pass/fail. Gating CI
//! on coverage threshold would lock in whatever the current
//! sample happens to exercise, which is the opposite of useful.

use chaos_generator::{
    coverage::{all_rule_names, merge, run_and_record, unseen},
    generate_yaml,
    walker::Budget,
};
use pipeline_cst::parse;
use seesaw_core::graph::TypedGraph;
use seesaw_core::rule::spec::RuleSetSpec;
use std::collections::BTreeSet;
use std::path::Path;

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
        "azure" => p::azure::seed_from_document(doc, source).graph,
        "travis" => p::travis::seed_from_document(doc, source).graph,
        "bitbucket" => p::bitbucket::seed_from_document(doc, source).graph,
        "aws_codebuild" => p::aws_codebuild::seed_from_document(doc, source).graph,
        "aws_codepipeline" => p::aws_codepipeline::seed_from_document(doc, source).graph,
        _ => panic!("unsupported: {platform}"),
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
];

const N_SEEDS: u64 = 100;

#[test]
#[ignore = "diagnostic — run with `cargo test rule_coverage_report -- --ignored --nocapture`"]
fn rule_coverage_report() {
    let budgets = [Budget::shallow(), Budget::deep()];

    println!(
        "\n── Rule-Coverage Report ── {} seeds × {} budgets per platform ──",
        N_SEEDS,
        budgets.len()
    );
    println!(
        "{:>20}  {:>6} / {:>6}  {:>6}  uncovered",
        "platform", "fired", "total", "%"
    );

    let mut grand_uncovered: usize = 0;
    let mut grand_total: usize = 0;
    let mut details: Vec<(String, Vec<String>)> = Vec::new();

    for &platform in PLATFORMS {
        let rs = ruleset(platform);
        let all = all_rule_names(&rs);
        let mut fired: BTreeSet<String> = BTreeSet::new();
        for seed in 0..N_SEEDS {
            for budget in &budgets {
                let yaml = match generate_yaml(platform, seed, budget) {
                    Ok(y) => y,
                    Err(_) => continue,
                };
                let doc = match parse(&yaml) {
                    Ok(d) => d,
                    Err(_) => continue,
                };
                let mut g = seed_for(platform, &doc, &yaml);
                let seen = run_and_record(&mut g, &rs, 5_000);
                merge(&mut fired, seen);
            }
        }
        let uncovered = unseen(&fired, &all);
        let pct = if all.is_empty() {
            100.0
        } else {
            (fired.len() as f64 / all.len() as f64) * 100.0
        };
        println!(
            "{:>20}  {:>6} / {:>6}  {:>5.1}%  {}",
            platform,
            fired.len(),
            all.len(),
            pct,
            uncovered.len(),
        );
        grand_total += all.len();
        grand_uncovered += uncovered.len();
        details.push((platform.to_string(), uncovered));
    }

    let grand_fired = grand_total - grand_uncovered;
    let grand_pct = (grand_fired as f64 / grand_total as f64) * 100.0;
    println!(
        "{:>20}  {:>6} / {:>6}  {:>5.1}%  {}",
        "TOTAL", grand_fired, grand_total, grand_pct, grand_uncovered,
    );

    println!("\n── Top under-covered platforms (up to 5 uncovered each) ──");
    let mut by_uncov: Vec<_> = details.iter().filter(|(_, u)| !u.is_empty()).collect();
    by_uncov.sort_by_key(|(_, u)| std::cmp::Reverse(u.len()));
    for (plat, uncov) in by_uncov.iter().take(5) {
        println!("\n{plat} ({} uncovered):", uncov.len());
        for name in uncov.iter().take(8) {
            println!("  - {name}");
        }
        if uncov.len() > 8 {
            println!("  ... ({} more)", uncov.len() - 8);
        }
    }
}
