//! Real functional end-to-end tests for `pipewright run`: they spin up actual
//! Docker containers against a real temp repo and assert on observable
//! behaviour (the repo is mounted, env reaches the container, conditions skip,
//! translate-only platforms refuse). `#[ignore]` because they need a Docker
//! daemon and are slow — run with `cargo test --test e2e_run -- --ignored`.

use std::path::{Path, PathBuf};
use std::process::Command;

/// Path to the freshly-built `pipewright` binary (Cargo sets this for tests).
fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_pipewright")
}

/// True if a Docker daemon answers — otherwise the e2e tests skip (print + pass)
/// so they never fail a machine without Docker.
fn docker_up() -> bool {
    Command::new("docker")
        .arg("info")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Make a throwaway repo dir under the system temp with `files` written into it.
fn make_repo(name: &str, files: &[(&str, &str)]) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("dc-e2e-{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    for (f, c) in files {
        std::fs::write(dir.join(f), c).unwrap();
    }
    dir
}

/// Run `pipewright run <pipeline_file> <extra…>` and return (success, stdout+stderr).
fn run(file: &Path, extra: &[&str]) -> (bool, String) {
    let out = Command::new(bin())
        .arg("run")
        .arg(file)
        .args(extra)
        .output()
        .expect("spawn pipewright");
    let mut s = String::from_utf8_lossy(&out.stdout).into_owned();
    s.push_str(&String::from_utf8_lossy(&out.stderr));
    (out.status.success(), s)
}

#[test]
#[ignore = "needs Docker; run with --ignored"]
fn run_mount_modes_ro_rw_copy() {
    if !docker_up() {
        eprintln!("SKIP: no Docker daemon");
        return;
    }
    let repo = make_repo(
        "mountmode",
        &[(
            ".gitlab-ci.yml",
            "build:\n  image: alpine:latest\n  script:\n    - echo written > out.txt\n",
        )],
    );
    let pf = repo.join(".gitlab-ci.yml");
    let marker = repo.join("out.txt");

    // Default = read-only: the write fails and the host dir is untouched.
    let _ = std::fs::remove_file(&marker);
    let (ok, log) = run(&pf, &[]);
    assert!(!ok, "read-only run should fail on a write:\n{log}");
    assert!(!marker.exists(), "read-only must not write to the host dir");

    // --rw: the write lands in the real directory.
    let _ = std::fs::remove_file(&marker);
    let (ok, log) = run(&pf, &["--rw"]);
    assert!(ok, "--rw run should succeed:\n{log}");
    assert!(marker.exists(), "--rw must write through to the host dir");

    // --rw-copy: the job succeeds but the real directory stays clean.
    let _ = std::fs::remove_file(&marker);
    let (ok, log) = run(&pf, &["--rw-copy"]);
    assert!(ok, "--rw-copy run should succeed:\n{log}");
    assert!(!marker.exists(), "--rw-copy must not touch the real dir");
}

#[test]
#[ignore = "needs Docker; run with --ignored"]
fn run_mounts_repo_and_passes_env() {
    if !docker_up() {
        eprintln!("SKIP: no Docker daemon");
        return;
    }
    // The pipeline reads a file that only exists in the repo (proves the mount)
    // and echoes an env var (proves env passthrough).
    let repo = make_repo(
        "mount",
        &[
            ("marker.txt", "i-am-mounted\n"),
            (
                ".gitlab-ci.yml",
                "build:\n  image: alpine:latest\n  variables:\n    GREETING: hello-e2e\n  script:\n    - cat marker.txt\n    - echo \"env=$GREETING\"\n",
            ),
        ],
    );
    let (ok, log) = run(&repo.join(".gitlab-ci.yml"), &[]);
    assert!(ok, "run should succeed:\n{log}");
    assert!(
        log.contains("i-am-mounted"),
        "repo file must be readable in the container:\n{log}"
    );
    assert!(
        log.contains("env=hello-e2e"),
        "env var must reach the container:\n{log}"
    );
    assert!(log.contains("job build ok"), "job reports ok:\n{log}");
}

#[test]
#[ignore = "needs Docker; run with --ignored"]
fn run_evaluates_conditions_by_trigger() {
    if !docker_up() {
        eprintln!("SKIP: no Docker daemon");
        return;
    }
    let repo = make_repo(
        "cond",
        &[(
            ".gitlab-ci.yml",
            "build:\n  image: alpine:latest\n  script: [echo built]\ndeploy:\n  image: alpine:latest\n  rules:\n    - if: '$CI_COMMIT_BRANCH == \"main\"'\n  script: [echo deployed]\n",
        )],
    );
    let pf = repo.join(".gitlab-ci.yml");
    // On main: deploy runs.
    let (ok, log) = run(&pf, &["--trigger", "push", "--ref", "main"]);
    assert!(ok, "main run ok:\n{log}");
    assert!(log.contains("deployed"), "deploy runs on main:\n{log}");
    // On dev: deploy is skipped (its rules:if is false), build still runs.
    let (ok, log) = run(&pf, &["--trigger", "push", "--ref", "dev"]);
    assert!(ok, "dev run ok:\n{log}");
    assert!(log.contains("built"), "build runs on dev:\n{log}");
    assert!(
        log.contains("deploy") && log.contains("SKIPPED"),
        "deploy skipped on dev:\n{log}"
    );
    assert!(
        !log.contains("deployed"),
        "deploy command must NOT execute on dev:\n{log}"
    );
}

#[test]
#[ignore = "needs Docker; run with --ignored"]
fn run_starts_service_sidecar() {
    if !docker_up() {
        eprintln!("SKIP: no Docker daemon");
        return;
    }
    let repo = make_repo(
        "svc",
        &[(
            ".gitlab-ci.yml",
            "test:\n  image: redis:7\n  services:\n    - redis:7\n  script:\n    - redis-cli -h redis ping\n",
        )],
    );
    let (ok, log) = run(&repo.join(".gitlab-ci.yml"), &[]);
    assert!(ok, "service run ok:\n{log}");
    assert!(
        log.contains("PONG"),
        "job reaches the redis sidecar by hostname:\n{log}"
    );
}

#[test]
fn run_refuses_translate_only_platform() {
    // No Docker needed: the refusal happens before any container work.
    let repo = make_repo(
        "argo",
        &[(
            "wf.yaml",
            "apiVersion: argoproj.io/v1alpha1\nkind: Workflow\nspec:\n  templates:\n    - name: build\n      container:\n        image: alpine\n        command: [echo, hi]\n",
        )],
    );
    let (ok, log) = run(&repo.join("wf.yaml"), &["-p", "argo"]);
    assert!(!ok, "argo run must fail fast:\n{log}");
    assert!(log.contains("can't run locally"), "explains why:\n{log}");
    assert!(
        log.to_lowercase().contains("kubernetes") || log.contains("inspect"),
        "points elsewhere:\n{log}"
    );
}

#[test]
#[ignore = "needs Docker; run with --ignored"]
fn run_buildkite_after_lift_fix() {
    if !docker_up() {
        eprintln!("SKIP: no Docker daemon");
        return;
    }
    // buildkite was previously un-runnable (label/command not lifted). It must
    // now run its command in the mounted repo.
    let repo = make_repo(
        "bk",
        &[
            ("flag.txt", "buildkite-sees-me\n"),
            (
                "pipeline.yml",
                "steps:\n  - label: build\n    command: cat flag.txt\n",
            ),
        ],
    );
    let (ok, log) = run(&repo.join("pipeline.yml"), &["-p", "buildkite"]);
    assert!(ok, "buildkite run ok:\n{log}");
    assert!(
        log.contains("buildkite-sees-me"),
        "buildkite command runs against the repo:\n{log}"
    );
}
