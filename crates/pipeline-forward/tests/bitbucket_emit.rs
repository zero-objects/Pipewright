//! F2 stage 1: bitbucket re-emit must reconstruct the `pipelines.default`
//! containment — each job re-nested as a `- step:` wrapper item — and the
//! emitted YAML must re-forward to the same jobs (non-vacuous fixpoint).

fn jobs(src: &str) -> Vec<String> {
    pipeline_forward::forward("bitbucket", src)
        .ok()
        .and_then(|g| pipeline_render::lift(&g))
        .map(|p| p.jobs.iter().map(|j| j.name.clone()).collect())
        .unwrap_or_default()
}

#[test]
fn bitbucket_parallel_steps_lift_and_re_emit() {
    // F2 stage 2a: `- parallel:` groups — jobs lift AND re-emit re-nests
    // the parallel wrapper (non-vacuous fixpoint via the grouped hub item).
    let src = "pipelines:\n  default:\n    - step:\n        name: lint\n        script:\n          - cargo clippy\n    - parallel:\n        - step:\n            name: test\n            script:\n              - cargo test\n        - step:\n            name: docs\n            script:\n              - cargo doc\n";
    let mut names = jobs(src);
    names.sort();
    assert_eq!(
        names,
        vec!["docs", "lint", "test"],
        "parallel members lift as jobs"
    );
    let g = pipeline_forward::forward("bitbucket", src).unwrap();
    let out = pipeline_forward::re_emit("bitbucket", &g).unwrap();
    assert!(
        out.contains("parallel:"),
        "re-emit keeps the parallel group:\n{out}"
    );
    let mut names2 = jobs(&out);
    names2.sort();
    assert_eq!(
        names2,
        vec!["docs", "lint", "test"],
        "emitted YAML re-forwards to the same jobs:\n{out}"
    );
}

#[test]
fn bitbucket_branch_steps_lift_with_condition_and_re_emit() {
    // F2 stage 2b: `pipelines.branches.<name>` — jobs lift carrying the
    // branch as condition, and re-emit reconstructs the named selector map.
    let src = "pipelines:\n  default:\n    - step:\n        name: build\n        script:\n          - cargo build\n  branches:\n    main:\n      - step:\n          name: deploy\n          script:\n            - make deploy\n";
    let g = pipeline_forward::forward("bitbucket", src).unwrap();
    let p = pipeline_render::lift(&g).unwrap();
    let deploy = p
        .jobs
        .iter()
        .find(|j| j.name == "deploy")
        .expect("branch job lifts");
    assert_eq!(
        deploy.condition.as_deref(),
        Some("branch: main"),
        "branch key surfaces as condition"
    );
    let out = pipeline_forward::re_emit("bitbucket", &g).unwrap();
    assert!(
        out.contains("branches:") && out.contains("main:"),
        "re-emit reconstructs the branches map:\n{out}"
    );
    let mut names = jobs(&out);
    names.sort();
    assert_eq!(
        names,
        vec!["build", "deploy"],
        "emitted YAML re-forwards to the same jobs:\n{out}"
    );
}

#[test]
fn bitbucket_re_emit_nests_jobs_under_step_wrappers() {
    let src = "pipelines:\n  default:\n    - step:\n        name: build\n        image: rust:1.75\n        script:\n          - cargo build\n    - step:\n        name: test\n        script:\n          - cargo test\n";
    let g = pipeline_forward::forward("bitbucket", src).unwrap();
    let out = pipeline_forward::re_emit("bitbucket", &g).unwrap();
    assert!(
        out.contains("pipelines:") && out.contains("default:") && out.contains("step:"),
        "re-emit reconstructs the pipelines.default `- step:` nesting:\n{out}"
    );
    let mut names = jobs(&out);
    names.sort_unstable();
    assert_eq!(
        names,
        vec!["build", "test"],
        "emitted YAML re-forwards to the same jobs:\n{out}"
    );
}

#[test]
fn bitbucket_parallel_inside_branch_lifts_and_re_emits() {
    // F2 stage 2c: `- parallel:` inside a selector list — jobs lift with the
    // branch condition AND the emit re-nests both group levels.
    let src = "pipelines:\n  branches:\n    main:\n      - parallel:\n          - step:\n              name: unit\n              script:\n                - cargo test\n          - step:\n              name: doc\n              script:\n                - cargo doc\n";
    let g = pipeline_forward::forward("bitbucket", src).unwrap();
    let p = pipeline_render::lift(&g).unwrap();
    let mut names: Vec<&str> = p.jobs.iter().map(|j| j.name.as_str()).collect();
    names.sort_unstable();
    assert_eq!(
        names,
        vec!["doc", "unit"],
        "parallel members inside branch lift"
    );
    let out = pipeline_forward::re_emit("bitbucket", &g).unwrap();
    assert!(
        out.contains("branches:") && out.contains("main:") && out.contains("parallel:"),
        "re-emit reconstructs branch + parallel nesting:\n{out}"
    );
    let mut names2 = jobs(&out);
    names2.sort();
    assert_eq!(
        names2,
        vec!["doc", "unit"],
        "emitted YAML re-forwards to the same jobs:\n{out}"
    );
}

#[test]
fn bitbucket_anchor_steps_resolve_to_full_jobs() {
    // F2 stage 2d: `- step: *alias` use sites resolve to the anchored
    // template body at seed time — each use site is a full job (steps from
    // the template), and the fixpoint holds with the denormalised emit.
    let src = "definitions:\n  steps:\n    - step: &lint\n        name: lint\n        script:\n          - cargo clippy\npipelines:\n  default:\n    - step: *lint\n  branches:\n    main:\n      - step: *lint\n      - step:\n          name: deploy\n          script:\n            - make deploy\n";
    let g = pipeline_forward::forward("bitbucket", src).unwrap();
    let p = pipeline_render::lift(&g).unwrap();
    let mut named: Vec<(String, Option<String>)> = p
        .jobs
        .iter()
        .map(|j| (j.name.clone(), j.condition.clone()))
        .collect();
    named.sort();
    assert_eq!(
        named,
        vec![
            ("deploy".into(), Some("branch: main".into())),
            ("lint".into(), None),
            ("lint".into(), Some("branch: main".into())),
        ],
        "both *lint use sites become full jobs in their groups"
    );
    let lint = p.jobs.iter().find(|j| j.name == "lint").unwrap();
    assert_eq!(
        lint.steps
            .iter()
            .map(|s| s.label.as_str())
            .collect::<Vec<_>>(),
        vec!["cargo clippy"],
        "template body resolved"
    );
    // Denormalised emit: inline bodies, no dangling aliases — and it
    // re-forwards to the same three jobs.
    let out = pipeline_forward::re_emit("bitbucket", &g).unwrap();
    assert!(!out.contains('*'), "no dangling aliases in emit:\n{out}");
    let mut emitted_jobs = jobs(&out);
    emitted_jobs.sort();
    assert_eq!(
        emitted_jobs,
        vec!["deploy", "lint", "lint"],
        "emitted YAML re-forwards to the same jobs:\n{out}"
    );
}

#[test]
fn bitbucket_parallel_expanded_lifts_and_re_emits() {
    // F2 stage 2e: the EXPANDED parallel object form — jobs lift, fail-fast
    // is captured, and the emit reconstructs the {steps, fail-fast} shape.
    let src = "pipelines:\n  default:\n    - parallel:\n        fail-fast: true\n        steps:\n          - step:\n              name: unit\n              script:\n                - cargo test\n          - step:\n              name: doc\n              script:\n                - cargo doc\n  branches:\n    main:\n      - parallel:\n          steps:\n            - step:\n                name: deploy\n                script:\n                  - make deploy\n";
    let g = pipeline_forward::forward("bitbucket", src).unwrap();
    let p = pipeline_render::lift(&g).unwrap();
    let mut named: Vec<(String, Option<String>)> = p
        .jobs
        .iter()
        .map(|j| (j.name.clone(), j.condition.clone()))
        .collect();
    named.sort();
    assert_eq!(
        named,
        vec![
            ("deploy".into(), Some("branch: main".into())),
            ("doc".into(), None),
            ("unit".into(), None),
        ],
        "expanded-parallel members lift (default + branch)"
    );
    let out = pipeline_forward::re_emit("bitbucket", &g).unwrap();
    assert!(
        out.contains("parallel:") && out.contains("steps:") && out.contains("fail-fast: true"),
        "re-emit reconstructs the expanded shape incl. fail-fast:\n{out}"
    );
    let mut emitted_jobs = jobs(&out);
    emitted_jobs.sort();
    assert_eq!(
        emitted_jobs,
        vec!["deploy", "doc", "unit"],
        "emitted YAML re-forwards to the same jobs:\n{out}"
    );
}
