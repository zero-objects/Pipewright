//! Render the Hub-IR into the two human-facing artefacts the UI/CLI need:
//!   * a clean DAG **diagram** (SVG + a layout descriptor for click-targets),
//!   * a **human-readable** runbook (HTML, or Markdown for the CLI).
//!
//! Both are projected from one `model::Pipeline` lifted off the graph, so they
//! can never disagree. Diagram nodes carry the source `GhostId` (a `data-hub`
//! attribute) — the hook for a future TGG-reverse editor.

#![allow(
    clippy::many_single_char_names,
    reason = "graph-walk (g/s/t/e) and geometry (x/y/w/h) read most clearly with short names"
)]
#![allow(
    clippy::cast_precision_loss,
    reason = "layout coordinates derive from small node/rank counts; f32 precision is never at risk"
)]

pub mod human;
pub mod layout;
pub mod model;
pub mod prose;
pub mod svg;

pub use human::{
    export, export_in, html, html_in, markdown, markdown_in, rtf, rtf_in, runbook, runbook_in,
    runbook_json, runbook_json_in, Runbook,
};
pub use layout::Layout;
pub use model::{lift, Job, Param, Pipeline, Step};
pub use prose::DEFAULT_LOCALE;
pub use svg::{render_diagram, Diagram, DiagramLayout, JobBox};

use seesaw_core::graph::TypedGraph;
use serde::Serialize;

/// A structural overview of a pipeline for the UI's detail panel, shaped as
/// `{pipeline: {name, jobs: [{name, stage, needs, steps, condition}]}}`.
#[derive(Debug, Clone, Serialize)]
pub struct Inspect {
    pub pipeline: InspectPipeline,
}

/// The pipeline level of an [`Inspect`].
#[derive(Debug, Clone, Serialize)]
pub struct InspectPipeline {
    pub name: Option<String>,
    pub jobs: Vec<InspectJob>,
}

/// One job in an [`Inspect`] — the fields the detail panel renders. Every
/// editable field carries its source `hub` `GhostId` hex, so the panel can pass
/// it straight to the edit FFI.
#[derive(Debug, Clone, Serialize)]
pub struct InspectJob {
    pub hub: String,
    pub name: String,
    pub stage: Option<String>,
    pub needs: Vec<String>,
    pub params: Vec<InspectField>,
    pub steps: Vec<InspectField>,
    pub condition: Option<String>,
    /// The `when:` disposition (`manual`/`never`/…), if any — a local run
    /// skips `manual`/`never`/`delayed` jobs.
    pub when: Option<String>,
    /// Source byte offset for jump-to-source (0 when no provenance).
    pub byte_start: usize,
}

/// An editable field: a label/value plus the `hub` `GhostId` hex to edit it by.
#[derive(Debug, Clone, Serialize)]
pub struct InspectField {
    pub hub: String,
    pub key: String,
    pub value: String,
}

/// Build an [`Inspect`] from a lifted pipeline.
#[must_use]
pub fn inspect(p: &Pipeline) -> Inspect {
    Inspect {
        pipeline: InspectPipeline {
            name: p.name.clone(),
            jobs: p
                .jobs
                .iter()
                .map(|j| InspectJob {
                    hub: j.id.hex(),
                    name: j.name.clone(),
                    stage: j.stage.clone(),
                    needs: j.needs.clone(),
                    params: j
                        .params
                        .iter()
                        .map(|p| InspectField {
                            hub: p.id.hex(),
                            key: p.key.clone(),
                            value: p.value.clone(),
                        })
                        .collect(),
                    steps: j
                        .steps
                        .iter()
                        .map(|s| InspectField {
                            hub: s.id.hex(),
                            key: "step".to_string(),
                            value: s.label.clone(),
                        })
                        .collect(),
                    condition: j.condition.clone(),
                    when: j.when.clone(),
                    byte_start: j.byte_start.unwrap_or(0),
                })
                .collect(),
        },
    }
}

/// JSON form of [`inspect`] for the FFI / UI boundary.
#[must_use]
pub fn inspect_json(p: &Pipeline) -> String {
    serde_json::to_string(&inspect(p)).unwrap_or_else(|_| r#"{"pipeline":{"jobs":[]}}"#.to_string())
}

/// The capabilities worth profiling. Each is detected from the Hub-IR via two
/// kinds of signal — dedicated node kinds (`hub:cache`, …) AND/OR unified-field
/// `hub:attr` names (`image`, `services`, …), since platforms model the same
/// feature either way. Tuple: `(key, label, node-type-ids, attr-names, universal)`.
/// `universal` = essentially every platform has it, so it's never a migration
/// caveat. Structural attrs (`name`/`script`/`steps`) are intentionally absent.
type Cap = (
    &'static str,
    &'static str,
    &'static [&'static str],
    &'static [&'static str],
    bool,
);
const CAPABILITIES: &[Cap] = &[
    (
        "image",
        "Container image",
        &["hub:image"],
        &["image", "container"],
        true,
    ),
    (
        "service",
        "Service containers",
        &["hub:service"],
        &["services"],
        false,
    ),
    ("cache", "Caching", &["hub:cache"], &["cache"], false),
    (
        "matrix",
        "Matrix / parallel expansion",
        &["hub:matrix"],
        &["matrix", "parallel", "strategy"],
        false,
    ),
    (
        "artifact",
        "Artifacts",
        &["hub:artifact"],
        &["artifacts", "artifact"],
        false,
    ),
    (
        "secret",
        "Secrets",
        &["hub:secret"],
        &["secrets", "secret"],
        false,
    ),
    (
        "condition",
        "Conditional execution",
        &["hub:condition", "hub:rule_clause"],
        &["rules", "when", "if_expr", "condition", "only", "except"],
        false,
    ),
    (
        "trigger",
        "Triggers / events",
        &["hub:trigger"],
        &["on", "trigger", "triggers"],
        false,
    ),
    (
        "variables",
        "Variables / env",
        &[],
        &["variables", "env", "environment_vars"],
        false,
    ),
    (
        "dependency",
        "Explicit job dependencies",
        &["hub:dependency_edge"],
        &["needs", "depends_on", "depends", "dependencies"],
        false,
    ),
    ("retry", "Retry policy", &["hub:retry"], &["retry"], false),
    (
        "schedule",
        "Scheduled runs",
        &["hub:schedule"],
        &["schedule", "cron"],
        false,
    ),
    (
        "deployment",
        "Deployment / environments",
        &["hub:deployment"],
        &["environment", "deployment", "deploy"],
        false,
    ),
    (
        "concurrency",
        "Concurrency control",
        &["hub:concurrency"],
        &["concurrency"],
        false,
    ),
    (
        "hook",
        "Lifecycle hooks",
        &["hub:hook"],
        &["hooks", "before_script", "after_script"],
        false,
    ),
    (
        "notification",
        "Notifications",
        &["hub:notification"],
        &["notifications", "notify"],
        false,
    ),
    (
        "permissions",
        "Permissions",
        &["hub:permissions"],
        &["permissions"],
        false,
    ),
    (
        "agent",
        "Agent / runner selection",
        &["hub:agent"],
        &["tags", "runs_on", "agent", "pool"],
        false,
    ),
    (
        "resource",
        "Resource classes",
        &["hub:resource"],
        &["resource_class", "resources"],
        false,
    ),
];

/// One capability family's presence in a pipeline graph.
#[derive(Debug, Clone)]
pub struct FeatureCount {
    pub key: &'static str,
    pub label: &'static str,
    pub count: usize,
    pub universal: bool,
}

/// Count each capability family in `g` (only families actually present). The
/// shared primitive behind both the capability profile and the migration
/// friction report.
#[must_use]
pub fn feature_counts(g: &TypedGraph) -> Vec<FeatureCount> {
    use std::collections::HashMap;
    let mut kinds: HashMap<&str, usize> = HashMap::new();
    let mut attr_names: HashMap<String, usize> = HashMap::new();
    for n in g.iter_nodes() {
        *kinds.entry(n.type_id.as_str()).or_default() += 1;
        if n.type_id == "hub:attr" {
            if let Some(name) = n.attrs.get("name") {
                *attr_names.entry(name.clone()).or_default() += 1;
            }
        }
    }
    let mut out = Vec::new();
    for (key, label, node_ids, names, universal) in CAPABILITIES {
        let from_nodes: usize = node_ids
            .iter()
            .map(|t| kinds.get(*t).copied().unwrap_or(0))
            .sum();
        let from_attrs: usize = names
            .iter()
            .map(|nm| attr_names.get(*nm).copied().unwrap_or(0))
            .sum();
        let count = from_nodes + from_attrs;
        if count > 0 {
            out.push(FeatureCount {
                key,
                label,
                count,
                universal: *universal,
            });
        }
    }
    out
}

/// Job and step totals of a graph (`hub:job` / `hub:step` node counts).
#[must_use]
pub fn job_step_totals(g: &TypedGraph) -> (usize, usize) {
    let mut jobs = 0;
    let mut steps = 0;
    for n in g.iter_nodes() {
        match n.type_id.as_str() {
            "hub:job" => jobs += 1,
            "hub:step" => steps += 1,
            _ => {}
        }
    }
    (jobs, steps)
}

/// Capability profile of a pipeline graph, derived from the Hub-IR: which
/// feature constructs it uses and how many of each. Returns
/// `{overall, summary, jobs, steps, features:[{key,label,count,universal}]}` as
/// JSON. `overall` is `Possible` when only universal constructs are used, else
/// `PossibleWithCaveats` — an honest source-side portability profile (a
/// target-specific verdict belongs to a migration check). Shared by the FFI and
/// the CLI so both report the same thing.
#[must_use]
pub fn capabilities_json(g: &TypedGraph) -> String {
    let (jobs, steps) = job_step_totals(g);
    let counts = feature_counts(g);
    let mut features = Vec::new();
    let mut caveats = 0usize;
    for f in &counts {
        if !f.universal {
            caveats += 1;
        }
        features.push(serde_json::json!({ "key": f.key, "label": f.label, "count": f.count, "universal": f.universal }));
    }
    let overall = if caveats == 0 {
        "Possible"
    } else {
        "PossibleWithCaveats"
    };
    let summary = format!(
        "{jobs} job(s), {steps} step(s); {caveats} non-universal capability famil{} in use.",
        if caveats == 1 { "y" } else { "ies" }
    );
    serde_json::json!({
        "overall": overall, "summary": summary, "jobs": jobs, "steps": steps, "features": features,
    })
    .to_string()
}

/// Everything a consumer needs for one pipeline graph: the DAG diagram (SVG +
/// layout descriptor), the HTML runbook, and the Markdown export. Returns
/// `None` if the graph has no pipeline root.
#[must_use]
pub fn render_all(g: &TypedGraph) -> Option<RenderBundle> {
    render_all_in(g, prose::DEFAULT_LOCALE)
}

/// [`render_all`] with the runbook rendered in `locale` (the diagram is
/// language-neutral). See `catalog/prose/<locale>.toml`.
#[must_use]
pub fn render_all_in(g: &TypedGraph, locale: &str) -> Option<RenderBundle> {
    let p = lift(g)?;
    Some(RenderBundle {
        diagram: render_diagram(&p),
        html: human::html_in(&p, locale),
        markdown: human::markdown_in(&p, locale),
        model: p,
    })
}

/// The full set of render artefacts for one pipeline.
#[derive(Debug, Clone)]
pub struct RenderBundle {
    pub model: Pipeline,
    pub diagram: Diagram,
    pub html: String,
    pub markdown: String,
}

/// A terse text outline of the lifted model — for tests / `--describe`.
#[must_use]
pub fn describe(p: &Pipeline) -> String {
    use std::fmt::Write as _;
    let mut s = String::new();
    let _ = writeln!(s, "pipeline {}", p.name.as_deref().unwrap_or("(unnamed)"));
    if !p.triggers.is_empty() {
        let _ = writeln!(s, "  triggers: {}", p.triggers.join(", "));
    }
    if !p.stages.is_empty() {
        let _ = writeln!(s, "  stages: {}", p.stages.join(" → "));
    }
    let _ = writeln!(s, "  jobs: {}", p.jobs.len());
    for j in &p.jobs {
        let stage = j
            .stage
            .as_deref()
            .map(|s| format!(" [{s}]"))
            .unwrap_or_default();
        let needs = if j.needs.is_empty() {
            String::new()
        } else {
            format!(" needs={:?}", j.needs)
        };
        let cond = j
            .condition
            .as_deref()
            .map(|c| format!(" when={c}"))
            .unwrap_or_default();
        let _ = writeln!(
            s,
            "    {}{stage}{needs}{cond} ({} steps)",
            j.name,
            j.steps.len()
        );
    }
    s
}
