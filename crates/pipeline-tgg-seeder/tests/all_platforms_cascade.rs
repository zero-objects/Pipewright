//! Cross-platform cascade validation.
//!
//! For each YAML platform the catalog generates a ruleset for:
//! seed a tiny fixture, run the generated ruleset to convergence,
//! and verify that — at minimum — a `hub:pipeline` node appeared.
//! This is the load-bearing proof that the
//! seeder + classify-table + ruleset chain holds for every
//! platform, not just GitLab.

use std::path::Path;

use pipeline_cst::parse as parse_yaml;
use pipeline_earthfile_cst::parse as parse_earthfile;
use pipeline_jenkinsfile_cst::parse as parse_jenkinsfile;
use pipeline_tgg_seeder::platforms;
use seesaw_core::engine::{cascade_step, Cascade, Rule, TerminationState};
use seesaw_core::graph::TypedGraph;
use seesaw_core::rule::compile::compile_bidirectional;
use seesaw_core::rule::instantiate::instantiate;
use seesaw_core::rule::spec::RuleSetSpec;

/// Kinds present in the graph = the delta for direction routing.
fn graph_kinds(g: &TypedGraph) -> std::collections::HashSet<String> {
    g.iter_nodes().map(|n| n.type_id.clone()).collect()
}

/// `(platform, fixture)` — the simplest YAML the platform's
/// classify table reacts to, so the cascade has something to
/// chew on beyond the bare `hub:pipeline`.
const FIXTURES: &[(&str, &str)] = &[
    ("gitlab", "build:\n  script:\n    - echo hi\n"),
    (
        "github",
        "name: ci\njobs:\n  build:\n    runs-on: ubuntu\n    steps:\n      - run: echo hi\n",
    ),
    (
        "circleci",
        "version: 2.1\njobs:\n  build:\n    steps:\n      - run: echo hi\n",
    ),
    (
        "azure",
        "jobs:\n  - job: build\n    steps:\n      - script: echo hi\n",
    ),
    ("travis", "language: rust\nscript:\n  - cargo build\n"),
    (
        "bitbucket",
        "pipelines:\n  default:\n    - step:\n        script:\n          - echo hi\n",
    ),
    (
        "buildkite",
        "steps:\n  - command: echo hi\n",
    ),
    (
        "drone",
        "kind: pipeline\nname: default\nsteps:\n  - name: build\n    image: alpine\n    commands:\n      - echo hi\n",
    ),
    (
        "woodpecker",
        "steps:\n  build:\n    image: alpine\n    commands:\n      - echo hi\n",
    ),
    (
        "tekton",
        "apiVersion: tekton.dev/v1\nkind: Pipeline\nspec:\n  tasks:\n    - name: build\n      taskRef:\n        name: build-task\n",
    ),
    (
        "argo",
        "apiVersion: argoproj.io/v1alpha1\nkind: Workflow\nspec:\n  templates:\n    - name: build\n      container:\n        image: alpine\n        command: [echo, hi]\n",
    ),
    (
        "google_cloudbuild",
        "steps:\n  - name: gcr.io/cloud-builders/docker\n    args: ['build', '.']\n",
    ),
    (
        "aws_codebuild",
        "version: 0.2\nphases:\n  build:\n    commands:\n      - echo hi\n",
    ),
    (
        "aws_codepipeline",
        "pipeline:\n  name: my-pipeline\n  stages:\n    - name: Build\n      actions:\n        - name: BuildAction\n",
    ),
    // Jenkinsfile DSL — pipeline-jenkinsfile-cst translates this
    // into the same pipeline_cst::Document the YAML parser
    // produces, so the shared seeder applies unchanged.
    (
        "jenkins",
        "pipeline {\n    agent any\n    stages {\n        stage('Build') {\n            steps {\n                sh 'cargo build'\n            }\n        }\n    }\n}\n",
    ),
    // Dagger module manifest — the only declarative part of a
    // Dagger setup (build LOGIC lives in SDK code, out of scope).
    // Real-world dagger.json uses JSON-flow syntax which the YAML
    // parser doesn't accept; block-YAML form is semantically
    // identical and would be the canonical representation if/when
    // a `dagger init` writes block.
    (
        "dagger",
        "name: my-module\nsdk: go\nsource: ./src\n",
    ),
    // Earthfile DSL — pipeline-earthfile-cst translates it into
    // the same pipeline_cst::Document shape so the shared seeder
    // applies. Top-level VERSION/FROM become mapping entries;
    // recipes become named sequences of one-entry mappings.
    (
        "earthly",
        "VERSION 0.8\nFROM rust:1.75\n\nbuild:\n    COPY . .\n    RUN cargo build\n    SAVE ARTIFACT target/release/binary\n",
    ),
];

fn ruleset_path(platform: &str) -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../catalog/rules")
        .join(format!("{platform}.ruleset.json"))
}

struct CascadeStats {
    steps: usize,
    pipelines: usize,
}

fn cascade_platform(platform: &str, src: &str) -> CascadeStats {
    let doc = match platform {
        "jenkins" => parse_jenkinsfile(src)
            .unwrap_or_else(|e| panic!("{platform}: jenkinsfile parse failed: {e:?}")),
        "earthly" => parse_earthfile(src)
            .unwrap_or_else(|e| panic!("{platform}: earthfile parse failed: {e:?}")),
        _ => parse_yaml(src).unwrap_or_else(|e| panic!("{platform}: yaml parse failed: {e:?}")),
    };
    let mut seeded = platforms::seed(platform, &doc, "fixture.yml")
        .unwrap_or_else(|| panic!("{platform}: unknown platform"));

    let json = std::fs::read_to_string(ruleset_path(platform))
        .unwrap_or_else(|e| panic!("{platform}: read ruleset: {e}"));
    let rs: RuleSetSpec =
        serde_json::from_str(&json).unwrap_or_else(|e| panic!("{platform}: deserialize: {e}"));
    // Mirror the real pipeline (`run_routed`): compile BOTH directed rules,
    // then activate only those whose `input_domain_kinds` intersect the
    // seeded graph's kinds. For a forward seed (cst kinds) this selects the
    // forward rules; backward rules (hub idk) stay dormant. Computing the
    // active set once from the initial delta is exactly what the roundtrip
    // does — an unfiltered `compile` run can loop where the routed one
    // converges, so the smoke test must use the same driver it's vouching for.
    let compiled: Vec<_> = rs
        .rules
        .iter()
        .flat_map(|r| {
            compile_bidirectional(r)
                .unwrap_or_else(|e| panic!("{platform}: compile {}: {e:?}", r.name))
        })
        .collect();
    let rules: Vec<Box<dyn Rule>> = compiled.iter().map(instantiate).collect();
    let delta = graph_kinds(&seeded.graph);
    let rule_refs: Vec<&dyn Rule> = rules
        .iter()
        .filter(|r| {
            let idk = r.input_domain_kinds();
            idk.is_empty() || idk.iter().any(|k| delta.contains(k))
        })
        .map(AsRef::as_ref)
        .collect();

    let mut cascade = Cascade::new();
    let mut steps = 0usize;
    loop {
        assert!(steps < 5000, "{platform}: cascade did not converge");
        match cascade_step(&mut cascade, &mut seeded.graph, &rule_refs)
            .unwrap_or_else(|e| panic!("{platform}: cascade step: {e:?}"))
        {
            TerminationState::Running => steps += 1,
            TerminationState::Convergence | TerminationState::Duplication => break,
            TerminationState::Contradiction { reason } => {
                panic!("{platform}: contradiction: {reason}")
            }
        }
    }

    let pipelines = seeded.graph.matchable_nodes_by_kind("hub:pipeline").count();
    // One platform produces more than one hub:pipeline by design:
    //   - bitbucket has the self-referential `pipeline.pipelines`
    //     field (catalog/ir.toml); each named event-gated sub-
    //     pipeline materialises as its own hub:pipeline child of the
    //     outer one.
    // jenkins USED to produce 2 — the seeder double-tagged the outer
    // document shell AND the inner `pipeline { … }` block body. The
    // seeder now HOISTS the block body onto the single shell pipeline
    // (like argo's spec/arguments wrappers), so it produces exactly 1;
    // that is what stopped pick_pipeline_root landing on an empty shell
    // and emitting a bare `pipeline {}` (jenkins wide-stress 17→30/30).
    // Every other platform produces exactly 1.
    let expected = if platform == "bitbucket" { 2 } else { 1 };
    assert_eq!(
        pipelines, expected,
        "{platform}: cascade must materialise {expected} hub:pipeline node(s), got {pipelines}",
    );
    CascadeStats { steps, pipelines }
}

/// Every YAML platform's seeder + generated ruleset chain must
/// produce a hub:pipeline on its simplest fixture. This is the
/// minimal cross-platform smoke test that locks in
/// "all platforms work end-to-end".
#[test]
fn every_platform_cascades_to_a_hub_pipeline() {
    let mut totals: Vec<(&str, CascadeStats)> = Vec::new();
    for (platform, src) in FIXTURES {
        let stats = cascade_platform(platform, src);
        totals.push((platform, stats));
    }
    eprintln!("\n  {:<18} {:>5}  {:>9}", "platform", "steps", "pipelines");
    for (platform, stats) in &totals {
        eprintln!(
            "  {platform:<18} {:>5}  {:>9}",
            stats.steps, stats.pipelines
        );
    }
    assert_eq!(
        totals.len(),
        17,
        "expected all 17 catalogued platforms to be covered",
    );
}
