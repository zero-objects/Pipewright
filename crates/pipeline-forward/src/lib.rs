//! The forward path as a library: a platform's surface syntax → Hub-IR
//! `TypedGraph`. Three stages, mirroring the test harness that proved them:
//!   1. **parse** the source into a CST `Document` (YAML, or the Jenkinsfile /
//!      Earthfile DSL parsers),
//!   2. **seed** a `TypedGraph` from the CST via the platform's seeder,
//!   3. run the **forward TGG cascade** to grow the `hub:` Hub-IR.
//!
//! The per-platform rulesets are embedded at build time, so a caller (CLI, FFI,
//! UI) needs no on-disk `catalog/` — [`forward`] is self-contained.

use std::collections::{HashSet, VecDeque};

use pipeline_cst::Document;
use seesaw_core::engine::{run_cascade_cached, Cascade, Rule};
use seesaw_core::graph::{GhostId, Status, TypedGraph};
use seesaw_core::rule::compile::compile_bidirectional;
use seesaw_core::rule::instantiate::instantiate;
use seesaw_core::rule::spec::RuleSetSpec;

/// Every platform `forward` understands, by its canonical key.
pub const PLATFORMS: &[&str] = &[
    "argo",
    "aws_codebuild",
    "aws_codepipeline",
    "azure",
    "bitbucket",
    "buildkite",
    "circleci",
    "dagger",
    "drone",
    "earthly",
    "github",
    "gitlab",
    "google_cloudbuild",
    "jenkins",
    "tekton",
    "travis",
    "woodpecker",
];

/// What went wrong on the way from source text to Hub-IR.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ForwardError {
    /// The platform key isn't one of [`PLATFORMS`].
    UnknownPlatform(String),
    /// The source failed to parse in the platform's surface syntax.
    Parse(String),
    /// The forward cascade failed to reach a fixpoint.
    Cascade(String),
    /// An edit could not be applied (unknown node, missing/invalid provenance).
    Edit(String),
}

impl std::fmt::Display for ForwardError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnknownPlatform(p) => write!(f, "unknown platform: {p}"),
            Self::Parse(e) => write!(f, "parse error: {e}"),
            Self::Cascade(e) => write!(f, "cascade error: {e}"),
            Self::Edit(e) => write!(f, "edit error: {e}"),
        }
    }
}

impl std::error::Error for ForwardError {}

/// `true` if `platform` is one [`forward`] can handle.
#[must_use]
pub fn is_supported(platform: &str) -> bool {
    PLATFORMS.contains(&platform)
}

/// How much of a platform's pipeline a local Docker run can faithfully execute.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunSupport {
    /// Container + shell model: jobs declare an image and shell commands, so the
    /// local runner reproduces them (gitlab/github/drone/…).
    Full,
    /// No local shell to run: the work lives elsewhere — Kubernetes CRDs
    /// (argo/tekton reference external Task definitions), cloud-service
    /// orchestration (`aws_codepipeline`), code-defined pipelines (dagger), a
    /// dedicated build engine (earthly), or a server runtime (jenkins). The tool
    /// still inspects, renders, and migrates these — it just can't *run* them.
    TranslateOnly,
}

impl RunSupport {
    /// A one-line reason a `TranslateOnly` platform can't run locally.
    #[must_use]
    pub fn reason(platform: &str) -> &'static str {
        match platform {
            "argo" | "tekton" => "Kubernetes-CRD pipelines reference external task definitions — there's no local shell script to run.",
            "aws_codepipeline" => "AWS CodePipeline orchestrates cloud service actions (CodeBuild, …), not local shell commands.",
            "dagger" => "dagger pipelines are defined in code (SDK), not as declarative jobs — nothing to run from the manifest.",
            "earthly" => "Earthfiles run through Earthly's own BuildKit engine, not a plain `docker run` shell.",
            "jenkins" => "Jenkins pipelines run on the Jenkins server runtime (Groovy), not a local container shell.",
            _ => "This platform's pipelines are translate/inspect-only locally.",
        }
    }
}

/// Whether `platform`'s pipelines can be run locally in Docker. The single
/// source of truth shared by the CLI runner, the FFI, and the UI Run tab — so
/// none of them claims to run a pipeline it can't.
#[must_use]
pub fn run_support(platform: &str) -> RunSupport {
    match platform {
        "gitlab" | "github" | "drone" | "woodpecker" | "bitbucket" | "circleci" | "azure"
        | "travis" | "aws_codebuild" | "google_cloudbuild" | "buildkite" => RunSupport::Full,
        // argo, tekton, aws_codepipeline, dagger, earthly, jenkins
        _ => RunSupport::TranslateOnly,
    }
}

/// The platform's conventional pipeline file name — what a save dialog
/// should suggest. Derived from the catalog's `[meta] config_file` entries
/// (catalog/<platform>.toml); platforms whose catalog entry is descriptive
/// rather than a file name (k8s CRDs, dagger's code-defined pipelines) get
/// the conventional concrete name instead. Directory-bound names (github's
/// `.github/workflows/*.yml`, circleci's `.circleci/config.yml`) are
/// reduced to their base name — a save dialog suggests a file, not a tree.
#[must_use]
#[allow(
    clippy::match_same_arms,
    reason = "every platform is listed explicitly as documentation, even when the file name coincides with the generic fallback"
)]
pub fn default_file_name(platform: &str) -> &'static str {
    match platform {
        "argo" => "workflow.yaml",
        "aws_codebuild" => "buildspec.yml",
        "aws_codepipeline" => "pipeline.yml",
        "azure" => "azure-pipelines.yml",
        "bitbucket" => "bitbucket-pipelines.yml",
        "buildkite" => "pipeline.yml",
        "circleci" => "config.yml",
        "dagger" => "dagger.json",
        "drone" => ".drone.yml",
        "earthly" => "Earthfile",
        "github" => "workflow.yml",
        "gitlab" => ".gitlab-ci.yml",
        "google_cloudbuild" => "cloudbuild.yaml",
        "jenkins" => "Jenkinsfile",
        "tekton" => "pipeline.yaml",
        "travis" => ".travis.yml",
        "woodpecker" => ".woodpecker.yml",
        _ => "pipeline.yml",
    }
}

/// Heuristic platform detection from distinctive surface markers. `has` is a
/// raw substring test; `key` matches a top-level YAML key (a line whose trimmed
/// start is `<name>:`), which avoids false hits like `on:` inside `python:`.
/// Returns `None` when nothing matches. Shared by the FFI and the CLI.
#[must_use]
pub fn detect(src: &str) -> Option<&'static str> {
    let has = |needle: &str| src.contains(needle);
    let key = |name: &str| {
        src.lines().any(|l| {
            let t = l.trim_start();
            t.starts_with(name) && t[name.len()..].trim_start().starts_with(':')
        })
    };
    let kind = if has("kind: pipeline") && key("steps") {
        "drone"
    } else if has("argoproj.io") {
        "argo"
    } else if has("tekton.dev") {
        "tekton"
    } else if key("pipelines") && has("- step:") {
        "bitbucket"
    } else if key("phases") && has("version: 0.2") {
        "aws_codebuild"
    } else if has("\"pipeline\"") && (has("\"Source\"") || has("\"actions\"")) {
        // aws_codepipeline definitions are JSON with a `pipeline` object.
        "aws_codepipeline"
    } else if key("sdk") && (key("name") && !key("steps") && !key("jobs")) {
        // dagger.json module manifest: name + sdk, no job/step structure.
        "dagger"
    } else if key("on") && key("jobs") {
        "github"
    } else if key("pool") || (key("trigger") && key("stages")) {
        "azure"
    } else if key("steps") && has("- task:") {
        "google_cloudbuild"
    } else if has("pipeline {") && has("agent") {
        "jenkins"
    } else if has("VERSION 0.") || (has("FROM ") && key("build")) {
        "earthly"
    } else if key("language") || key("dist") {
        "travis"
    } else if has("version: 2") && key("jobs") {
        "circleci"
    } else if key("steps") && (has("- command:") || has("- label:") || has("- wait")) {
        // buildkite steps carry command/label/wait — distinct from woodpecker's
        // named-map steps (`steps:\n  build:`).
        "buildkite"
    } else if key("steps") {
        "woodpecker"
    } else if key("stages") || key("script") {
        "gitlab"
    } else {
        return None;
    };
    Some(kind)
}

/// Detect a platform, preferring the file NAME when it is conventional
/// (`.gitlab-ci.yml`, `Jenkinsfile`, `bitbucket-pipelines.yml`, …) — the
/// strongest, least-forgeable signal — and falling back to [`detect`] on the
/// content. `path` may be a full path or a bare file name.
#[must_use]
pub fn detect_with_path(path: &str, src: &str) -> Option<&'static str> {
    if let Some(by_name) = detect_from_filename(path) {
        return Some(by_name);
    }
    detect(src)
}

/// Match a conventional pipeline file name to its platform. Returns `None` for
/// a generic name (`*.yml`) where only the content can decide.
#[must_use]
pub fn detect_from_filename(path: &str) -> Option<&'static str> {
    let file = path.rsplit(['/', '\\']).next().unwrap_or(path);
    // Directory-scoped conventions (the parent dir names the platform).
    if path.contains("/.github/workflows/") || path.contains("\\.github\\workflows\\") {
        return Some("github");
    }
    if path.contains("/.circleci/") || path.contains("\\.circleci\\") {
        return Some("circleci");
    }
    if path.contains("/.buildkite/") || path.contains("\\.buildkite\\") {
        return Some("buildkite");
    }
    Some(match file {
        ".gitlab-ci.yml" => "gitlab",
        ".drone.yml" => "drone",
        ".travis.yml" => "travis",
        ".woodpecker.yml" | ".woodpecker.yaml" => "woodpecker",
        "bitbucket-pipelines.yml" => "bitbucket",
        "azure-pipelines.yml" | "azure-pipelines.yaml" => "azure",
        "buildspec.yml" | "buildspec.yaml" => "aws_codebuild",
        "cloudbuild.yaml" | "cloudbuild.yml" => "google_cloudbuild",
        "Jenkinsfile" => "jenkins",
        "Earthfile" => "earthly",
        "dagger.json" => "dagger",
        _ => return None,
    })
}

/// Parse `src` into a CST `Document` using the right surface parser for the
/// platform (YAML for most; the Jenkinsfile / Earthfile DSL parsers otherwise).
///
/// # Errors
/// Returns [`ForwardError::Parse`] if the source is malformed.
pub fn parse(platform: &str, src: &str) -> Result<Document, ForwardError> {
    match platform {
        "jenkins" => {
            pipeline_jenkinsfile_cst::parse(src).map_err(|e| ForwardError::Parse(format!("{e:?}")))
        }
        "earthly" => {
            pipeline_earthfile_cst::parse(src).map_err(|e| ForwardError::Parse(format!("{e:?}")))
        }
        _ => pipeline_cst::parse(src).map_err(|e| ForwardError::Parse(format!("{e:?}"))),
    }
}

/// Seed a `TypedGraph` from an already-parsed CST `Document` for `platform`.
///
/// # Errors
/// Returns [`ForwardError::UnknownPlatform`] for an unrecognised platform.
pub fn seed(platform: &str, doc: &Document, source: &str) -> Result<TypedGraph, ForwardError> {
    use pipeline_tgg_seeder::platforms as p;
    let g = match platform {
        "argo" => p::argo::seed_from_document(doc, source).graph,
        "aws_codebuild" => p::aws_codebuild::seed_from_document(doc, source).graph,
        "aws_codepipeline" => p::aws_codepipeline::seed_from_document(doc, source).graph,
        "azure" => p::azure::seed_from_document(doc, source).graph,
        "bitbucket" => p::bitbucket::seed_from_document(doc, source).graph,
        "buildkite" => p::buildkite::seed_from_document(doc, source).graph,
        "circleci" => p::circleci::seed_from_document(doc, source).graph,
        "dagger" => p::dagger::seed_from_document(doc, source).graph,
        "drone" => p::drone::seed_from_document(doc, source).graph,
        "earthly" => p::earthly::seed_from_document(doc, source).graph,
        "github" => p::github::seed_from_document(doc, source).graph,
        "gitlab" => p::gitlab::seed_from_document(doc, source).graph,
        "google_cloudbuild" => p::google_cloudbuild::seed_from_document(doc, source).graph,
        "jenkins" => p::jenkins::seed_from_document(doc, source).graph,
        "tekton" => p::tekton::seed_from_document(doc, source).graph,
        "travis" => p::travis::seed_from_document(doc, source).graph,
        "woodpecker" => p::woodpecker::seed_from_document(doc, source).graph,
        other => return Err(ForwardError::UnknownPlatform(other.to_string())),
    };
    Ok(g)
}

/// The embedded ruleset JSON for a platform, if known.
fn ruleset_json(platform: &str) -> Option<&'static str> {
    Some(match platform {
        "argo" => include_str!("../../../catalog/rules/argo.ruleset.json"),
        "aws_codebuild" => include_str!("../../../catalog/rules/aws_codebuild.ruleset.json"),
        "aws_codepipeline" => include_str!("../../../catalog/rules/aws_codepipeline.ruleset.json"),
        "azure" => include_str!("../../../catalog/rules/azure.ruleset.json"),
        "bitbucket" => include_str!("../../../catalog/rules/bitbucket.ruleset.json"),
        "buildkite" => include_str!("../../../catalog/rules/buildkite.ruleset.json"),
        "circleci" => include_str!("../../../catalog/rules/circleci.ruleset.json"),
        "dagger" => include_str!("../../../catalog/rules/dagger.ruleset.json"),
        "drone" => include_str!("../../../catalog/rules/drone.ruleset.json"),
        "earthly" => include_str!("../../../catalog/rules/earthly.ruleset.json"),
        "github" => include_str!("../../../catalog/rules/github.ruleset.json"),
        "gitlab" => include_str!("../../../catalog/rules/gitlab.ruleset.json"),
        "google_cloudbuild" => {
            include_str!("../../../catalog/rules/google_cloudbuild.ruleset.json")
        }
        "jenkins" => include_str!("../../../catalog/rules/jenkins.ruleset.json"),
        "tekton" => include_str!("../../../catalog/rules/tekton.ruleset.json"),
        "travis" => include_str!("../../../catalog/rules/travis.ruleset.json"),
        "woodpecker" => include_str!("../../../catalog/rules/woodpecker.ruleset.json"),
        _ => return None,
    })
}

/// The compiled, instantiated bidirectional rule pool for a platform.
///
/// # Errors
/// Returns [`ForwardError::UnknownPlatform`] if no ruleset is embedded for it.
pub fn rule_pool(platform: &str) -> Result<Vec<Box<dyn Rule>>, ForwardError> {
    let json = ruleset_json(platform)
        .ok_or_else(|| ForwardError::UnknownPlatform(platform.to_string()))?;
    let spec: RuleSetSpec = serde_json::from_str(json)
        .map_err(|e| ForwardError::Cascade(format!("ruleset parse: {e}")))?;
    Ok(spec
        .rules
        .iter()
        .flat_map(|r| compile_bidirectional(r).expect("compile_bidirectional"))
        .map(|c| instantiate(&c))
        .collect())
}

/// Run the forward TGG cascade in place, growing the `hub:` Hub-IR. Only rules
/// whose input domain overlaps the current node kinds are activated.
///
/// # Errors
/// Returns [`ForwardError::Cascade`] if the cascade fails to terminate.
pub fn run_forward(graph: &mut TypedGraph, rules: &[Box<dyn Rule>]) -> Result<(), ForwardError> {
    let kinds: std::collections::HashSet<String> =
        graph.iter_nodes().map(|n| n.type_id.clone()).collect();
    let active: Vec<&dyn Rule> = rules
        .iter()
        .filter(|r| {
            let idk = r.input_domain_kinds();
            idk.is_empty() || idk.iter().any(|k| kinds.contains(k))
        })
        .map(AsRef::as_ref)
        .collect();
    let mut cascade = Cascade::new();
    run_cascade_cached(&mut cascade, graph, &active, 20_000)
        .map_err(|e| ForwardError::Cascade(format!("{e:?}")))?;
    Ok(())
}

/// The whole forward path: surface source → Hub-IR `TypedGraph`. The graph
/// retains the CST nodes alongside the grown `hub:` nodes (the cascade keeps
/// both poles); consumers read the `hub:` projection via `pipeline-hub-ir`.
///
/// # Errors
/// [`ForwardError`] for an unknown platform, a parse failure, or a cascade that
/// does not converge.
pub fn forward(platform: &str, src: &str) -> Result<TypedGraph, ForwardError> {
    if !is_supported(platform) {
        return Err(ForwardError::UnknownPlatform(platform.to_string()));
    }
    let doc = parse(platform, src)?;
    let mut graph = seed(platform, &doc, src)?;
    let rules = rule_pool(platform)?;
    run_forward(&mut graph, &rules)?;
    Ok(graph)
}

// ===========================================================================
// The TGG edit path: mutate the Hub-IR, then re-emit through the BACKWARD
// cascade. NOT byte-level patching — the IR is the canonical artefact; the
// surface syntax is regenerated from it (and so normalises to canonical form).
// This is the foundation a real bidirectional editor + the recipe system both
// build on: every edit (scalar change, add/remove step or job, insert a recipe
// subgraph) is a graph mutation followed by `re_emit`.
// ===========================================================================

/// Extract the `hub:`-typed subgraph (nodes + edges) into a fresh graph. The
/// backward cascade run over this hub-only graph materialises the CST pole.
#[must_use]
pub fn isolate_hub(g: &TypedGraph) -> TypedGraph {
    isolate_hub_excluding(g, &HashSet::new())
}

/// Like [`isolate_hub`], but drops the nodes in `exclude` (and any edge that
/// touches one). Used to delete a construct: omit its subtree from the hub the
/// backward cascade rebuilds from, and it simply isn't emitted.
fn isolate_hub_excluding(g: &TypedGraph, exclude: &HashSet<GhostId>) -> TypedGraph {
    let mut hub = TypedGraph::new();
    for nd in g.iter_nodes() {
        if nd.type_id.starts_with("hub:") && !exclude.contains(&nd.id) {
            hub.insert_node_data(nd.clone());
        }
    }
    for (s, t, e) in g.iter_edges() {
        if e.type_id.starts_with("hub:") && !exclude.contains(&s) && !exclude.contains(&t) {
            hub.insert_edge_data(s, t, e.clone());
        }
    }
    hub
}

/// The outermost `cst:Mapping` pipeline root — the node `emit` walks from. Found
/// after the backward cascade has materialised the CST pole.
fn cst_pipeline_root(g: &TypedGraph) -> Option<GhostId> {
    let inner: std::collections::HashSet<_> = g
        .iter_edges()
        .into_iter()
        .filter(|(s, _, e)| {
            e.type_id == "cst:value_of"
                && g.get_node(s)
                    .is_some_and(|n| n.type_id == "cst:MappingEntry")
        })
        .map(|(_, t, _)| t)
        .collect();
    g.iter_nodes()
        .filter(|n| {
            n.type_id == "cst:Mapping"
                && n.attrs.get("construct").map(String::as_str) == Some("pipeline")
        })
        .map(|n| n.id)
        .find(|m| !inner.contains(m))
}

/// Emit surface syntax from a graph whose CST pole is materialised, in the
/// platform's surface form (YAML, or the Jenkinsfile / Earthfile emitters).
fn emit(platform: &str, g: &TypedGraph, root: GhostId) -> String {
    match platform {
        "jenkins" => pipeline_tgg_seeder::emit_jenkinsfile::emit_jenkinsfile(g, root),
        "earthly" => pipeline_tgg_seeder::emit_earthfile::emit_earthfile(g, root),
        _ => pipeline_tgg_seeder::emit::emit_yaml(g, root),
    }
}

/// Re-emit surface syntax from a (possibly edited) graph: isolate its hub,
/// run the backward cascade to rebuild the CST from that hub, then emit. The
/// output is canonical — the IR is the source of truth, so formatting/key-order
/// normalise.
///
/// # Errors
/// [`ForwardError::Cascade`] if the backward cascade fails; [`ForwardError::Edit`]
/// if no pipeline root materialises (an empty / malformed hub).
pub fn re_emit(platform: &str, graph: &TypedGraph) -> Result<String, ForwardError> {
    re_emit_excluding(platform, graph, &HashSet::new())
}

/// [`re_emit`], omitting the nodes in `exclude` from the hub the backward
/// cascade rebuilds from (so a deleted construct's subtree never re-emits).
fn re_emit_excluding(
    platform: &str,
    graph: &TypedGraph,
    exclude: &HashSet<GhostId>,
) -> Result<String, ForwardError> {
    let mut hub = isolate_hub_excluding(graph, exclude);
    let rules = rule_pool(platform)?;
    run_forward(&mut hub, &rules)?;
    if platform == "argo" {
        // The argo seeder HOISTS `dag.tasks` / `inputs.parameters` onto the job
        // (it tolerates both the wrapped and bare forms), so the backward
        // cascade rebuilds them bare — emitting `tasks:` / `parameters:` directly
        // under the template, which isn't valid argo (it requires `dag:` /
        // `inputs:` wrappers). Re-nest them on the CST before emit. Safe for the
        // round-trip: the forward hoists the wrapped form back to the same hub.
        wrap_argo_containers(&mut hub);
    }
    let root = cst_pipeline_root(&hub)
        .ok_or_else(|| ForwardError::Edit("no pipeline root after backward cascade".to_string()))?;
    Ok(emit(platform, &hub, root))
}

/// Re-nest a job's hoisted argo container entries on the CST: `tasks:` under a
/// `dag:` wrapper and `parameters:` under an `inputs:` wrapper, so emit produces
/// valid argo. Tombstone the old direct edge + add the wrapper (Ghost-overlay).
fn wrap_argo_containers(hub: &mut TypedGraph) {
    let live = |s: Status| !matches!(s, Status::Tombstone | Status::TentativeTombstone);
    let jobs: Vec<GhostId> = hub
        .iter_nodes()
        .filter(|n| {
            n.type_id == "cst:Mapping"
                && n.attrs.get("construct").map(String::as_str) == Some("job")
        })
        .map(|n| n.id)
        .collect();
    for job in jobs {
        for (inner_key, wrapper_key) in [("tasks", "dag"), ("parameters", "inputs")] {
            // The job's direct `<inner_key>:` mapping-entry child, if any.
            let found = hub
                .iter_edges()
                .into_iter()
                .find(|(s, t, e)| {
                    *s == job
                        && e.type_id == "cst:has_child"
                        && live(e.status)
                        && hub
                            .get_node(t)
                            .and_then(|n| n.attrs.get("key").cloned())
                            .as_deref()
                            == Some(inner_key)
                })
                .map(|(_, t, e)| (e.id, t));
            let Some((edge_id, inner_entry)) = found else {
                continue;
            };
            // Preserve ordering: the wrapper takes the inner entry's span.
            let span = hub
                .get_node(&inner_entry)
                .map(|n| {
                    (
                        n.attrs.get("span_start").cloned().unwrap_or_default(),
                        n.attrs.get("span_end").cloned().unwrap_or_default(),
                    )
                })
                .unwrap_or_default();
            let mut entry_attrs = std::collections::BTreeMap::new();
            entry_attrs.insert("key".to_string(), wrapper_key.to_string());
            entry_attrs.insert("span_start".to_string(), span.0.clone());
            entry_attrs.insert("span_end".to_string(), span.1.clone());
            let wrap_entry =
                hub.add_solid_child_node(job, "cst:has_child", "cst:MappingEntry", entry_attrs);
            let mut map_attrs = std::collections::BTreeMap::new();
            map_attrs.insert("span_start".to_string(), span.0);
            map_attrs.insert("span_end".to_string(), span.1);
            let wrap_map =
                hub.add_solid_child_node(wrap_entry, "cst:value_of", "cst:Mapping", map_attrs);
            // add_solid_child_node mints the node; the walkable Solid edge needs
            // an explicit add_edge (mirrors normalize_job_containment).
            hub.add_edge(
                job,
                wrap_entry,
                "cst:has_child",
                std::collections::BTreeMap::new(),
                Status::Solid,
            );
            hub.add_edge(
                wrap_entry,
                wrap_map,
                "cst:value_of",
                std::collections::BTreeMap::new(),
                Status::Solid,
            );
            // Move the inner entry under the wrapper mapping.
            hub.add_edge(
                wrap_map,
                inner_entry,
                "cst:has_child",
                std::collections::BTreeMap::new(),
                Status::Solid,
            );
            hub.set_edge_status(&edge_id, Status::Tombstone);
        }
    }
}

/// Migrate `source` from platform `from` to platform `to`: forward to the
/// neutral Hub-IR, re-key the hub into `to`'s `prov_key` vocabulary (so `to`'s
/// key-gated backward rules fire), then re-emit `to`'s surface syntax. Fields
/// `to` doesn't model fall away — the expected interop loss.
///
/// # Errors
/// [`ForwardError`] for a parse / cascade / re-key failure.
pub fn migrate(from: &str, source: &str, to: &str) -> Result<String, ForwardError> {
    let graph = forward(from, source)?;
    // Cross-structural bridge (F1): the rekey path is field-name remapping, not
    // structural restructuring — so migrating between a JOB-based family
    // (gitlab/github/circleci: pipeline → jobs → steps) and a STEP-FLAT family
    // (drone/woodpecker: pipeline → steps) can't work through it (it produces
    // nothing, or a mis-shaped result). Detect the family crossing up front and
    // synthesise the target from the lifted, family-neutral model (which maps
    // both families to one: jobs with name/image/steps/needs).
    if is_step_flat(from) != is_step_flat(to) {
        if let Some(syn) =
            pipeline_render::lift(&graph).and_then(|p| synthesize_cross_structural(&p, to))
        {
            return Ok(syn);
        }
        // else fall through — the bridge doesn't synthesise this target yet.
    }
    let mut hub = isolate_hub(&graph);
    rekey_hub(&mut hub, &field_key_map(to)?);
    normalize_job_containment(&mut hub, to);
    let rules = rule_pool(to)?;
    run_forward(&mut hub, &rules)?;
    let root = cst_pipeline_root(&hub)
        .ok_or_else(|| ForwardError::Edit("no pipeline root after migration".to_string()))?;
    Ok(emit(to, &hub, root))
}

/// One thing that didn't survive a migration 1:1.
#[derive(Debug, Clone)]
pub struct Friction {
    /// `info` (1:1, surfaced) / `approximated` (reduced) / `manual` (dropped).
    pub severity: &'static str,
    /// The capability key (`cache`, `services`, …).
    pub feature: String,
    /// Human explanation.
    pub note: String,
}

/// Migrate AND report what the target couldn't represent. The friction report
/// is derived honestly, not declared: forward the SOURCE and re-forward the
/// MIGRATED output, then diff their capability-family counts. A family the
/// source uses that the target drops entirely is `manual`; one whose count
/// shrinks is `approximated`; a universal family that somehow regressed is
/// `info`. No silent loss — every reduction becomes a report line.
///
/// # Errors
/// [`ForwardError`] for a parse / cascade / re-key failure of the source.
pub fn migrate_with_report(
    from: &str,
    source: &str,
    to: &str,
) -> Result<(String, Vec<Friction>), ForwardError> {
    let src_graph = forward(from, source)?;
    let yaml = migrate(from, source, to)?;
    // Re-forward the migrated output in the TARGET's vocabulary to see what
    // actually survived. If that fails, we can't diff — report nothing rather
    // than guess (the migration itself still succeeded).
    let Ok(dst_graph) = forward(to, &yaml) else {
        return Ok((yaml, Vec::new()));
    };
    let before = pipeline_render::feature_counts(&src_graph);
    let after = pipeline_render::feature_counts(&dst_graph);
    let after_count = |key: &str| after.iter().find(|f| f.key == key).map_or(0, |f| f.count);

    let mut report = Vec::new();
    for f in &before {
        let now = after_count(f.key);
        if now >= f.count {
            continue; // fully preserved (or grew via normalization)
        }
        let (severity, note) = if now == 0 {
            let sev = if f.universal { "info" } else { "manual" };
            (
                sev,
                format!(
                    "{} ({}×) not represented in {to} — rewrite by hand",
                    f.label, f.count
                ),
            )
        } else {
            (
                "approximated",
                format!(
                    "{}: {} of {} survived to {to}; review the rest",
                    f.label, now, f.count
                ),
            )
        };
        report.push(Friction {
            severity,
            feature: f.key.to_string(),
            note,
        });
    }
    Ok((yaml, report))
}

/// A STEP-FLAT platform models work as a top-level step list with no job layer
/// (`drone`/`woodpecker`/`buildkite`/`google_cloudbuild`); the rest are JOB-based
/// (pipeline → jobs → steps). Migrating across the two families needs the
/// structural bridge in [`migrate`], not plain field re-keying.
fn is_step_flat(platform: &str) -> bool {
    matches!(
        platform,
        "drone" | "woodpecker" | "buildkite" | "google_cloudbuild"
    )
}

/// A job's container image (its `image` param), if declared.
fn model_image(j: &pipeline_render::Job) -> Option<&str> {
    j.params
        .iter()
        .find(|p| p.key == "image")
        .map(|p| p.value.as_str())
}

/// Single-quote a YAML scalar when it carries characters that would otherwise
/// need escaping; pass plain identifiers through bare.
fn yq(s: &str) -> String {
    if !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '/'))
    {
        s.to_string()
    } else {
        format!("'{}'", s.replace('\'', "''"))
    }
}

/// Synthesise `to`'s surface YAML from the lifted, family-neutral pipeline model
/// — the cross-structural migration bridge. A job (name / image / steps / needs)
/// maps to a step-flat platform's step (commands from steps, `depends_on` from
/// needs) or a job-based platform's job (script from steps). Returns `None` for
/// targets this bridge doesn't synthesise (callers keep the normal result).
// One flat match arm per target platform — the length is additive emitter
// boilerplate (six dialects), not nested complexity; splitting it would only
// scatter the per-platform shapes.
#[allow(clippy::too_many_lines)]
fn synthesize_cross_structural(p: &pipeline_render::Pipeline, to: &str) -> Option<String> {
    use std::fmt::Write as _;
    if p.jobs.is_empty() {
        return None;
    }
    let name = p.name.clone().unwrap_or_else(|| "pipeline".to_string());
    let deps_list =
        |j: &pipeline_render::Job| j.needs.iter().map(|n| yq(n)).collect::<Vec<_>>().join(", ");
    let mut s = String::new();
    match to {
        // Step-flat: each job becomes a top-level step; its commands are the
        // step's command list, its needs the step's depends_on.
        "drone" | "woodpecker" => {
            if to == "drone" {
                let _ = writeln!(s, "kind: pipeline\nname: {}", yq(&name));
            }
            s.push_str("steps:\n");
            for j in &p.jobs {
                let _ = writeln!(s, "  - name: {}", yq(&j.name));
                if let Some(img) = model_image(j) {
                    let _ = writeln!(s, "    image: {}", yq(img));
                }
                if !j.needs.is_empty() {
                    let _ = writeln!(s, "    depends_on: [{}]", deps_list(j));
                }
                s.push_str("    commands:\n");
                for st in &j.steps {
                    let _ = writeln!(s, "      - {}", yq(&st.label));
                }
            }
        }
        // Step-flat (buildkite): each job becomes a command step; the catalog
        // models `image` directly on the commandStep, depends_on takes labels.
        "buildkite" => {
            s.push_str("steps:\n");
            for j in &p.jobs {
                let _ = writeln!(s, "  - label: {}", yq(&j.name));
                if let Some(img) = model_image(j) {
                    let _ = writeln!(s, "    image: {}", yq(img));
                }
                if !j.needs.is_empty() {
                    let _ = writeln!(s, "    depends_on: [{}]", deps_list(j));
                }
                s.push_str("    commands:\n");
                for st in &j.steps {
                    let _ = writeln!(s, "      - {}", yq(&st.label));
                }
            }
        }
        // Step-flat (google_cloudbuild): each job becomes a builder step —
        // `name` IS the image (required by GCB; default builder when the model
        // has none), commands go into `script`, needs into `waitFor`.
        "google_cloudbuild" => {
            s.push_str("steps:\n");
            for j in &p.jobs {
                let _ = writeln!(s, "  - id: {}", yq(&j.name));
                let _ = writeln!(s, "    name: {}", yq(model_image(j).unwrap_or("ubuntu")));
                if !j.needs.is_empty() {
                    s.push_str("    waitFor:\n");
                    for n in &j.needs {
                        let _ = writeln!(s, "      - {}", yq(n));
                    }
                }
                match j.steps.as_slice() {
                    [only] => {
                        let _ = writeln!(s, "    script: {}", yq(&only.label));
                    }
                    many => {
                        s.push_str("    script: |\n");
                        for st in many {
                            let _ = writeln!(s, "      {}", st.label);
                        }
                    }
                }
            }
        }
        // Job-based (gitlab): each job is a top-level keyed mapping with script.
        "gitlab" => {
            for j in &p.jobs {
                let _ = writeln!(s, "{}:", yq(&j.name));
                if let Some(img) = model_image(j) {
                    let _ = writeln!(s, "  image: {}", yq(img));
                }
                if !j.needs.is_empty() {
                    let _ = writeln!(s, "  needs: [{}]", deps_list(j));
                }
                s.push_str("  script:\n");
                for st in &j.steps {
                    let _ = writeln!(s, "    - {}", yq(&st.label));
                }
            }
        }
        // Job-based (github): jobs map, each with a steps list of run actions.
        "github" => {
            s.push_str("on: push\njobs:\n");
            for j in &p.jobs {
                let _ = writeln!(s, "  {}:", yq(&j.name));
                if !j.needs.is_empty() {
                    let _ = writeln!(s, "    needs: [{}]", deps_list(j));
                }
                s.push_str("    steps:\n");
                for st in &j.steps {
                    let _ = writeln!(s, "      - run: {}", yq(&st.label));
                }
            }
        }
        // Job-based (circleci): jobs map (docker executor + run steps), the DAG
        // lives in a workflow (`requires`).
        "circleci" => {
            s.push_str("version: 2.1\njobs:\n");
            for j in &p.jobs {
                let _ = writeln!(s, "  {}:", yq(&j.name));
                if let Some(img) = model_image(j) {
                    s.push_str("    docker:\n");
                    let _ = writeln!(s, "      - image: {}", yq(img));
                }
                s.push_str("    steps:\n");
                for st in &j.steps {
                    let _ = writeln!(s, "      - run: {}", yq(&st.label));
                }
            }
            s.push_str("workflows:\n");
            let _ = writeln!(s, "  {}:", yq(&name));
            s.push_str("    jobs:\n");
            for j in &p.jobs {
                if j.needs.is_empty() {
                    let _ = writeln!(s, "      - {}", yq(&j.name));
                } else {
                    let _ = writeln!(s, "      - {}:", yq(&j.name));
                    s.push_str("          requires:\n");
                    for n in &j.needs {
                        let _ = writeln!(s, "            - {}", yq(n));
                    }
                }
            }
        }
        // Job-based (azure): a `jobs:` list of `- job:` entries with script
        // steps; image maps to `container`, needs to `dependsOn`.
        "azure" => {
            s.push_str("jobs:\n");
            for j in &p.jobs {
                let _ = writeln!(s, "  - job: {}", yq(&j.name));
                if let Some(img) = model_image(j) {
                    let _ = writeln!(s, "    container: {}", yq(img));
                }
                if !j.needs.is_empty() {
                    let _ = writeln!(s, "    dependsOn: [{}]", deps_list(j));
                }
                s.push_str("    steps:\n");
                for st in &j.steps {
                    let _ = writeln!(s, "      - script: {}", yq(&st.label));
                }
            }
        }
        _ => return None,
    }
    Some(s)
}

/// How a platform contains its pipeline's jobs in the hub: a direct keyless
/// `pipeline --has_job--> job` edge (gitlab/earthly), or an `attr[name=jobs]`+
/// collection wrapper (github/circleci/tekton/travis/azure/…). The two are the
/// SAME semantics in two shapes; [`normalize_job_containment`] re-shapes a hub to
/// the target's form so jobs cross faithfully (not as bare top-level keys / not
/// dropped). Read deterministically from the embedded ruleset.
enum JobForm {
    Keyless,
    AttrCollection { key: String, vkind: Option<String> },
    Unknown,
}

// `&kind` (needless_borrow) vs `|id| kind(id)` (redundant_closure) is a clippy
// self-contradiction for a multiply-used capturing closure; keep the closure form.
#[allow(clippy::redundant_closure)]
fn target_job_form(platform: &str) -> JobForm {
    let Some(json) = ruleset_json(platform) else {
        return JobForm::Unknown;
    };
    let Ok(v) = serde_json::from_str::<serde_json::Value>(json) else {
        return JobForm::Unknown;
    };
    let mut attr_collection: Option<(String, Option<String>)> = None;
    for rule in v["rules"].as_array().into_iter().flatten() {
        let nodes = &rule["r_pattern"]["nodes"];
        let edges = &rule["r_pattern"]["edges"];
        let kind = |id: &str| -> Option<String> {
            nodes
                .as_array()?
                .iter()
                .find(|n| n["id"].as_str() == Some(id))
                .and_then(|n| n["kind"].as_str())
                .map(ToString::to_string)
        };
        // Keyless: a direct pipeline --has_job--> job edge → no wrapping.
        let keyless = edges.as_array().into_iter().flatten().any(|e| {
            e["kind"].as_str() == Some("hub:has_job")
                && e["source_node_id"]
                    .as_str()
                    .and_then(|id| kind(id))
                    .as_deref()
                    == Some("hub:pipeline")
        });
        if keyless {
            return JobForm::Keyless;
        }
        let has_item_job = edges.as_array().into_iter().flatten().any(|e| {
            e["kind"].as_str() == Some("hub:has_item")
                && e["target_node_id"]
                    .as_str()
                    .and_then(|id| kind(id))
                    .as_deref()
                    == Some("hub:job")
        });
        if has_item_job {
            for n in nodes.as_array().into_iter().flatten() {
                if n["kind"].as_str() != Some("hub:attr") {
                    continue;
                }
                let cons: std::collections::HashMap<&str, &str> = n["constraints"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|c| Some((c["name"].as_str()?, c["matcher"]["value"].as_str()?)))
                    .collect();
                let owned_by_pipeline = edges.as_array().into_iter().flatten().any(|e| {
                    e["kind"].as_str() == Some("hub:has_attr")
                        && e["source_node_id"]
                            .as_str()
                            .and_then(|id| kind(id))
                            .as_deref()
                            == Some("hub:pipeline")
                        && e["target_node_id"].as_str() == n["id"].as_str()
                });
                if owned_by_pipeline
                    && cons.get("name") == Some(&"jobs")
                    && attr_collection.is_none()
                {
                    if let Some(k) = cons.get("prov_key") {
                        attr_collection = Some((
                            (*k).to_string(),
                            cons.get("vkind").map(|s| (*s).to_string()),
                        ));
                    }
                }
            }
        }
    }
    match attr_collection {
        Some((key, vkind)) => JobForm::AttrCollection { key, vkind },
        None => JobForm::Unknown,
    }
}

/// Bridge job-containment forms to the target's, in place (tombstone + add — the
/// Ghost-overlay marks an edge deleted, the matchable view hides it; no rebuild).
/// Keyless `pipeline --has_job--> job` ⇄ `attr[name=jobs]`+collection are the same
/// jobs in two shapes; re-shape to the form the TARGET's backward consumes.
#[allow(
    clippy::too_many_lines,
    reason = "per-platform job-containment reshaping; one cohesive pass"
)]
fn normalize_job_containment(hub: &mut TypedGraph, to: &str) {
    let live =
        |e_status: Status| !matches!(e_status, Status::Tombstone | Status::TentativeTombstone);
    let pipes: Vec<GhostId> = hub
        .iter_nodes()
        .filter(|n| n.type_id == "hub:pipeline")
        .map(|n| n.id)
        .collect();
    match target_job_form(to) {
        JobForm::AttrCollection { key, vkind } => {
            for pid in pipes {
                let job_edges: Vec<(GhostId, GhostId)> = hub
                    .iter_edges()
                    .into_iter()
                    .filter(|(s, _, e)| *s == pid && e.type_id == "hub:has_job" && live(e.status))
                    .map(|(_, t, e)| (e.id, t))
                    .collect();
                if job_edges.is_empty() {
                    continue;
                }
                let mut a_attrs = std::collections::BTreeMap::new();
                a_attrs.insert("name".to_string(), "jobs".to_string());
                a_attrs.insert("prov_key".to_string(), key.clone());
                if let Some(vk) = &vkind {
                    a_attrs.insert("vkind".to_string(), vk.clone());
                }
                let attr = hub.add_solid_child_node(pid, "hub:has_attr", "hub:attr", a_attrs);
                let coll = hub.add_solid_child_node(
                    attr,
                    "hub:has_value",
                    "hub:collection",
                    std::collections::BTreeMap::new(),
                );
                hub.add_edge(
                    pid,
                    attr,
                    "hub:has_attr",
                    std::collections::BTreeMap::new(),
                    Status::Solid,
                );
                hub.add_edge(
                    attr,
                    coll,
                    "hub:has_value",
                    std::collections::BTreeMap::new(),
                    Status::Solid,
                );
                for (eid, job) in job_edges {
                    hub.set_edge_status(&eid, Status::Tombstone);
                    hub.add_edge(
                        coll,
                        job,
                        "hub:has_item",
                        std::collections::BTreeMap::new(),
                        Status::Solid,
                    );
                }
            }
        }
        JobForm::Keyless => {
            for pid in pipes {
                let job_attrs: Vec<(GhostId, GhostId)> = hub
                    .iter_edges()
                    .into_iter()
                    .filter(|(s, t, e)| {
                        *s == pid
                            && e.type_id == "hub:has_attr"
                            && live(e.status)
                            && hub
                                .get_node(t)
                                .and_then(|n| n.attrs.get("name").cloned())
                                .as_deref()
                                == Some("jobs")
                    })
                    .map(|(_, t, e)| (e.id, t))
                    .collect();
                for (attr_edge, attr) in job_attrs {
                    let colls: Vec<GhostId> = hub
                        .iter_edges()
                        .into_iter()
                        .filter(|(s, t, e)| {
                            *s == attr
                                && e.type_id == "hub:has_value"
                                && live(e.status)
                                && hub
                                    .get_node(t)
                                    .is_some_and(|n| n.type_id == "hub:collection")
                        })
                        .map(|(_, t, _)| t)
                        .collect();
                    let mut jobs: Vec<GhostId> = Vec::new();
                    for c in &colls {
                        for (s, t, e) in hub.iter_edges() {
                            if s == *c
                                && e.type_id == "hub:has_item"
                                && live(e.status)
                                && hub.get_node(&t).is_some_and(|n| n.type_id == "hub:job")
                            {
                                jobs.push(t);
                            }
                        }
                    }
                    if jobs.is_empty() {
                        continue;
                    }
                    for job in jobs {
                        hub.add_edge(
                            pid,
                            job,
                            "hub:has_job",
                            std::collections::BTreeMap::new(),
                            Status::Solid,
                        );
                    }
                    hub.set_edge_status(&attr_edge, Status::Tombstone);
                    hub.set_node_status(&attr, Status::Tombstone);
                }
            }
        }
        JobForm::Unknown => {}
    }
}

/// The target `prov_key`s a platform emits for one `(construct, field)`, split
/// by the attribute's structural shape — a scalar value vs. a sequence
/// (collection). Platforms often spell the SAME semantic field two ways (e.g.
/// google-cloudbuild's step command is the scalar `script` OR the sequence
/// `args`); re-keying must pick the spelling that matches the live attr's shape,
/// or a collection-shaped value gets forced onto a scalar key and dropped by the
/// target's backward cascade (the gcb→drone drift root cause).
/// One target spelling for a field at a given shape: the surface `prov_key`
/// plus the `vkind` the target's rule annotates it with. The target's BACKWARD
/// cascade keys off BOTH (e.g. gcb's `options:` block rule requires
/// `vkind=block`), so re-keying must carry the vkind too — not just the key — or
/// a block sourced from a platform that doesn't annotate `vkind` (acb `batch`,
/// bb `options`) fails gcb's constraint and the whole block is dropped.
#[derive(Clone)]
struct KeySpec {
    key: String,
    vkind: Option<String>,
}

#[derive(Default, Clone)]
struct ShapeKeys {
    scalar: Option<KeySpec>,
    seq: Option<KeySpec>,
}

/// `true` if `attr_id`'s `hub:has_value` edge (within the JSON rule pattern's
/// node/edge arrays) targets a `hub:collection` node — i.e. the attr is a
/// sequence, not a scalar.
fn rule_attr_is_seq(nodes: &serde_json::Value, edges: &serde_json::Value, attr_id: &str) -> bool {
    let kind_of = |id: &str| -> Option<&str> {
        nodes
            .as_array()?
            .iter()
            .find(|n| n["id"].as_str() == Some(id))
            .and_then(|n| n["kind"].as_str())
    };
    edges.as_array().into_iter().flatten().any(|e| {
        e["kind"].as_str() == Some("hub:has_value")
            && e["source_node_id"].as_str() == Some(attr_id)
            && e["target_node_id"].as_str().and_then(kind_of) == Some("hub:collection")
    })
}

/// For a construct-reference attr (its `hub:has_value` targets a nested CONSTRUCT
/// node like `hub:trigger`, not a plain `hub:value`/`hub:collection`), the
/// construct that OWNS the attr (source of its `hub:has_attr` edge). Such a rule
/// names two constructs (owner + referent), so "last construct wins" picks the
/// referent; the structural owner is the defined truth. Limited to
/// construct-reference attrs so collection-container fields stay last-construct.
fn rule_attr_construct_owner(
    nodes: &serde_json::Value,
    edges: &serde_json::Value,
    attr_id: &str,
) -> Option<String> {
    let kind_of = |id: &str| -> Option<&str> {
        nodes
            .as_array()?
            .iter()
            .find(|n| n["id"].as_str() == Some(id))
            .and_then(|n| n["kind"].as_str())
    };
    let edge_arr = edges.as_array()?;
    let refers_construct = edge_arr.iter().any(|e| {
        e["kind"].as_str() == Some("hub:has_value")
            && e["source_node_id"].as_str() == Some(attr_id)
            && e["target_node_id"]
                .as_str()
                .and_then(kind_of)
                .and_then(|k| k.strip_prefix("hub:"))
                .is_some_and(|k| !matches!(k, "value" | "collection" | "attr"))
    });
    if !refers_construct {
        return None;
    }
    edge_arr.iter().find_map(|e| {
        (e["kind"].as_str() == Some("hub:has_attr")
            && e["target_node_id"].as_str() == Some(attr_id))
        .then(|| e["source_node_id"].as_str().and_then(kind_of))
        .flatten()
        .and_then(|k| k.strip_prefix("hub:"))
        .map(ToString::to_string)
    })
}

/// The primary `prov_key` per `(construct, field)` that `platform` emits with,
/// split by attribute shape — read from its embedded ruleset (first rule per
/// (pair, shape) wins; ir.toml lists the canonical key first).
fn field_key_map(
    platform: &str,
) -> Result<std::collections::HashMap<(String, String), ShapeKeys>, ForwardError> {
    let json = ruleset_json(platform)
        .ok_or_else(|| ForwardError::UnknownPlatform(platform.to_string()))?;
    let v: serde_json::Value = serde_json::from_str(json)
        .map_err(|e| ForwardError::Cascade(format!("ruleset parse: {e}")))?;
    let mut m: std::collections::HashMap<(String, String), ShapeKeys> =
        std::collections::HashMap::new();
    for rule in v["rules"].as_array().into_iter().flatten() {
        let nodes = &rule["r_pattern"]["nodes"];
        let edges = &rule["r_pattern"]["edges"];
        let (mut construct, mut field, mut key, mut vkind, mut attr_id) =
            (None, None, None, None, None);
        for n in nodes.as_array().into_iter().flatten() {
            let kind = n["kind"].as_str().unwrap_or("");
            if let Some(c) = kind.strip_prefix("hub:") {
                if !matches!(c, "attr" | "value" | "collection") {
                    construct = Some(c.to_string());
                }
            }
            if kind == "hub:attr" {
                attr_id = n["id"].as_str().map(ToString::to_string);
                for c in n["constraints"].as_array().into_iter().flatten() {
                    match c["name"].as_str().unwrap_or("") {
                        "name" => field = c["matcher"]["value"].as_str().map(ToString::to_string),
                        "prov_key" => key = c["matcher"]["value"].as_str().map(ToString::to_string),
                        "vkind" => vkind = c["matcher"]["value"].as_str().map(ToString::to_string),
                        _ => {}
                    }
                }
            }
        }
        if let Some(aid) = &attr_id {
            if let Some(owner) = rule_attr_construct_owner(nodes, edges, aid) {
                construct = Some(owner);
            }
        }
        if let (Some(c), Some(f), Some(k)) = (construct, field, key) {
            let is_seq = attr_id
                .as_deref()
                .is_some_and(|id| rule_attr_is_seq(nodes, edges, id));
            let slot = m.entry((c, f)).or_default();
            let target = if is_seq {
                &mut slot.seq
            } else {
                &mut slot.scalar
            };
            if target.is_none() {
                *target = Some(KeySpec { key: k, vkind });
            }
        }
    }
    Ok(m)
}

/// `true` if the live `hub:attr` `id` has a `hub:has_value` edge to a
/// `hub:collection` node (i.e. it is a sequence, not a scalar).
fn attr_is_seq(hub: &TypedGraph, id: GhostId) -> bool {
    out_one(hub, id, "hub:has_value")
        .and_then(|v| hub.get_node(&v))
        .is_some_and(|n| n.type_id == "hub:collection")
}

/// Re-key a hub into a target's vocabulary: for each `hub:attr`, look up the
/// target's primary `prov_key` for its `(owning-construct, name)` AT THE ATTR'S
/// SHAPE (scalar vs. sequence) and overwrite it. When the target spells the
/// field only in the other shape, fall back to that single spelling; when the
/// target doesn't model the field at all, the key is left unchanged (and falls
/// away in the target's backward — the expected interop intersection loss).
fn rekey_hub(hub: &mut TypedGraph, fk: &std::collections::HashMap<(String, String), ShapeKeys>) {
    let mut owner: std::collections::HashMap<GhostId, String> = std::collections::HashMap::new();
    for (s, t, e) in hub.iter_edges() {
        if e.type_id == "hub:has_attr" {
            if let Some(sn) = hub.get_node(&s) {
                owner.insert(
                    t,
                    sn.type_id
                        .strip_prefix("hub:")
                        .unwrap_or(&sn.type_id)
                        .to_string(),
                );
            }
        }
    }
    let ids: Vec<GhostId> = hub
        .iter_nodes()
        .filter(|n| n.type_id == "hub:attr")
        .map(|n| n.id)
        .collect();
    for id in ids {
        let (Some(field), Some(c)) = (
            hub.get_node(&id).and_then(|n| n.attrs.get("name").cloned()),
            owner.get(&id).cloned(),
        ) else {
            continue;
        };
        if let Some(sk) = fk.get(&(c, field)) {
            let seq = attr_is_seq(hub, id);
            // Prefer the spelling matching this attr's shape; `.or` only crosses
            // shape for a single-representation field (where the other slot is
            // None), never when the target offers both.
            let spec = if seq {
                sk.seq.clone().or_else(|| sk.scalar.clone())
            } else {
                sk.scalar.clone().or_else(|| sk.seq.clone())
            };
            if let Some(spec) = spec {
                hub.set_node_attr(&id, "prov_key", &spec.key);
                // Adopt the target's vkind so its backward constraint matches; if
                // the target rule annotates none, leave the existing vkind (a
                // no-vkind backward rule matches regardless).
                if let Some(vk) = spec.vkind {
                    hub.set_node_attr(&id, "vkind", &vk);
                }
            }
        }
    }
}

/// The single out-neighbour of `src` along `edge_kind`, if any.
fn out_one(g: &TypedGraph, src: GhostId, edge_kind: &str) -> Option<GhostId> {
    g.iter_edges()
        .into_iter()
        .find(|(s, _, e)| *s == src && e.type_id == edge_kind)
        .map(|(_, t, _)| t)
}

/// Set the scalar value at `anchor_hex` (a `data-hub` `GhostId` from the diagram)
/// to `new_value`, mutating the Hub-IR in place. Resolves the anchor to its
/// scalar carrier: a `hub:value` (its `text`), the `hub:value` behind a
/// `has_value` edge, or an inline `name`/`value` attr (e.g. `hub:image`).
///
/// # Errors
/// [`ForwardError::Edit`] if the hex is malformed, the node is absent, or it
/// carries no editable scalar.
pub fn set_value(
    graph: &mut TypedGraph,
    anchor_hex: &str,
    new_value: &str,
) -> Result<(), ForwardError> {
    let id = GhostId::from_hex(anchor_hex)
        .ok_or_else(|| ForwardError::Edit(format!("bad ghost-id hex: {anchor_hex}")))?;
    let node = graph
        .get_node(&id)
        .ok_or_else(|| ForwardError::Edit("no node with that id in the graph".to_string()))?;
    if node.type_id == "hub:value" {
        graph.set_node_attr(&id, "text", new_value);
        return Ok(());
    }
    if let Some(v) = out_one(graph, id, "hub:has_value") {
        if graph.get_node(&v).is_some_and(|n| n.type_id == "hub:value") {
            graph.set_node_attr(&v, "text", new_value);
            return Ok(());
        }
    }
    for key in ["name", "value"] {
        if node.attrs.contains_key(key) {
            graph.set_node_attr(&id, key, new_value);
            return Ok(());
        }
    }
    Err(ForwardError::Edit(
        "anchor carries no editable scalar".to_string(),
    ))
}

/// Apply a value edit and return the re-emitted surface source: forward the
/// source to a graph, mutate the Hub-IR at `anchor_hex`, then re-emit through
/// the backward cascade. The output is canonical (see [`re_emit`]).
///
/// # Errors
/// [`ForwardError`] for a parse / cascade / edit failure.
pub fn edit(
    platform: &str,
    source: &str,
    anchor_hex: &str,
    new_value: &str,
) -> Result<String, ForwardError> {
    let mut graph = forward(platform, source)?;
    set_value(&mut graph, anchor_hex, new_value)?;
    re_emit(platform, &graph)
}

// ===========================================================================
// Structural edits: duplicate / delete a construct (job, step, …) by cloning
// or omitting its hub subgraph, then re-emitting backward. These are the
// primitives copy/paste and the recipe system build on.
// ===========================================================================

/// Every node reachable from `root` through outgoing edges — a construct's
/// subtree (the node, its attr satellites, values, collections, items).
fn subtree(g: &TypedGraph, root: GhostId) -> Vec<GhostId> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    let mut q = VecDeque::from([root]);
    while let Some(id) = q.pop_front() {
        if !seen.insert(id) {
            continue;
        }
        out.push(id);
        for (s, t, _) in g.iter_edges() {
            if s == id {
                q.push_back(t);
            }
        }
    }
    out
}

/// The `hub:collection` that holds `node` as a `has_item`, if any — i.e. the
/// sequence a job/step lives in, the natural place to insert a duplicate.
fn parent_collection(g: &TypedGraph, node: GhostId) -> Option<GhostId> {
    g.iter_edges()
        .into_iter()
        .find(|(s, t, e)| {
            *t == node
                && e.type_id == "hub:has_item"
                && g.get_node(s).is_some_and(|n| n.type_id == "hub:collection")
        })
        .map(|(s, _, _)| s)
}

/// Deep-clone `root`'s subtree in `g` with fresh ids (salted by `salt` so two
/// clones never collide), returning the new root. The clipboard / copy
/// primitive — within one graph (duplicate) or across graphs (recipe graft).
fn clone_subtree(g: &mut TypedGraph, root: GhostId, salt: &str) -> GhostId {
    let sub = subtree(g, root);
    let idmap: std::collections::BTreeMap<GhostId, GhostId> = sub
        .iter()
        .map(|id| (*id, GhostId::from_opaque(&format!("{salt}#{}", id.hex()))))
        .collect();
    for id in &sub {
        if let Some(nd) = g.get_node(id) {
            let mut clone = nd.clone();
            clone.id = idmap[id];
            g.insert_node_data(clone);
        }
    }
    let edges: Vec<_> = g
        .iter_edges()
        .into_iter()
        .filter(|(s, t, _)| idmap.contains_key(s) && idmap.contains_key(t))
        .map(|(s, t, e)| (s, t, e.type_id.clone(), e.attrs.clone(), e.status))
        .collect();
    for (s, t, ty, attrs, status) in edges {
        g.add_edge(idmap[&s], idmap[&t], &ty, attrs, status);
    }
    idmap[&root]
}

/// Duplicate the construct at `anchor_hex` (a job or step): clone its subtree
/// and append the copy to the same collection, then re-emit. This is in-place
/// copy/paste.
///
/// # Errors
/// [`ForwardError`] if the node is unknown or isn't a collection item.
pub fn duplicate(platform: &str, source: &str, anchor_hex: &str) -> Result<String, ForwardError> {
    let mut g = forward(platform, source)?;
    let id = GhostId::from_hex(anchor_hex)
        .ok_or_else(|| ForwardError::Edit(format!("bad ghost-id hex: {anchor_hex}")))?;
    if g.get_node(&id).is_none() {
        return Err(ForwardError::Edit("no node with that id".to_string()));
    }
    let coll = parent_collection(&g, id).ok_or_else(|| {
        ForwardError::Edit(
            "this construct is not a list item — nothing to duplicate into".to_string(),
        )
    })?;
    let new_root = clone_subtree(&mut g, id, "dup");
    g.add_edge(
        coll,
        new_root,
        "hub:has_item",
        std::collections::BTreeMap::new(),
        Status::Solid,
    );
    re_emit(platform, &g)
}

/// Delete the construct at `anchor_hex` (a job or step): omit its subtree from
/// the hub the backward cascade rebuilds from, then re-emit.
///
/// # Errors
/// [`ForwardError`] if the node is unknown.
pub fn delete(platform: &str, source: &str, anchor_hex: &str) -> Result<String, ForwardError> {
    let g = forward(platform, source)?;
    let id = GhostId::from_hex(anchor_hex)
        .ok_or_else(|| ForwardError::Edit(format!("bad ghost-id hex: {anchor_hex}")))?;
    if g.get_node(&id).is_none() {
        return Err(ForwardError::Edit("no node with that id".to_string()));
    }
    let exclude: HashSet<GhostId> = subtree(&g, id).into_iter().collect();
    re_emit_excluding(platform, &g, &exclude)
}

#[cfg(test)]
mod tests {
    use super::*;

    const GITLAB: &str = "build:\n  script:\n    - cargo build\ntest:\n  needs:\n    - build\n  script:\n    - cargo test\n";

    #[test]
    fn detect_covers_previously_missed_platforms() {
        // buildkite was never detected (fell through to woodpecker).
        assert_eq!(
            detect("steps:\n  - label: build\n    command: make\n"),
            Some("buildkite")
        );
        // dagger.json manifest (name + sdk, no jobs/steps).
        assert_eq!(detect("name: mod\nsdk: go\n"), Some("dagger"));
        // woodpecker (named-map steps) still wins when there's no command/label.
        assert_eq!(
            detect("steps:\n  build:\n    image: alpine\n"),
            Some("woodpecker")
        );
    }

    #[test]
    fn filename_beats_content() {
        // A file NAMED .gitlab-ci.yml is gitlab even if its content has the
        // github markers (on: + jobs:) — the Roast's misclassification trap.
        let tricky = "on:\n  script: [x]\njobs:\n  script: [y]\n";
        assert_eq!(detect(tricky), Some("github"), "content alone reads github");
        assert_eq!(
            detect_with_path("/repo/.gitlab-ci.yml", tricky),
            Some("gitlab")
        );
        // A scripted Jenkinsfile (no `pipeline {`) is caught by name.
        assert_eq!(
            detect("node { sh 'make' }\n"),
            None,
            "content alone can't tell"
        );
        assert_eq!(
            detect_with_path("ci/Jenkinsfile", "node { sh 'make' }\n"),
            Some("jenkins")
        );
        // Directory-scoped conventions.
        assert_eq!(
            detect_from_filename("/r/.github/workflows/ci.yml"),
            Some("github")
        );
        assert_eq!(
            detect_from_filename("/r/.circleci/config.yml"),
            Some("circleci")
        );
        // A generic name defers to content.
        assert_eq!(detect_from_filename("pipeline.yml"), None);
    }

    /// A hub construct's name: its `name` satellite's value, or an inline attr.
    fn name_of(g: &TypedGraph, id: GhostId) -> Option<String> {
        for a in subtree(g, id).into_iter().skip(1) {
            let an = g.get_node(&a)?;
            if an.type_id == "hub:attr" && an.attrs.get("name").map(String::as_str) == Some("name")
            {
                if let Some(v) = out_one(g, a, "hub:has_value") {
                    if let Some(t) = g.get_node(&v).and_then(|n| n.attrs.get("text")) {
                        return Some(t.clone());
                    }
                }
                return an.attrs.get("value").cloned();
            }
        }
        None
    }

    #[test]
    fn forward_gitlab_grows_hub_jobs() {
        let g = forward("gitlab", GITLAB).expect("forward gitlab");
        let jobs = g.iter_nodes().filter(|n| n.type_id == "hub:job").count();
        assert_eq!(jobs, 2, "expected two hub:job nodes");
    }

    #[test]
    fn unknown_platform_is_an_error() {
        assert_eq!(
            forward("nope", "x: 1").unwrap_err(),
            ForwardError::UnknownPlatform("nope".to_string())
        );
    }

    #[test]
    fn every_listed_platform_has_an_embedded_ruleset() {
        for p in PLATFORMS {
            assert!(ruleset_json(p).is_some(), "missing ruleset for {p}");
            assert!(rule_pool(p).is_ok(), "ruleset for {p} failed to compile");
        }
    }

    #[test]
    fn edit_image_via_tgg_backward() {
        // drone step image; edit rust:1.75 → rust:1.80 by mutating the Hub-IR
        // and re-emitting through the backward cascade.
        let src = "kind: pipeline\nname: ci\nsteps:\n  - name: build\n    image: rust:1.75\n    commands:\n      - cargo build\n";
        let g = forward("drone", src).expect("forward");
        let img = g
            .iter_nodes()
            .find(|n| n.type_id == "hub:image")
            .expect("an image node");
        let edited = edit("drone", src, &img.id.hex(), "rust:1.80").expect("edit");
        assert!(edited.contains("rust:1.80"), "edit must appear: {edited}");
        assert!(!edited.contains("rust:1.75"), "old value gone: {edited}");
        // Re-emit is canonical, but the rest of the pipeline survives intact.
        assert!(edited.contains("cargo build") && edited.contains("build"));
    }

    #[test]
    fn re_emit_round_trips_a_pipeline() {
        // No edit: forward then re_emit must reproduce an equivalent pipeline.
        let g = forward("gitlab", GITLAB).expect("forward");
        let out = re_emit("gitlab", &g).expect("re_emit");
        assert!(out.contains("build") && out.contains("test") && out.contains("cargo"));
    }

    #[test]
    fn migrate_between_compatible_platforms() {
        // A real cross-platform migration: drone (step-flat) → woodpecker
        // (step-flat). The commands and the dependency survive; this is the
        // same rekey→backward→emit path the interop matrix proves at scale.
        let drone = "kind: pipeline\nname: ci\nsteps:\n  - name: build\n    image: rust:1.75\n    commands:\n      - cargo build\n  - name: test\n    depends_on:\n      - build\n    commands:\n      - cargo test\n";
        let out = migrate("drone", drone, "woodpecker").expect("migrate");
        assert!(
            out.contains("cargo build") && out.contains("cargo test"),
            "commands survive:\n{out}"
        );
        assert!(out.contains("steps:"), "woodpecker shape:\n{out}");
    }

    #[test]
    fn edit_rejects_bad_id() {
        assert!(matches!(
            edit("gitlab", GITLAB, "deadbeef", "x"),
            Err(ForwardError::Edit(_))
        ));
    }

    #[test]
    fn duplicate_step_adds_a_copy() {
        // A drone step list: duplicating a step yields two of its command.
        let src = "kind: pipeline\nname: ci\nsteps:\n  - name: build\n    commands:\n      - cargo build\n";
        let g = forward("drone", src).unwrap();
        let step = g.iter_nodes().find(|n| n.type_id == "hub:step").unwrap().id;
        let out = duplicate("drone", src, &step.hex()).expect("duplicate");
        assert!(
            out.matches("cargo build").count() >= 2,
            "expected the duplicated step:\n{out}"
        );
    }

    #[test]
    fn delete_job_removes_it() {
        // gitlab build+test; delete the build job → only test remains.
        let g = forward("gitlab", GITLAB).unwrap();
        let build = g
            .iter_nodes()
            .find(|n| n.type_id == "hub:job" && name_of(&g, n.id).as_deref() == Some("build"))
            .map(|n| n.id)
            .expect("build job");
        let out = delete("gitlab", GITLAB, &build.hex()).expect("delete");
        assert!(out.contains("test"), "test job kept:\n{out}");
        assert!(!out.contains("cargo build"), "build job gone:\n{out}");
    }

    #[test]
    fn delete_rejects_bad_id() {
        assert!(matches!(
            delete("gitlab", GITLAB, "deadbeef"),
            Err(ForwardError::Edit(_))
        ));
    }

    #[test]
    fn forward_drone_grows_steps() {
        // A step-flat platform: the cascade grows hub:step under the pipeline.
        let drone = "kind: pipeline\nname: ci\nsteps:\n  - name: build\n    commands:\n      - cargo build\n";
        let g = forward("drone", drone).expect("forward drone");
        assert!(g.iter_nodes().any(|n| n.type_id == "hub:step"));
        assert!(g.iter_nodes().any(|n| n.type_id == "hub:pipeline"));
    }
}
