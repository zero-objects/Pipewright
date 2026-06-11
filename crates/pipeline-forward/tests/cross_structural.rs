//! F1: cross-structural migration (job-based ↔ step-flat families) via the
//! model-synthesis bridge in `migrate`.

fn jobs(plat: &str, src: &str) -> usize {
    pipeline_forward::forward(plat, src)
        .ok()
        .and_then(|g| pipeline_render::lift(&g))
        .map_or(0, |p| p.jobs.len())
}

fn job_names(plat: &str, src: &str) -> Vec<String> {
    let mut v: Vec<String> = pipeline_forward::forward(plat, src)
        .ok()
        .and_then(|g| pipeline_render::lift(&g))
        .map(|p| p.jobs.iter().map(|j| j.name.clone()).collect())
        .unwrap_or_default();
    v.sort();
    v
}

#[test]
fn gitlab_to_drone_flattens_jobs_to_steps() {
    let src = "build:\n  image: rust:1.75\n  script:\n    - cargo build\ndeploy:\n  needs: [build]\n  image: alpine\n  script:\n    - make deploy\n";
    let out = pipeline_forward::migrate("gitlab", src, "drone").unwrap();
    assert!(
        out.contains("kind: pipeline") && out.contains("steps:"),
        "valid drone:\n{out}"
    );
    assert!(out.contains("name: build") && out.contains("name: deploy"));
    assert!(
        out.contains("depends_on: [build]"),
        "needs → depends_on:\n{out}"
    );
    // Re-forwards to a real drone pipeline (2 steps).
    assert_eq!(jobs("drone", &out), 2, "drone output is valid:\n{out}");
}

#[test]
fn drone_to_gitlab_wraps_steps_to_jobs() {
    let src = "kind: pipeline\nname: ci\nsteps:\n  - name: build\n    image: rust:1.75\n    commands:\n      - cargo build\n      - cargo test\n";
    let out = pipeline_forward::migrate("drone", src, "gitlab").unwrap();
    assert!(
        out.contains("build:") && out.contains("script:"),
        "valid gitlab:\n{out}"
    );
    assert!(out.contains("cargo build") && out.contains("cargo test"));
    assert_eq!(jobs("gitlab", &out), 1, "gitlab output is valid:\n{out}");
}

#[test]
fn gitlab_to_buildkite_flattens_jobs_to_steps() {
    let src = "build:\n  image: rust:1.75\n  script:\n    - cargo build\ndeploy:\n  needs: [build]\n  script:\n    - make deploy\n";
    let out = pipeline_forward::migrate("gitlab", src, "buildkite").unwrap();
    assert!(
        out.contains("steps:") && out.contains("label: build"),
        "valid buildkite:\n{out}"
    );
    assert!(
        out.contains("depends_on: [build]"),
        "needs → depends_on:\n{out}"
    );
    assert!(out.contains("cargo build") && out.contains("make deploy"));
    assert_eq!(
        jobs("buildkite", &out),
        2,
        "buildkite output is valid:\n{out}"
    );
}

#[test]
fn gitlab_to_google_cloudbuild_flattens_jobs_to_steps() {
    let src = "build:\n  image: rust:1.75\n  script:\n    - cargo build\ndeploy:\n  needs: [build]\n  image: alpine\n  script:\n    - make deploy\n";
    let out = pipeline_forward::migrate("gitlab", src, "google_cloudbuild").unwrap();
    assert!(
        out.contains("id: build") && out.contains("name: 'rust:1.75'"),
        "valid gcb:\n{out}"
    );
    assert!(out.contains("waitFor:"), "needs → waitFor:\n{out}");
    assert!(out.contains("cargo build") && out.contains("make deploy"));
    assert_eq!(
        jobs("google_cloudbuild", &out),
        2,
        "gcb output is valid:\n{out}"
    );
}

#[test]
fn drone_to_circleci_wraps_steps_to_jobs() {
    let src = "kind: pipeline\nname: ci\nsteps:\n  - name: build\n    image: rust:1.75\n    commands:\n      - cargo build\n  - name: deploy\n    image: alpine\n    depends_on: [build]\n    commands:\n      - make deploy\n";
    let out = pipeline_forward::migrate("drone", src, "circleci").unwrap();
    assert!(
        out.contains("version: 2.1") && out.contains("jobs:"),
        "valid circleci:\n{out}"
    );
    assert!(
        out.contains("- image: 'rust:1.75'"),
        "image → docker:\n{out}"
    );
    assert!(
        out.contains("workflows:") && out.contains("requires:"),
        "needs → requires:\n{out}"
    );
    assert!(out.contains("run: 'cargo build'") && out.contains("run: 'make deploy'"));
    assert_eq!(
        jobs("circleci", &out),
        2,
        "circleci output is valid:\n{out}"
    );
}

#[test]
fn drone_to_azure_wraps_steps_to_jobs() {
    let src = "kind: pipeline\nname: ci\nsteps:\n  - name: build\n    image: rust:1.75\n    commands:\n      - cargo build\n  - name: deploy\n    depends_on: [build]\n    commands:\n      - make deploy\n";
    let out = pipeline_forward::migrate("drone", src, "azure").unwrap();
    assert!(
        out.contains("- job: build") && out.contains("- job: deploy"),
        "valid azure:\n{out}"
    );
    assert!(
        out.contains("container: 'rust:1.75'"),
        "image → container:\n{out}"
    );
    assert!(
        out.contains("dependsOn: [build]"),
        "needs → dependsOn:\n{out}"
    );
    assert!(out.contains("script: 'cargo build'") && out.contains("script: 'make deploy'"));
    // F3 fixed (azure `- job:` discriminator → scalar_attr): names survive.
    assert_eq!(
        job_names("azure", &out),
        vec!["build", "deploy"],
        "azure output re-forwards with names:\n{out}"
    );
}

#[test]
fn same_family_migration_uses_the_normal_path() {
    // gitlab → github (both job-based) must NOT use the bridge — the normal
    // re-key path handles it (and the bridge would be a no-op anyway).
    let src = "build:\n  image: rust:1.75\n  script:\n    - cargo build\n";
    let out = pipeline_forward::migrate("gitlab", src, "github").unwrap();
    assert!(
        out.contains("jobs:") && out.contains("build:"),
        "github via normal path:\n{out}"
    );
}
