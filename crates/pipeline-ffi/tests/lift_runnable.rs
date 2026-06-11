//! R6: every "Full" (locally-runnable) platform must lift jobs that carry real
//! commands — otherwise `run` would silently do nothing. This guards against
//! lift regressions like the buildkite/gcb bugs found during the run-honesty
//! pass.

use std::path::Path;

/// Lift `src` for `plat` and return each job's (name, command-labels).
fn lift_jobs(plat: &str, src: &str) -> Vec<(String, Vec<String>)> {
    let g = pipeline_forward::forward(plat, src).expect("forward");
    pipeline_render::lift(&g)
        .expect("lift")
        .jobs
        .iter()
        .map(|j| {
            (
                j.name.clone(),
                j.steps.iter().map(|s| s.label.clone()).collect(),
            )
        })
        .collect()
}

fn corpus(plat: &str) -> String {
    let base = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/cross_corpus")
        .join(plat);
    let f = std::fs::read_dir(&base)
        .unwrap_or_else(|e| panic!("no corpus dir {}: {e}", base.display()))
        .flatten()
        .map(|e| e.path())
        .find(|p| p.is_file())
        .unwrap_or_else(|| panic!("no fixture in {}", base.display()));
    std::fs::read_to_string(f).unwrap()
}

/// The platforms whose pipelines run locally (container + shell). Mirrors
/// `pipeline_forward::RunSupport::Full` — kept in lockstep by R6.6's test below.
const RUNNABLE: &[&str] = &[
    "gitlab",
    "github",
    "drone",
    "woodpecker",
    "bitbucket",
    "circleci",
    "azure",
    "travis",
    "aws_codebuild",
    "google_cloudbuild",
    "buildkite",
];

#[test]
fn every_runnable_platform_lifts_commands() {
    for &plat in RUNNABLE {
        let jobs = lift_jobs(plat, &corpus(plat));
        assert!(!jobs.is_empty(), "{plat}: lifted no jobs");
        let with_cmd = jobs
            .iter()
            .filter(|(_, cmds)| cmds.iter().any(|c| !c.trim().is_empty() && c != "(step)"))
            .count();
        assert!(
            with_cmd > 0,
            "{plat}: no job carries a runnable command — jobs: {jobs:?}"
        );
    }
}

#[test]
fn run_support_matches_runnable_reality() {
    use pipeline_forward::{run_support, RunSupport, PLATFORMS};
    // Every platform classified Full must be in the locally-verified RUNNABLE
    // list, and vice versa — the classification can't drift from what lifts.
    for &plat in PLATFORMS {
        let full = run_support(plat) == RunSupport::Full;
        assert_eq!(
            full,
            RUNNABLE.contains(&plat),
            "{plat}: run_support Full={full} but RUNNABLE membership differs"
        );
        // TranslateOnly platforms must carry a non-empty reason.
        if !full {
            assert!(
                !RunSupport::reason(plat).is_empty(),
                "{plat}: no TranslateOnly reason"
            );
        }
    }
}

#[test]
fn buildkite_lifts_label_command_and_depends() {
    // The bug: label/command hung off an inner hub:step behind hub:item_element;
    // build_step_node read the empty outer wrapper → name "(step)", 0 commands.
    let jobs = lift_jobs(
        "buildkite",
        "steps:\n  - label: build\n    command: cargo build --release\n  - label: test\n    command: cargo test\n    depends_on: build\n",
    );
    let names: Vec<&str> = jobs.iter().map(|(n, _)| n.as_str()).collect();
    assert!(
        names.contains(&"build") && names.contains(&"test"),
        "labels → names: {jobs:?}"
    );
    let build = jobs.iter().find(|(n, _)| n == "build").unwrap();
    assert_eq!(build.1, vec!["cargo build --release"], "command → step");
}

#[test]
fn gcb_joins_argv_into_one_command_with_image_and_id() {
    // Cloud Build: `name:` is the image, `id:` the step name, `args:` argv
    // tokens for ONE command (must be joined in source order, not split).
    let g = pipeline_forward::forward(
        "google_cloudbuild",
        "steps:\n  - id: build\n    name: rust:1.75\n    args: [cargo, build, --release]\n  - id: test\n    name: rust:1.75\n    args: [cargo, test]\n    waitFor: [build]\n",
    )
    .unwrap();
    let p = pipeline_render::lift(&g).unwrap();
    let build = p
        .jobs
        .iter()
        .find(|j| j.name == "build")
        .expect("id → name");
    assert_eq!(
        build
            .steps
            .iter()
            .map(|s| s.label.as_str())
            .collect::<Vec<_>>(),
        vec!["cargo build --release"],
        "argv joined in order"
    );
    assert!(
        build
            .params
            .iter()
            .any(|pp| pp.key == "image" && pp.value == "rust:1.75"),
        "name → image: {:?}",
        build.params
    );
    let test = p.jobs.iter().find(|j| j.name == "test").unwrap();
    assert_eq!(test.needs, vec!["build"], "waitFor → needs");
}
