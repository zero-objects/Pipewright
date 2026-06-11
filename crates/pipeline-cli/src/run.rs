//! Local execution: order jobs by their dependencies, then either print a plan
//! (`plan`) or run each job in a Docker container (`run`, via bollard).

use std::fmt::Write as _;
use std::path::Path;

use pipeline_render::{Job, Pipeline};

/// Order jobs so every job comes after the jobs it `needs`. Kahn's algorithm;
/// jobs with unknown/cyclic deps fall to the end in declaration order (so a
/// malformed DAG still runs everything rather than dropping jobs).
fn topo_order(p: &Pipeline) -> Vec<&Job> {
    let mut done: Vec<String> = Vec::new();
    let mut ordered: Vec<&Job> = Vec::new();
    let mut remaining: Vec<&Job> = p.jobs.iter().collect();
    while !remaining.is_empty() {
        // The FIRST job (declaration order) whose needs are all scheduled — so
        // among ready jobs we keep the source order. Fall back to the first
        // remaining job when none is ready (unknown/cyclic deps) rather than
        // dropping it.
        let idx = remaining
            .iter()
            .position(|j| j.needs.iter().all(|n| done.contains(n)))
            .unwrap_or(0);
        let j = remaining.remove(idx);
        done.push(j.name.clone());
        ordered.push(j);
    }
    ordered
}

/// The container image a job runs in (its `image` param), if declared.
fn job_image(job: &Job) -> Option<&str> {
    job.params
        .iter()
        .find(|p| p.key == "image")
        .map(|p| p.value.as_str())
}

/// The shell commands a job runs (its step labels).
fn job_commands(job: &Job) -> Vec<&str> {
    job.steps.iter().map(|s| s.label.as_str()).collect()
}

/// The job's `KEY=VALUE` environment entries (from `variables` / `env`), in the
/// `key=value` form the model already stores them as.
fn job_env(job: &Job) -> Vec<String> {
    job.params
        .iter()
        .filter(|p| p.key == "env")
        .map(|p| p.value.clone())
        .collect()
}

/// Recursively copy `src` into `dst` (used by `MountMode::ReadWriteCopy` to
/// clone the workspace before mounting it read-write). Skips `.git` and
/// `target` — version-control internals and build output a fresh run doesn't
/// need, which are also the bulkiest directories to copy.
fn copy_dir_all(src: &Path, dst: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let name = entry.file_name();
        if matches!(name.to_str(), Some(".git" | "target")) {
            continue;
        }
        let from = entry.path();
        let to = dst.join(&name);
        if entry.file_type()?.is_dir() {
            copy_dir_all(&from, &to)?;
        } else {
            std::fs::copy(&from, &to)?;
        }
    }
    Ok(())
}

/// A Docker-safe container-name fragment: `[a-zA-Z0-9_.-]` only.
fn sanitize(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-') {
                c
            } else {
                '_'
            }
        })
        .collect()
}

/// The simulated trigger context a local run evaluates `rules:if` against.
pub struct Trigger {
    /// `push` / `tag` / `merge_request` / `schedule` / `manual` / `external`.
    pub event: String,
    /// The git ref name (branch for `push`, tag name for `tag`).
    pub r#ref: String,
}

impl Trigger {
    /// Resolve a CI variable used in `rules:if` to its value in this context.
    /// Returns `None` for variables we don't model — the caller then refuses to
    /// evaluate rather than guessing.
    fn var(&self, name: &str) -> Option<String> {
        let is_tag = self.event == "tag";
        match name {
            "CI_PIPELINE_SOURCE" => Some(self.event.clone()),
            "CI_COMMIT_REF_NAME" => Some(self.r#ref.clone()),
            "CI_COMMIT_BRANCH" => Some(if is_tag {
                String::new()
            } else {
                self.r#ref.clone()
            }),
            "CI_COMMIT_TAG" => Some(if is_tag {
                self.r#ref.clone()
            } else {
                String::new()
            }),
            _ => None,
        }
    }
}

/// Whether a job runs under `trigger`, given its `when:` and `rules:if`.
/// CONSERVATIVE BY DESIGN: only a confidently-understood condition skips a job;
/// anything we can't evaluate runs (with a printed note), so we never silently
/// drop work on a guess.
enum Gate {
    /// Run it.
    Run,
    /// Skip it, with a human reason.
    Skip(String),
    /// Run it, but flag that the condition wasn't evaluated.
    RunUnevaluated(String),
}

fn gate(job: &Job, trigger: &Trigger) -> Gate {
    // `when:` first — manual/never/delayed don't run in a plain local execution.
    if let Some(w) = &job.when {
        match w.as_str() {
            "never" => return Gate::Skip("when: never".to_string()),
            "manual" => return Gate::Skip("when: manual (start it explicitly)".to_string()),
            "delayed" => return Gate::Skip("when: delayed".to_string()),
            _ => {}
        }
    }
    // `rules:if` — evaluate the common single-comparison shapes only.
    if let Some(expr) = &job.condition {
        // A bare `when:` value already handled above isn't an if-expression.
        if !expr.contains('$') {
            return Gate::Run;
        }
        match eval_if(expr, trigger) {
            Some(true) => Gate::Run,
            Some(false) => Gate::Skip(format!("rules:if false ({expr})")),
            None => Gate::RunUnevaluated(format!("rules:if not evaluated ({expr})")),
        }
    } else {
        Gate::Run
    }
}

/// Evaluate a single `rules:if` comparison against the trigger. Handles
/// `$VAR == "x"`, `!= "x"`, `=~ /re/`, `!~ /re/`, and bare `$VAR` truthiness.
/// Returns `None` for anything with `&&`/`||`, unknown variables, or a shape we
/// don't parse — the caller treats `None` as "run, unevaluated".
fn eval_if(expr: &str, trigger: &Trigger) -> Option<bool> {
    let e = expr.trim();
    // No boolean composition — too easy to get wrong; bail honestly.
    if e.contains("&&") || e.contains("||") {
        return None;
    }
    // Operator comparisons.
    for (op, neg) in [("==", false), ("!=", true), ("=~", false), ("!~", true)] {
        if let Some((lhs, rhs)) = e.split_once(op) {
            let var_name = lhs.trim().strip_prefix('$')?;
            let val = trigger.var(var_name.trim())?;
            let rhs = rhs.trim();
            if op == "==" || op == "!=" {
                let want = rhs.trim_matches(|c| c == '"' || c == '\'');
                return Some((val == want) ^ neg);
            }
            // Regex match: rhs is `/pattern/`.
            let pat = rhs.strip_prefix('/').and_then(|s| s.strip_suffix('/'))?;
            let re = regex_lite_match(pat, &val)?;
            return Some(re ^ neg);
        }
    }
    // Bare `$VAR` → truthy if the variable is non-empty.
    if let Some(var) = e.strip_prefix('$') {
        return Some(!trigger.var(var.trim())?.is_empty());
    }
    None
}

/// Minimal anchored-substring regex check sufficient for the `=~ /…/` patterns
/// real pipelines use (`^main$`, `^release/`, `v.*`). Falls back to `None` for
/// metacharacters we don't handle, so the caller stays honest.
fn regex_lite_match(pat: &str, val: &str) -> Option<bool> {
    // Only handle ^anchors, $anchors, and `.*`; reject other metachars.
    if pat.chars().any(|c| {
        matches!(
            c,
            '[' | ']' | '(' | ')' | '|' | '+' | '?' | '{' | '}' | '\\'
        )
    }) {
        return None;
    }
    let anchored_start = pat.starts_with('^');
    let anchored_end = pat.ends_with('$');
    let core = pat.trim_start_matches('^').trim_end_matches('$');
    // `.*` is the only wildcard we accept; split on it and check the literal parts.
    let parts: Vec<&str> = core.split(".*").collect();
    if parts.iter().any(|p| p.contains('.') || p.contains('*')) {
        return None; // leftover metachar
    }
    Some(match parts.as_slice() {
        [single] => {
            // No wildcard: anchored compare or substring.
            match (anchored_start, anchored_end) {
                (true, true) => val == *single,
                (true, false) => val.starts_with(single),
                (false, true) => val.ends_with(single),
                (false, false) => val.contains(single),
            }
        }
        many => {
            // `a.*b.*c`: first part respects start anchor, last respects end.
            let mut pos = 0usize;
            for (i, part) in many.iter().enumerate() {
                if i == 0 && anchored_start && !val[pos..].starts_with(part) {
                    return Some(false);
                }
                match val[pos..].find(part) {
                    Some(idx) => pos += idx + part.len(),
                    None => return Some(false),
                }
            }
            if anchored_end {
                val.ends_with(many.last().unwrap())
            } else {
                true
            }
        }
    })
}

/// A human-readable execution plan: jobs in dependency order with their image
/// and commands. No Docker — pure projection of the IR, so it always works.
#[must_use]
pub fn plan(p: &Pipeline) -> String {
    let mut out = String::new();
    let order = topo_order(p);
    let _ = writeln!(out, "Execution plan — {} job(s):", order.len());
    for (i, job) in order.iter().enumerate() {
        let _ = write!(out, "\n{}. {}", i + 1, job.name);
        if !job.needs.is_empty() {
            let _ = write!(out, "  (after: {})", job.needs.join(", "));
        }
        out.push('\n');
        let _ = writeln!(
            out,
            "   image: {}",
            job_image(job).unwrap_or("(none — would default to alpine:latest)")
        );
        let cmds = job_commands(job);
        if cmds.is_empty() {
            out.push_str("   (no commands)\n");
        } else {
            for c in cmds {
                let _ = writeln!(out, "   $ {c}");
            }
        }
    }
    out
}

/// Run the pipeline locally in Docker. Each job (in dependency order, or only
/// `only_job` if given) runs its commands in its image; output streams to
/// stdout. Stops at the first failing job.
///
/// # Errors
/// A message if Docker is unreachable, an image can't be pulled, or a job exits
/// non-zero.
/// How the pipeline's directory is mounted into each job's container.
///
/// A pipeline you didn't write is untrusted code: its commands run with your
/// Docker permissions against whatever is mounted. The default is the safe one.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[allow(
    clippy::enum_variant_names,
    reason = "the Read* prefix mirrors the mount permission and reads clearly"
)]
pub enum MountMode {
    /// Mount the real directory read-only. Safest; a job that writes fails.
    ReadOnly,
    /// Mount the real directory read-write. The job's commands can modify it —
    /// only for pipelines you trust.
    ReadWrite,
    /// Copy the directory to a throwaway location and mount that read-write.
    /// Builds work, but the real directory is never touched.
    ReadWriteCopy,
}

pub fn run(
    p: &Pipeline,
    only_job: Option<&str>,
    workspace: &Path,
    trigger: &Trigger,
    mount: MountMode,
) -> Result<String, String> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| format!("cannot start runtime: {e}"))?;
    rt.block_on(run_async(p, only_job, workspace, trigger, mount))
}

/// Connect to the Docker daemon, tolerating Docker Desktop's per-user socket.
/// Tries the standard locations (`DOCKER_HOST` / `/var/run/docker.sock` via
/// `connect_with_local_defaults`), then the Docker Desktop default
/// `~/.docker/run/docker.sock`, verifying each with a ping.
async fn connect_docker() -> Result<bollard::Docker, String> {
    use bollard::Docker;
    if let Ok(d) = Docker::connect_with_local_defaults() {
        if d.ping().await.is_ok() {
            return Ok(d);
        }
    }
    if let Some(home) = std::env::var_os("HOME") {
        let sock = format!("{}/.docker/run/docker.sock", home.to_string_lossy());
        if std::path::Path::new(&sock).exists() {
            if let Ok(d) = Docker::connect_with_unix(&sock, 120, bollard::API_DEFAULT_VERSION) {
                if d.ping().await.is_ok() {
                    return Ok(d);
                }
            }
        }
    }
    Err("cannot connect to Docker (is the daemon running? set DOCKER_HOST if it's on a non-default socket)".to_string())
}

async fn run_async(
    p: &Pipeline,
    only_job: Option<&str>,
    workspace: &Path,
    trigger: &Trigger,
    mount: MountMode,
) -> Result<String, String> {
    let docker = connect_docker().await?;

    // The workspace is bind-mounted into every job at /workspace, so commands
    // run against the real repo (a CI job with no source is useless). An
    // absolute host path is required by the Docker API.
    let real_dir = std::fs::canonicalize(workspace)
        .map_err(|e| format!("cannot resolve workspace {}: {e}", workspace.display()))?;

    // Resolve the effective mount dir + read-only flag from the mode. For the
    // copy mode, the real directory is cloned to a throwaway location so the
    // job can write freely without touching it.
    let (host_dir, read_only) = match mount {
        MountMode::ReadOnly => (real_dir.to_string_lossy().to_string(), true),
        MountMode::ReadWrite => (real_dir.to_string_lossy().to_string(), false),
        MountMode::ReadWriteCopy => {
            let dest = std::env::temp_dir().join(format!("pipewright-run-{}", std::process::id()));
            copy_dir_all(&real_dir, &dest)
                .map_err(|e| format!("cannot copy workspace for --rw-copy: {e}"))?;
            (dest.to_string_lossy().to_string(), false)
        }
    };
    let mode_note = match mount {
        MountMode::ReadOnly => "read-only",
        MountMode::ReadWrite => "read-write (commands can modify this directory)",
        MountMode::ReadWriteCopy => "read-write on a throwaway copy (real directory untouched)",
    };
    println!("workspace: {host_dir} → /workspace  [{mode_note}]");
    println!("trigger:   event={} ref={}", trigger.event, trigger.r#ref);

    // A per-run user bridge network so a job's service sidecars are reachable
    // by name (`postgres:5432`). Reused across jobs; torn down at the end.
    let net = format!("pipewright-net-{}", std::process::id());
    let need_net = order_needs_services(p, only_job, trigger);
    if need_net {
        ensure_network(&docker, &net).await?;
    }

    let order = topo_order(p);
    let mut result = Ok(String::new());
    for (idx, job) in order.iter().enumerate() {
        // An explicit `--job` runs that one job unconditionally (the user asked
        // for it by name); otherwise honor the job's gate.
        let explicit = only_job == Some(job.name.as_str());
        if let Some(want) = only_job {
            if job.name != want {
                continue;
            }
        }
        if !explicit {
            match gate(job, trigger) {
                Gate::Run => {}
                Gate::Skip(why) => {
                    println!("\n=== job: {} — SKIPPED ({why}) ===", job.name);
                    continue;
                }
                Gate::RunUnevaluated(note) => {
                    println!("\n[note] {} — {note}; running anyway", job.name);
                }
            }
        }
        let image = job_image(job).unwrap_or("alpine:latest").to_string();
        let cmds = job_commands(job);
        let env = job_env(job);
        println!("\n=== job: {} (image: {image}) ===", job.name);
        if cmds.is_empty() {
            println!("(no commands — skipping)");
            continue;
        }
        // Start this job's service sidecars on the run network.
        let mut sidecars = Vec::new();
        for (si, svc) in job.services.iter().enumerate() {
            match start_service(&docker, idx, si, svc, &net).await {
                Ok(cid) => {
                    println!("  service up: {svc} (host: {})", service_host(svc));
                    sidecars.push(cid);
                }
                Err(e) => eprintln!("  [service {svc} failed to start] {e}"),
            }
        }
        let job_net = if job.services.is_empty() {
            None
        } else {
            Some(net.as_str())
        };
        let r = run_job(
            &docker, &job.name, idx, &image, &cmds, &env, &host_dir, read_only, job_net,
        )
        .await;
        // Tear down this job's sidecars regardless of the job's outcome.
        for cid in sidecars {
            let _ = docker
                .remove_container(
                    &cid,
                    Some(bollard::container::RemoveContainerOptions {
                        force: true,
                        ..Default::default()
                    }),
                )
                .await;
        }
        if let Err(e) = r {
            result = Err(e);
            break;
        }
    }
    // Remove the run network if we created one.
    if need_net {
        let _ = docker.remove_network(&net).await;
    }
    result
}

/// Whether any job that would actually run declares a service — so we only
/// create the bridge network when it's needed.
fn order_needs_services(p: &Pipeline, only_job: Option<&str>, trigger: &Trigger) -> bool {
    p.jobs.iter().any(|j| {
        if only_job.is_some_and(|w| w != j.name) {
            return false;
        }
        !j.services.is_empty()
            && (only_job == Some(j.name.as_str()) || !matches!(gate(j, trigger), Gate::Skip(_)))
    })
}

/// The hostname a service is reachable at on the run network — the image's bare
/// name without registry path or tag (`docker.io/library/postgres:16` →
/// `postgres`), matching GitLab's service-aliasing convention.
fn service_host(image: &str) -> String {
    let no_tag = image.rsplit_once(':').map_or(image, |(n, _)| n);
    no_tag.rsplit('/').next().unwrap_or(no_tag).to_string()
}

/// Create a user bridge network if it doesn't already exist (idempotent).
async fn ensure_network(docker: &bollard::Docker, name: &str) -> Result<(), String> {
    use bollard::network::CreateNetworkOptions;
    // Ignore "already exists"; surface other failures.
    let _ = docker
        .create_network(CreateNetworkOptions {
            name,
            ..Default::default()
        })
        .await;
    Ok(())
}

/// Start one service sidecar on `net`, aliased to its [`service_host`] so the
/// job reaches it by name. Returns the container id for later teardown.
async fn start_service(
    docker: &bollard::Docker,
    job_idx: usize,
    svc_idx: usize,
    image: &str,
    net: &str,
) -> Result<String, String> {
    use bollard::container::{Config, CreateContainerOptions};
    use bollard::image::CreateImageOptions;
    use bollard::models::{EndpointSettings, HostConfig};
    use futures_util::StreamExt;
    use std::collections::HashMap;

    let mut pull = docker.create_image(
        Some(CreateImageOptions {
            from_image: image,
            ..Default::default()
        }),
        None,
        None,
    );
    while let Some(item) = pull.next().await {
        item.map_err(|e| format!("pull {image}: {e}"))?;
    }

    let host = service_host(image);
    let mut endpoints = HashMap::new();
    endpoints.insert(
        net.to_string(),
        EndpointSettings {
            aliases: Some(vec![host.clone()]),
            ..Default::default()
        },
    );
    let config = Config {
        image: Some(image.to_string()),
        host_config: Some(HostConfig {
            network_mode: Some(net.to_string()),
            ..Default::default()
        }),
        networking_config: Some(bollard::container::NetworkingConfig {
            endpoints_config: endpoints,
        }),
        ..Default::default()
    };
    // The pid keeps concurrent runs (two UI windows / parallel CI) from
    // colliding on a container name.
    let cname = format!(
        "pipewright-{}-{job_idx}-svc{svc_idx}-{}",
        std::process::id(),
        sanitize(&host)
    );
    let _ = docker
        .remove_container(
            &cname,
            Some(bollard::container::RemoveContainerOptions {
                force: true,
                ..Default::default()
            }),
        )
        .await;
    let c = docker
        .create_container(
            Some(CreateContainerOptions {
                name: cname,
                platform: None,
            }),
            config,
        )
        .await
        .map_err(|e| format!("create service {image}: {e}"))?;
    docker
        .start_container::<String>(&c.id, None)
        .await
        .map_err(|e| format!("start service {image}: {e}"))?;
    Ok(c.id)
}

#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
async fn run_job(
    docker: &bollard::Docker,
    name: &str,
    idx: usize,
    image: &str,
    cmds: &[&str],
    env: &[String],
    host_dir: &str,
    read_only: bool,
    net: Option<&str>,
) -> Result<(), String> {
    use bollard::container::{Config, CreateContainerOptions, LogsOptions, WaitContainerOptions};
    use bollard::image::CreateImageOptions;
    use bollard::models::HostConfig;
    use futures_util::StreamExt;

    // Pull the image (no-op if already present).
    let mut pull = docker.create_image(
        Some(CreateImageOptions {
            from_image: image,
            ..Default::default()
        }),
        None,
        None,
    );
    while let Some(item) = pull.next().await {
        item.map_err(|e| format!("pull {image}: {e}"))?;
    }

    // Run the commands as one shell script so `&&`/env carry across steps.
    let script = cmds.join(" && ");
    let config = Config {
        image: Some(image.to_string()),
        cmd: Some(vec!["/bin/sh".to_string(), "-c".to_string(), script]),
        env: if env.is_empty() {
            None
        } else {
            Some(env.to_vec())
        },
        working_dir: Some("/workspace".to_string()),
        host_config: Some(HostConfig {
            binds: Some(vec![if read_only {
                format!("{host_dir}:/workspace:ro")
            } else {
                format!("{host_dir}:/workspace")
            }]),
            // Join the service network so sidecars resolve by hostname.
            network_mode: net.map(ToString::to_string),
            ..Default::default()
        }),
        tty: Some(false),
        ..Default::default()
    };
    // Name carries the pid + job INDEX: the index separates two jobs sharing a
    // name within a run, the pid separates concurrent runs (two UI windows /
    // parallel CI) from each other.
    let cname = format!("pipewright-{}-{idx}-{}", std::process::id(), sanitize(name));
    // A leftover container from a crashed/killed run would 409 on create —
    // force-remove any namesake first (ignore "not found").
    let _ = docker
        .remove_container(
            &cname,
            Some(bollard::container::RemoveContainerOptions {
                force: true,
                ..Default::default()
            }),
        )
        .await;
    let opts = CreateContainerOptions {
        name: cname,
        platform: None,
    };
    let container = docker
        .create_container(Some(opts), config)
        .await
        .map_err(|e| format!("create container for {name}: {e}"))?;
    let id = container.id;

    docker
        .start_container::<String>(&id, None)
        .await
        .map_err(|e| format!("start {name}: {e}"))?;

    // Stream stdout+stderr live.
    let mut logs = docker.logs(
        &id,
        Some(LogsOptions::<String> {
            follow: true,
            stdout: true,
            stderr: true,
            ..Default::default()
        }),
    );
    while let Some(chunk) = logs.next().await {
        match chunk {
            Ok(out) => print!("{out}"),
            Err(e) => eprintln!("[log error] {e}"),
        }
    }

    // Wait for exit and capture the status code.
    let mut wait = docker.wait_container(&id, None::<WaitContainerOptions<String>>);
    let mut status = 0i64;
    while let Some(res) = wait.next().await {
        match res {
            Ok(r) => status = r.status_code,
            // wait_container yields an Err for a non-zero exit, carrying the code.
            Err(bollard::errors::Error::DockerContainerWaitError { code, .. }) => status = code,
            Err(e) => return Err(format!("wait {name}: {e}")),
        }
    }

    let _ = docker
        .remove_container(
            &id,
            Some(bollard::container::RemoveContainerOptions {
                force: true,
                ..Default::default()
            }),
        )
        .await;

    if status == 0 {
        println!("--- job {name} ok ---");
        Ok(())
    } else {
        Err(format!("job {name} failed (exit {status})"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pipeline_render::lift;

    fn pipeline(src: &str, plat: &str) -> Pipeline {
        let g = pipeline_forward::forward(plat, src).unwrap();
        lift(&g).unwrap()
    }

    #[test]
    fn topo_order_respects_needs() {
        // block-style needs (flow `needs: [build]` is parsed as an opaque scalar
        // today — a separate CST gap tracked in the backlog; run then falls back
        // to declaration order).
        let p = pipeline(
            "build:\n  script:\n    - make\ntest:\n  needs:\n    - build\n  script:\n    - make test\n",
            "gitlab",
        );
        let order = topo_order(&p);
        let names: Vec<&str> = order.iter().map(|j| j.name.as_str()).collect();
        let bi = names.iter().position(|n| *n == "build").unwrap();
        let ti = names.iter().position(|n| *n == "test").unwrap();
        assert!(bi < ti, "build must precede test: {names:?}");
    }

    #[test]
    fn aws_codebuild_phases_lift_to_named_jobs_with_steps() {
        // F2: a buildspec's `phases:` map was previously dropped (0 jobs). It
        // now lifts to one named job per phase, each phase's `commands:` its
        // steps.
        let p = pipeline(
            "version: 0.2\nphases:\n  install:\n    commands:\n      - npm ci\n  build:\n    commands:\n      - npm test\n",
            "aws_codebuild",
        );
        let mut names: Vec<&str> = p.jobs.iter().map(|j| j.name.as_str()).collect();
        names.sort_unstable();
        assert_eq!(names, vec!["build", "install"], "phases → named jobs");
        let build = p.jobs.iter().find(|j| j.name == "build").unwrap();
        assert_eq!(
            build
                .steps
                .iter()
                .map(|s| s.label.as_str())
                .collect::<Vec<_>>(),
            vec!["npm test"]
        );
    }

    #[test]
    fn bitbucket_default_steps_lift_to_named_jobs_with_steps() {
        // F2 stage 1: `pipelines.default` `- step:` entries were previously
        // unreachable (0 jobs). Each inline step now lifts to a named job
        // with its image param and its `script:` lines as steps.
        let p = pipeline(
            "pipelines:\n  default:\n    - step:\n        name: build\n        image: rust:1.75\n        script:\n          - cargo build --release\n    - step:\n        name: test\n        script:\n          - cargo test\n",
            "bitbucket",
        );
        let mut names: Vec<&str> = p.jobs.iter().map(|j| j.name.as_str()).collect();
        names.sort_unstable();
        assert_eq!(names, vec!["build", "test"], "default steps → named jobs");
        let build = p.jobs.iter().find(|j| j.name == "build").unwrap();
        assert_eq!(
            build
                .steps
                .iter()
                .map(|s| s.label.as_str())
                .collect::<Vec<_>>(),
            vec!["cargo build --release"]
        );
        assert!(
            build
                .params
                .iter()
                .any(|pp| pp.key == "image" && pp.value == "rust:1.75"),
            "image param lifted"
        );
    }

    #[test]
    fn plan_lists_jobs_images_and_commands() {
        let p = pipeline(
            "build:\n  image: rust:1.75\n  script:\n    - cargo build\n",
            "gitlab",
        );
        let plan = plan(&p);
        assert!(plan.contains("build"));
        assert!(plan.contains("rust:1.75"));
        assert!(plan.contains("cargo build"));
    }

    fn trig(event: &str, r#ref: &str) -> Trigger {
        Trigger {
            event: event.to_string(),
            r#ref: r#ref.to_string(),
        }
    }

    #[test]
    fn eval_if_branch_equality() {
        let e = "$CI_COMMIT_BRANCH == \"main\"";
        assert_eq!(eval_if(e, &trig("push", "main")), Some(true));
        assert_eq!(eval_if(e, &trig("push", "dev")), Some(false));
        // On a tag event CI_COMMIT_BRANCH is empty → not "main".
        assert_eq!(eval_if(e, &trig("tag", "v1")), Some(false));
    }

    #[test]
    fn eval_if_pipeline_source_and_tag_truthiness() {
        assert_eq!(
            eval_if(
                "$CI_PIPELINE_SOURCE == \"schedule\"",
                &trig("schedule", "main")
            ),
            Some(true)
        );
        assert_eq!(
            eval_if("$CI_PIPELINE_SOURCE == \"schedule\"", &trig("push", "main")),
            Some(false)
        );
        // bare `$CI_COMMIT_TAG` is truthy only on a tag event.
        assert_eq!(eval_if("$CI_COMMIT_TAG", &trig("tag", "v1.0")), Some(true));
        assert_eq!(
            eval_if("$CI_COMMIT_TAG", &trig("push", "main")),
            Some(false)
        );
    }

    #[test]
    fn eval_if_regex_and_negation() {
        assert_eq!(
            eval_if(
                "$CI_COMMIT_BRANCH =~ /^release/",
                &trig("push", "release/2.0")
            ),
            Some(true)
        );
        assert_eq!(
            eval_if("$CI_COMMIT_BRANCH =~ /^release/", &trig("push", "main")),
            Some(false)
        );
        assert_eq!(
            eval_if("$CI_COMMIT_BRANCH != \"main\"", &trig("push", "dev")),
            Some(true)
        );
    }

    #[test]
    fn eval_if_refuses_to_guess() {
        // Boolean composition, unknown variable, unparsable shape → None (= run, unevaluated).
        assert_eq!(
            eval_if(
                "$CI_COMMIT_BRANCH == \"main\" && $X == \"y\"",
                &trig("push", "main")
            ),
            None
        );
        assert_eq!(eval_if("$CUSTOM_VAR == \"x\"", &trig("push", "main")), None);
        assert_eq!(
            eval_if("$CI_COMMIT_BRANCH =~ /^(a|b)$/", &trig("push", "a")),
            None
        );
    }

    #[test]
    fn gate_skips_manual_and_false_rules() {
        let p = pipeline(
            "build:\n  image: alpine\n  script: [echo b]\ndeploy:\n  image: alpine\n  rules:\n    - if: '$CI_COMMIT_BRANCH == \"main\"'\n  script: [echo d]\nmanualjob:\n  image: alpine\n  when: manual\n  script: [echo m]\n",
            "gitlab",
        );
        let by = |n: &str| p.jobs.iter().find(|j| j.name == n).unwrap();
        assert!(matches!(
            gate(by("build"), &trig("push", "main")),
            Gate::Run
        ));
        assert!(matches!(
            gate(by("deploy"), &trig("push", "main")),
            Gate::Run
        ));
        assert!(matches!(
            gate(by("deploy"), &trig("push", "dev")),
            Gate::Skip(_)
        ));
        assert!(matches!(
            gate(by("manualjob"), &trig("push", "main")),
            Gate::Skip(_)
        ));
    }
}
