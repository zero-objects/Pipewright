//! A flat, render-friendly view of a pipeline, lifted from the Hub-IR
//! `TypedGraph` via the `pipeline-hub-ir` read API. Both the diagram and the
//! human-readable runbook are projected from this one model so they never
//! disagree. Every node keeps its source `GhostId` so a click-target (and, in a
//! future TGG-reverse editor, an edit) maps straight back to the IR node.

use pipeline_hub_ir::graph as hir;
use seesaw_core::graph::{GhostId, TypedGraph};

/// One step inside a job — a single command / action.
#[derive(Debug, Clone)]
pub struct Step {
    pub id: GhostId,
    /// Human label: the run/script command, else the step name, else a kind.
    pub label: String,
}

/// One displayed parameter on a job — a `(key, value)` pair such as
/// `("image", "rust:1.75")` or `("env", "RUST_LOG=info")`. The `id` is the hub
/// node carrying it, so the UI can target an edit straight back to the IR.
#[derive(Debug, Clone)]
pub struct Param {
    pub id: GhostId,
    pub key: String,
    pub value: String,
}

/// One job (the diagram's nodes; the runbook's sections).
#[derive(Debug, Clone)]
pub struct Job {
    pub id: GhostId,
    pub name: String,
    /// Stage this job belongs to, if the platform groups jobs into stages.
    pub stage: Option<String>,
    /// Names of jobs this one depends on (the DAG edges, `job.needs`).
    pub needs: Vec<String>,
    /// A short when-condition summary, if any (the `rules:if` expression or a
    /// generic condition kind) — for display.
    pub condition: Option<String>,
    /// The `when:` disposition if the job carries one (`manual`, `never`,
    /// `always`, `on_success`, …) — drives whether a local run executes it.
    pub when: Option<String>,
    /// Service side-containers (DB/broker/…) the job needs, as image strings
    /// (`postgres:16`). A local run starts each as a network-linked sidecar.
    pub services: Vec<String>,
    /// Displayed parameters (image, env, timeout, artifacts, …) — the UML
    /// node's middle compartment.
    pub params: Vec<Param>,
    pub steps: Vec<Step>,
    /// Byte offset of this job in the source (from the hub node's provenance),
    /// for jump-to-source. `None` when the node carries no provenance.
    pub byte_start: Option<usize>,
}

/// The whole pipeline, flattened for rendering.
#[derive(Debug, Clone)]
pub struct Pipeline {
    pub name: Option<String>,
    /// Declared stage order (left→right / top→bottom). Empty if the platform
    /// has no explicit stages — the layout then ranks jobs topologically.
    pub stages: Vec<String>,
    pub jobs: Vec<Job>,
    /// Trigger one-line summaries (what starts the pipeline).
    pub triggers: Vec<String>,
}

/// The source byte offset a node was lifted from (its `prov_byte_start`).
fn node_byte_start(g: &TypedGraph, id: GhostId) -> Option<usize> {
    g.get_node(&id)
        .and_then(|n| n.attrs.get("prov_byte_start"))
        .and_then(|s| s.parse().ok())
}

/// A node's display name, tolerating both conventions: a `name` satellite
/// (`has_attr → attr{name=name} → value`) or an inline `name`/`value` attr on
/// the node itself (e.g. `hub:image{name="rust:1.75"}`).
fn node_name(g: &TypedGraph, id: GhostId) -> Option<String> {
    hir::name(g, id).map(ToString::to_string).or_else(|| {
        g.get_node(&id)
            .and_then(|n| n.attrs.get("name").or_else(|| n.attrs.get("value")))
            .filter(|s| !s.is_empty())
            .cloned()
    })
}

/// Read the first non-empty scalar field on a node, trying each name in order.
fn first_attr(g: &TypedGraph, node: GhostId, fields: &[&str]) -> Option<String> {
    fields.iter().find_map(|f| {
        hir::attr(g, node, f)
            .filter(|v| !v.is_empty())
            .map(ToString::to_string)
    })
}

/// Lift a render `Pipeline` from the Hub-IR graph. Returns `None` if the graph
/// has no `hub:pipeline` root.
#[must_use]
pub fn lift(g: &TypedGraph) -> Option<Pipeline> {
    let root = hir::pipeline_root(g)?;

    let stages: Vec<String> = hir::stages(g, root)
        .into_iter()
        .filter_map(|s| hir::name(g, s).map(ToString::to_string))
        .collect();

    let triggers: Vec<String> = hir::refs(g, root, "triggers")
        .into_iter()
        .chain(hir::refs(g, root, "pull_request"))
        .chain(hir::refs(g, root, "schedules"))
        .filter_map(|t| trigger_label(g, t))
        .collect();

    // Jobs live either directly under the pipeline (`jobs` seq, github; or a
    // direct `hub:has_job` edge, gitlab) or nested under stages (`stages` →
    // `stage.jobs`, azure). Collect both; a stage-nested job inherits its stage.
    let mut jobs: Vec<Job> = hir::jobs(g, root)
        .into_iter()
        .map(|j| build_job(g, j, None))
        .collect();
    for sid in hir::stages(g, root) {
        let stage_name = hir::name(g, sid).map(ToString::to_string);
        for jid in hir::jobs(g, sid) {
            jobs.push(build_job(g, jid, stage_name.clone()));
        }
    }

    // Step-flat platforms (drone/woodpecker/buildkite/gcb/…) have no job layer:
    // the work units are top-level `hub:step`s, each with its own `depends_on`
    // DAG and command lines. Promote each to a render node so the diagram and
    // runbook treat both shapes uniformly.
    if jobs.is_empty() {
        jobs = hir::steps(g, root)
            .into_iter()
            .map(|sid| build_step_node(g, sid))
            .collect();
    }

    // Event-selector groups (bitbucket `pipelines.branches/tags/…`): the
    // group key surfaces as the job's condition ("branch: main"). Harvested
    // from the root UNCONDITIONALLY — whichever hub:pipeline the root
    // resolution lands on (outer vs. the `pipelines:` sub-pipeline differs
    // between a source-seeded and a re-emitted graph), selector jobs must
    // not depend on the empty-jobs descent below.
    jobs.extend(selector_group_jobs(g, root));

    // Nested sub-pipelines (bitbucket's self-referential `pipelines` field):
    // descend one level and harvest their jobs / top-level steps + groups.
    if jobs.is_empty() {
        for sub in hir::refs(g, root, "pipelines") {
            for jid in hir::jobs(g, sub) {
                jobs.push(build_job(g, jid, None));
            }
            jobs.extend(selector_group_jobs(g, sub));
            for sid in hir::steps(g, sub) {
                jobs.push(build_step_node(g, sid));
            }
        }
    }

    // Some platforms model only named stages with no queryable inner jobs
    // (aws_codepipeline stages, where actions survive solely in provenance).
    // Surface each named stage as a bare node — real IR data, no fabrication;
    // the runbook then honestly shows "stages, contents not in IR".
    if jobs.is_empty() {
        for sid in hir::stages(g, root) {
            if let Some(name) = hir::name(g, sid) {
                jobs.push(Job {
                    id: sid,
                    name: name.to_string(),
                    stage: None,
                    needs: vec![],
                    condition: None,
                    when: None,
                    services: vec![],
                    params: vec![],
                    steps: vec![],
                    byte_start: node_byte_start(g, sid),
                });
            }
        }
    }

    // Final generic fallback: platforms whose work units hang under a
    // branch-keyed map the schema doesn't name as `steps`/`jobs` (bitbucket's
    // `pipelines.default`). Harvest every reachable `hub:step` rather than
    // hard-coding each platform's container field.
    if jobs.is_empty() {
        jobs = reachable_steps(g, root)
            .into_iter()
            .map(|sid| build_step_node(g, sid))
            .collect();
    }

    Some(Pipeline {
        name: hir::name(g, root).map(ToString::to_string),
        stages,
        jobs,
        triggers,
    })
}

/// Jobs inside event-selector groups on `node` (bitbucket branches/tags/
/// bookmarks/pull-requests/custom), each carrying its group as condition
/// ("branch: main"). Empty for platforms without selector fields.
fn selector_group_jobs(g: &TypedGraph, node: GhostId) -> Vec<Job> {
    let mut out = Vec::new();
    for field in [
        "branch_jobs",
        "tag_jobs",
        "bookmark_jobs",
        "pr_jobs",
        "custom_jobs",
    ] {
        for (grp, members) in hir::job_groups(g, node, field) {
            // vkind/name are INLINE attrs on the group hub:item.
            let label = g
                .get_node(&grp)
                .and_then(|n| n.attrs.get("vkind").cloned())
                .zip(node_name(g, grp))
                .map(|(k, n)| format!("{k}: {n}"));
            for jid in members {
                let mut j = build_job(g, jid, None);
                j.condition = j.condition.or_else(|| label.clone());
                out.push(j);
            }
        }
    }
    out
}

/// Every `hub:step` reachable from `root` through the satellite edges, in
/// breadth-first order. A best-effort harvest for platforms whose work units
/// hang under a container the schema doesn't expose as a named field.
fn reachable_steps(g: &TypedGraph, root: GhostId) -> Vec<GhostId> {
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let mut queue = std::collections::VecDeque::from([root]);
    while let Some(id) = queue.pop_front() {
        if !seen.insert(id) {
            continue;
        }
        if g.get_node(&id).is_some_and(|n| n.type_id == "hub:step") {
            out.push(id);
        }
        for (s, t, _) in g.iter_edges() {
            if s == id {
                queue.push_back(t);
            }
        }
    }
    out
}

/// Some step-flat platforms wrap the real step in an outer carrier whose only
/// child is a `hub:item_element → hub:step` holding the actual attributes
/// (buildkite's `commandStep` union models it this way). Descend through a lone
/// `hub:item_element` to the inner `hub:step` so the content is found; return
/// the node unchanged when there's no such wrapper.
fn unwrap_step_carrier(g: &TypedGraph, sid: GhostId) -> GhostId {
    let inner: Vec<GhostId> = g
        .iter_edges()
        .into_iter()
        .filter(|(s, t, e)| {
            *s == sid
                && e.type_id == "hub:item_element"
                && g.get_node(t).is_some_and(|n| n.type_id == "hub:step")
        })
        .map(|(_, t, _)| t)
        .collect();
    match inner.as_slice() {
        [single] => *single,
        _ => sid,
    }
}

/// A `name` value that looks like a container image reference (`rust:1.75`,
/// `gcr.io/cloud-builders/docker`) rather than a human job name — used to spot
/// the Cloud Build step shape, where `name:` IS the builder image.
fn looks_like_image(s: &str) -> bool {
    !s.contains(' ') && (s.contains(':') || s.contains('/'))
}

/// Build a render node from a top-level step on a step-flat platform. The
/// step's name is the node label, its `depends_on` is the DAG edge set, and its
/// command lines become the node's inner action steps.
fn build_step_node(g: &TypedGraph, outer: GhostId) -> Job {
    // Read content from the inner step when the platform wraps it; keep the
    // OUTER node's provenance for jump-to-source.
    let sid = unwrap_step_carrier(g, outer);
    let needs = read_needs(g, sid);

    // Google Cloud Build shape: `name:` is the builder IMAGE and the step's
    // human name is `id:`; `args:` is a single command split into argv tokens.
    // Detect it unambiguously (an image-like `name` AND an `id`), then read the
    // argv tokens in SOURCE ORDER (prov_byte_start) and join them into one
    // command — otherwise the generic path treats each token as its own step.
    let name_attr = first_attr(g, sid, &["name"]);
    let id_attr = first_attr(g, sid, &["id"]);
    let gcb_shape = matches!((&name_attr, &id_attr), (Some(n), Some(_)) if looks_like_image(n));

    let mut params;
    let name;
    let mut steps: Vec<Step>;
    if gcb_shape {
        name = id_attr.unwrap_or_else(|| "(step)".to_string());
        let mut argv: Vec<(usize, GhostId, String)> = ["run", "commands", "script", "args"]
            .iter()
            .flat_map(|f| hir::refs(g, sid, f))
            .filter_map(|c| {
                hir::leaf_text(g, c).map(|t| (node_byte_start(g, c).unwrap_or(0), c, t.to_string()))
            })
            .collect();
        argv.sort_by_key(|(b, _, _)| *b);
        steps = if argv.is_empty() {
            vec![]
        } else {
            let id = argv[0].1;
            vec![Step {
                id,
                label: argv
                    .iter()
                    .map(|(_, _, t)| t.as_str())
                    .collect::<Vec<_>>()
                    .join(" "),
            }]
        };
        params = collect_params(g, sid);
        // Surface the builder image as a param (so the runner picks it up) and
        // drop the now-redundant `id`/`name` scalar params.
        if let Some(img) = name_attr.filter(|n| looks_like_image(n)) {
            params.retain(|p| p.key != "id" && p.key != "name");
            params.insert(
                0,
                Param {
                    id: sid,
                    key: "image".to_string(),
                    value: img,
                },
            );
        }
    } else {
        name = name_attr.unwrap_or_else(|| "(step)".to_string());
        // Command lines: a sequence under run/commands/script, each a scalar leaf.
        steps = ["run", "commands", "script"]
            .iter()
            .flat_map(|f| hir::refs(g, sid, f))
            .filter_map(|c| {
                hir::leaf_text(g, c).map(|t| Step {
                    id: c,
                    label: t.to_string(),
                })
            })
            .collect();
        // Fall back to the step's own one-line label if it carries no command list.
        if steps.is_empty() {
            if let Some(label) = first_attr(g, sid, &["run", "script", "uses", "image", "kind"]) {
                steps.push(Step { id: sid, label });
            }
        }
        params = collect_params(g, sid);
    }
    // Identity/provenance from the OUTER node so clicks line up with the
    // hir::steps node the jobs list is built from; content from the inner.
    Job {
        id: outer,
        name,
        stage: None,
        needs,
        condition: job_condition(g, sid),
        when: job_when(g, sid),
        services: job_services(g, sid),
        params,
        steps,
        byte_start: node_byte_start(g, outer).or_else(|| node_byte_start(g, sid)),
    }
}

/// A step's one-line label, read robustly across conventions: a scalar attr
/// (`run`/`script`/`name`/…), else a `run`/`commands`/`script` ref whose payload
/// is a value leaf (the command string — `aws_codebuild` `commands`, etc.).
fn step_label(g: &TypedGraph, sid: GhostId) -> String {
    if let Some(label) = first_attr(
        g,
        sid,
        &[
            "run", "script", "commands", "name", "uses", "checkout", "kind",
        ],
    ) {
        return label;
    }
    for field in ["run", "commands", "script"] {
        if let Some(text) = hir::refs(g, sid, field)
            .into_iter()
            .find_map(|c| hir::leaf_text(g, c).map(ToString::to_string))
        {
            return text;
        }
    }
    // The step IS a collection item whose command hangs below it as a value
    // leaf (bitbucket `script:` lines — attr{steps} → coll → item → value).
    if let Some(text) = hir::leaf_text(g, sid) {
        return text.to_string();
    }
    "(step)".to_string()
}

fn build_job(g: &TypedGraph, jid: GhostId, stage_from_container: Option<String>) -> Job {
    let name = hir::name(g, jid).map_or_else(|| "(unnamed)".to_string(), ToString::to_string);
    let stage = stage_from_container.or_else(|| first_attr(g, jid, &["stage"]));
    let condition = job_condition(g, jid);
    let when = job_when(g, jid);
    let services = job_services(g, jid);
    let needs = read_needs(g, jid);
    let params = collect_params(g, jid);
    // Steps run in source order; the hub collection doesn't guarantee that
    // iteration order, so sort by source byte offset. Steps without
    // provenance keep their relative position at the front (stable sort).
    let mut step_ids = hir::steps(g, jid);
    step_ids.sort_by_key(|sid| node_byte_start(g, *sid).unwrap_or(0));
    let steps = step_ids
        .into_iter()
        .map(|sid| Step {
            id: sid,
            label: step_label(g, sid),
        })
        .collect();
    Job {
        id: jid,
        name,
        stage,
        needs,
        condition,
        when,
        services,
        params,
        steps,
        byte_start: node_byte_start(g, jid),
    }
}

/// Collect the displayed parameters of a job or step node — image, env /
/// variables, and any other notable scalar field (`timeout`, `working_dir`, …)
/// — each tagged with its source hub node so the UI can edit it back. Faithful
/// to the IR: only fields actually present are surfaced, never fabricated.
fn collect_params(g: &TypedGraph, node: GhostId) -> Vec<Param> {
    // Fields shown elsewhere (title / steps / needs) or handled above as
    // ref-fields (env/artifacts), plus internal attrs — never as a param.
    // `image` is intentionally NOT here: ref-image platforms are handled below;
    // unified-field platforms (gitlab) carry `image` as a `hub:attr`, so the
    // generic pass must be allowed to surface it.
    const HANDLED: &[&str] = &[
        "name",
        "stage",
        "script",
        "run",
        "needs",
        "depends_on",
        "steps",
        "variables",
        "env",
        "artifacts",
        "kind",
        "vkind",
    ];
    let mut params = Vec::new();

    // image: on ref-image platforms a `hub:image` ref whose name is the image
    // string (satellite or inline `name`); on unified-field platforms it's a
    // plain `hub:attr{name=image}` picked up by the generic pass below.
    for img in hir::refs(g, node, "image") {
        if let Some(name) = node_name(g, img) {
            params.push(Param {
                id: img,
                key: "image".to_string(),
                value: name,
            });
        }
    }

    // env / variables: each element is a `key=value` pair (a `hub:variable` or a
    // bare `hub:attr{name=key}` → value). The key may be inline; the value hangs
    // off a `has_value` leaf.
    for field in ["variables", "env"] {
        for v in hir::refs(g, node, field) {
            let key = node_name(g, v);
            let val = first_attr(g, v, &["value"])
                .or_else(|| hir::leaf_text(g, v).map(ToString::to_string));
            if let (Some(k), Some(val)) = (key, val) {
                params.push(Param {
                    id: v,
                    key: "env".to_string(),
                    value: format!("{k}={val}"),
                });
            }
        }
    }

    // artifacts: surface the path list, if any.
    for art in hir::refs(g, node, "artifacts") {
        let paths: Vec<String> = hir::refs(g, art, "paths")
            .into_iter()
            .filter_map(|p| hir::leaf_text(g, p).map(ToString::to_string))
            .collect();
        if !paths.is_empty() {
            params.push(Param {
                id: art,
                key: "artifacts".to_string(),
                value: paths.join(", "),
            });
        }
    }

    // Every OTHER scalar field on the node, generically — so the runbook follows
    // the IR as it grows (new fields like `labels`, `expose`, `volume`,
    // `healthcheck`, `clone`, `definitions`, `version`, … surface automatically,
    // no hardcoded list to forget to update). Skip the HANDLED fields and
    // internal/provenance attrs. Sorted for deterministic output.
    let mut extra: Vec<(String, String)> = hir::all_attrs(g, node)
        .filter(|(k, v)| !HANDLED.contains(k) && !k.starts_with("prov") && !v.is_empty())
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    extra.sort();
    extra.dedup();
    for (k, v) in extra {
        params.push(Param {
            id: node,
            key: k,
            value: v,
        });
    }

    params
}

/// The DAG-dependency target names of a job or step. The `needs` / `depends_on`
/// sequence holds either bare scalar job-name leaves (gitlab/drone) or
/// `dependency_edge` constructs whose target lives under a platform-specific
/// satellite (tekton `runAfter`, etc.) — [`dep_target`] tolerates both.
fn read_needs(g: &TypedGraph, id: GhostId) -> Vec<String> {
    let mut needs: Vec<String> = hir::refs(g, id, "needs")
        .into_iter()
        .chain(hir::refs(g, id, "depends_on"))
        .filter_map(|d| dep_target(g, d))
        .collect();
    // A SCALAR `depends_on: build` (buildkite) is a `hub:attr` with a value
    // leaf, not a collection — `refs` misses it. Pick up the single-name form.
    if needs.is_empty() {
        for field in ["depends_on", "needs"] {
            if let Some(t) = first_attr(g, id, &[field]).filter(|t| !t.is_empty()) {
                needs.push(t);
                break;
            }
        }
    }
    needs
}

/// Resolve one dependency element to a target name. Order: a bare scalar leaf;
/// then known target fields; then the first meaningful scalar satellite (a
/// `dependency_edge`'s sole purpose is to name a target, so this is safe).
fn dep_target(g: &TypedGraph, dep: GhostId) -> Option<String> {
    if let Some(t) = hir::leaf_text(g, dep) {
        return Some(t.to_string());
    }
    if let Some(t) = first_attr(
        g,
        dep,
        &[
            "name",
            "target",
            "runAfter",
            "run_after",
            "depends_on",
            "needs",
            "job",
            "task",
            "step",
        ],
    ) {
        return Some(t);
    }
    hir::all_attrs(g, dep)
        .find(|(k, v)| !k.starts_with("prov") && *k != "kind" && !v.is_empty())
        .map(|(_, v)| v.to_string())
}

/// A best-effort one-line label for a job's when-condition.
/// Nodes of a given kind reachable from `start` within `depth` hops (BFS over
/// outgoing edges). Used to find a job's `rule_clause` / `condition` satellites
/// regardless of the exact attr/collection wrapping between them.
fn reachable_of_kind(g: &TypedGraph, start: GhostId, kind: &str, depth: u8) -> Vec<GhostId> {
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let mut queue = std::collections::VecDeque::from([(start, 0u8)]);
    while let Some((id, d)) = queue.pop_front() {
        if !seen.insert(id) {
            continue;
        }
        if id != start && g.get_node(&id).is_some_and(|n| n.type_id == kind) {
            out.push(id);
        }
        if d < depth {
            for (s, t, _) in g.iter_edges() {
                if s == id {
                    queue.push_back((t, d + 1));
                }
            }
        }
    }
    out
}

fn job_condition(g: &TypedGraph, job: GhostId) -> Option<String> {
    // gitlab modern rules: a rule_clause's if_expr (within a few hops of the
    // job, regardless of the attr/collection wrapping); else a condition kind.
    for r in reachable_of_kind(g, job, "hub:rule_clause", 4) {
        if let Some(e) = first_attr(g, r, &["if_expr", "when"]).or_else(|| node_name(g, r)) {
            return Some(e);
        }
    }
    for c in reachable_of_kind(g, job, "hub:condition", 4) {
        if let Some(e) =
            first_attr(g, c, &["expr", "if_expr", "branch", "kind"]).or_else(|| node_name(g, c))
        {
            return Some(e);
        }
    }
    None
}

/// The job's service side-containers as image strings. gitlab/travis/… spell a
/// service either as a bare image (`services: [postgres:16]` → the service's
/// name IS the image) or as a mapping with an explicit `image:`/`name:`.
fn job_services(g: &TypedGraph, job: GhostId) -> Vec<String> {
    hir::refs(g, job, "services")
        .into_iter()
        .filter_map(|s| {
            // explicit image ref / attr first; then the bare-string form, where
            // the image hangs off the element as a `hub:value{text}` leaf or a
            // name/value attr.
            hir::refs(g, s, "image")
                .into_iter()
                .find_map(|i| node_name(g, i))
                .or_else(|| first_attr(g, s, &["image"]))
                .or_else(|| hir::leaf_text(g, s).map(ToString::to_string))
                .or_else(|| node_name(g, s))
        })
        .filter(|s| !s.is_empty())
        .collect()
}

/// `when:` dispositions a local run understands.
const WHEN_KINDS: &[&str] = &[
    "manual",
    "never",
    "always",
    "on_success",
    "on_failure",
    "delayed",
];

/// The job's `when:` disposition (`manual` / `never` / `always` / …), if any —
/// from a direct `when` attr, a `hub:condition{name=<when>}` satellite, or a
/// `rule_clause`'s `when`. Drives whether a local run executes the job.
fn job_when(g: &TypedGraph, job: GhostId) -> Option<String> {
    if let Some(w) = first_attr(g, job, &["when"]).filter(|w| WHEN_KINDS.contains(&w.as_str())) {
        return Some(w);
    }
    for c in reachable_of_kind(g, job, "hub:condition", 4) {
        if let Some(n) = node_name(g, c).filter(|n| WHEN_KINDS.contains(&n.as_str())) {
            return Some(n);
        }
        if let Some(w) = first_attr(g, c, &["when"]).filter(|w| WHEN_KINDS.contains(&w.as_str())) {
            return Some(w);
        }
    }
    for r in reachable_of_kind(g, job, "hub:rule_clause", 4) {
        if let Some(w) = first_attr(g, r, &["when"]).filter(|w| WHEN_KINDS.contains(&w.as_str())) {
            return Some(w);
        }
    }
    None
}

/// One-line label for a trigger / pr / schedule construct.
fn trigger_label(g: &TypedGraph, t: GhostId) -> Option<String> {
    let kind = g
        .get_node(&t)
        .map(|n| n.type_id.trim_start_matches("hub:").to_string())?;
    let detail = first_attr(g, t, &["cron", "kind", "if_expr", "when"]);
    Some(match detail {
        Some(d) => format!("{kind}: {d}"),
        None => kind,
    })
}
