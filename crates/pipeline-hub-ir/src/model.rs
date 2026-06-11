#![allow(
    clippy::doc_markdown,
    clippy::doc_lazy_continuation,
    reason = "module docs reference type names and have multi-line bullets"
)]

//! Top-level Hub-IR model: `Pipeline`, `Job`, `Trigger`, `Environment`,
//! and all annotation types (`PermissionsSpec`, `ConcurrencySpec`,
//! `VariableScope`, …).
//!
//! All types are pure data — no transformation logic. Each derives
//! the conservative set `Debug + Clone + PartialEq + Serialize +
//! Deserialize`. `Eq`/`Hash` only where useful as map-keys (e.g. on
//! [`crate::JobIdentity`]).
//!
//! Hub-IR v0.1 spec: `docs/specs/2026-05-13-hub-ir-v0.md` §2.0–§3.5.

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use crate::conditions::Condition;
use crate::identity::JobIdentity;
use crate::provenance::Provenance;

// ────────────────────────────────────────────────────────────────────
// 2.0 Pipeline — root node
// ────────────────────────────────────────────────────────────────────

/// Root of a Hub-IR model. Spec §2.0.
///
/// Holds everything that doesn't belong to a single job: workflow
/// rules, permissions, parameters, concurrency, variable scopes, the
/// lockfile reference. Jobs/triggers/environments/recipe-boundaries
/// are owned by the Pipeline directly (no separate graph layer in the
/// IR; that's an implementation detail of the forward driver).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Pipeline {
    pub id: String,
    pub provenance: Provenance,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lockfile_ref: Option<String>,

    #[serde(default)]
    pub jobs: Vec<Job>,
    #[serde(default)]
    pub triggers: Vec<Trigger>,
    #[serde(default)]
    pub environments: Vec<Environment>,
    #[serde(default)]
    pub recipe_boundaries: Vec<RecipeBoundary>,
    #[serde(default)]
    pub edges: Vec<Edge>,

    /// M5: ordered list of stage names from the top-level `stages:`
    /// declaration. Used by the effective-evaluator to derive
    /// stage_implicit DAG-edges between jobs that have no `needs:`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub stages: Vec<String>,
    /// v1: `workflow:rules:` as structured RulesEntry list (was
    /// `Vec<Condition>` in v0.1).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub workflow_rules: Vec<RulesEntry>,
    /// v1: `include:` references (resolver expansion is M4-scope).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub includes: Vec<IncludeSpec>,
    /// v1: Pipeline-level default image (`default: { image: ... }`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_image: Option<ImageSpec>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub permissions: Option<PermissionsSpec>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub parameters: Vec<ParameterSpec>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub concurrency: Option<ConcurrencySpec>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub variable_scopes: Vec<VariableScope>,

    #[serde(default, skip_serializing_if = "IndexMap::is_empty")]
    pub opaque: IndexMap<String, serde_json::Value>,

    /// Free-form comments that appear at the very top of the source
    /// file, before any structural content. Forward TGGs populate
    /// this; backward emitters write them back verbatim. Each entry
    /// is a single comment line without the leading `#` / `//`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub preamble_comments: Vec<String>,
    /// Comments at the end of the source file that don't attach to
    /// any job (typical case: a closing banner). Same shape as
    /// `preamble_comments`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub trailing_comments: Vec<String>,
}

impl Pipeline {
    /// Construct an empty pipeline rooted at the given provenance.
    #[must_use]
    pub fn new(id: impl Into<String>, provenance: Provenance) -> Self {
        Self {
            id: id.into(),
            provenance,
            lockfile_ref: None,
            jobs: Vec::new(),
            triggers: Vec::new(),
            environments: Vec::new(),
            recipe_boundaries: Vec::new(),
            edges: Vec::new(),
            stages: Vec::new(),
            workflow_rules: Vec::new(),
            includes: Vec::new(),
            default_image: None,
            permissions: None,
            parameters: Vec::new(),
            concurrency: None,
            variable_scopes: Vec::new(),
            opaque: IndexMap::new(),
            preamble_comments: Vec::new(),
            trailing_comments: Vec::new(),
        }
    }
}

// ────────────────────────────────────────────────────────────────────
// 2.1 Job
// ────────────────────────────────────────────────────────────────────

/// A unit of work, scheduled and executed as a whole. Spec §2.1
/// + Hub-IR v1 extensions (`before_steps`, `after_steps`, `image`,
/// `rules`, `extends`, `synth_origin`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Job {
    pub id: String,
    pub identity: JobIdentity,
    pub name: String,
    #[serde(default)]
    pub r#type: JobType,
    pub provenance: Provenance,

    /// Steps before the main `script:`. v1 — see hub-ir-v1.md Gap #1.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub before_steps: Vec<Step>,
    #[serde(default)]
    pub steps: Vec<Step>,
    /// Steps after the main `script:` (GitLab `after_script:`,
    /// Jenkins `post`, etc.). v1.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub after_steps: Vec<Step>,

    /// Trail of materialiser-events applied to this job over edit
    /// history. Always empty for fresh forward-parse (no edits yet).
    /// See `docs/plans/2026-05-14-m3-interfaces.md` §3.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub correspondence_trail: Vec<CorrespondenceEvent>,

    /// Origin marker for `r#type == Synth` jobs (v1, supersedes the
    /// type-discriminator alone).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub synth_origin: Option<SynthOrigin>,

    // ── annotations (all optional / default-empty) ──
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub conditions: Vec<Condition>,
    /// Typed `rules:` list (v1 — see hub-ir-v1.md Gap #3). Each
    /// entry combines `if:` / `when:` / `changes:` / `exists:` /
    /// `variables:` semantically.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rules: Vec<RulesEntry>,
    /// Template-references resolved at M4-time (v1 — Gap #7).
    /// Populated by forward when `extends:` is present in the CST.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub extends: Vec<String>,
    /// Container/runtime image spec (v1 — Gap #2).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image: Option<ImageSpec>,
    #[serde(default, skip_serializing_if = "IndexMap::is_empty")]
    pub variables: IndexMap<String, VariableSpec>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub secrets_refs: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub caches: Vec<CacheSpec>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub artifacts_produced: Vec<ArtifactSpec>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub artifacts_consumed: Vec<ArtifactRef>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub services: Vec<ServiceSpec>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resource_class: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub runner_selector: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub matrix: Option<MatrixSpec>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry_policy: Option<RetrySpec>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout: Option<Duration>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allow_failure: Option<AllowFailure>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dag_source: Option<DagSource>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub permissions: Option<PermissionsSpec>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub concurrency: Option<ConcurrencySpec>,
    #[serde(default, skip_serializing_if = "IndexMap::is_empty")]
    pub opaque: IndexMap<String, serde_json::Value>,

    /// Comments immediately preceding this job in the source file
    /// (between the previous job and this one, or between the file
    /// preamble and the first job). Each entry is one comment line
    /// without the leading `#` / `//`. Forward TGGs populate it,
    /// backward emitters write it back.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub leading_comments: Vec<String>,
}

impl Job {
    /// Construct a Standard job with given identity, name, provenance.
    #[must_use]
    pub fn new(
        id: impl Into<String>,
        identity: JobIdentity,
        name: impl Into<String>,
        provenance: Provenance,
    ) -> Self {
        Self {
            id: id.into(),
            identity,
            name: name.into(),
            r#type: JobType::Standard,
            provenance,
            before_steps: Vec::new(),
            steps: Vec::new(),
            after_steps: Vec::new(),
            correspondence_trail: Vec::new(),
            synth_origin: None,
            conditions: Vec::new(),
            rules: Vec::new(),
            extends: Vec::new(),
            image: None,
            variables: IndexMap::new(),
            secrets_refs: Vec::new(),
            caches: Vec::new(),
            artifacts_produced: Vec::new(),
            artifacts_consumed: Vec::new(),
            services: Vec::new(),
            resource_class: None,
            runner_selector: Vec::new(),
            matrix: None,
            retry_policy: None,
            timeout: None,
            allow_failure: None,
            dag_source: None,
            permissions: None,
            concurrency: None,
            opaque: IndexMap::new(),
            leading_comments: Vec::new(),
        }
    }
}

/// Job variant — Standard (user-authored) vs Synth (generated from
/// platform constructs like `after_script:` or Jenkins `post {}`).
/// Spec §2.1.2.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum JobType {
    #[default]
    Standard,
    Synth,
}

/// One step in a job's `steps:` list. Steps have provenance but no
/// own identity (spec §2.5).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Step {
    /// For `Run` steps: the raw shell command. For `Uses` steps
    /// (GitHub-Actions style): the action reference (e.g.
    /// `actions/checkout@v4`). The `kind` discriminates the two.
    pub command: String,
    /// M8.1 (v1.1, additive): `Run` for shell scripts, `Uses` for
    /// GitHub-Actions action references. Defaults to `Run` so v1.0
    /// data deserialises unchanged.
    #[serde(default)]
    pub kind: StepKind,
    pub provenance: Provenance,
    /// YAML scalar-style preservation (v1 — Gap #6). Default `Plain`.
    #[serde(default)]
    pub style: ScalarStyle,
    /// Comments that appear directly above this step in the source
    /// (between the previous step and this one), with the leading
    /// `#` / `//` stripped. Backward emitters write them back as
    /// `# `-prefixed lines at the same indent as the step itself.
    /// v1.2 — see hub-ir-v1.2.md.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub leading_comments: Vec<String>,
    /// Per-step platform-specific keys not modelled structurally
    /// (e.g. GitHub Actions `name:`, `id:`, `if:`, `env:`, `with:`,
    /// `continue-on-error:`, `working-directory:`, `timeout-minutes:`,
    /// `shell:`). Forward populates this from any unknown step-level
    /// keys; backward emits each entry back at the same scope. Keys
    /// are namespaced (e.g. `github.name`, `github.with`) to keep
    /// the cross-platform model unambiguous. v1.2 (additive).
    #[serde(default, skip_serializing_if = "indexmap::IndexMap::is_empty")]
    pub opaque: indexmap::IndexMap<String, serde_json::Value>,
}

/// Discriminator for `Step` — what *kind* of step this is.
///
/// `Run` is the original v1.0 semantics: a shell command. `Uses` was
/// added in v1.1 (M8.1) so the GitHub axis can carry action
/// references natively instead of via a `uses:` command-prefix
/// convention. Other platforms (GitLab) only ever produce `Run`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum StepKind {
    /// Shell command. The default — what v1.0 data deserialises to.
    #[default]
    Run,
    /// GitHub Actions action reference (`uses:` in the workflow).
    Uses,
}

/// YAML scalar-style for round-trip preservation. Mirrors the
/// `pipeline_cst::ScalarStyle` variants the seeder may encounter,
/// but lives in Hub-IR so the emitter can faithfully reproduce
/// `|` / `>` block-literals etc.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum ScalarStyle {
    #[default]
    Plain,
    DoubleQuoted,
    SingleQuoted,
    /// `|` block-literal: preserves newlines verbatim.
    LiteralBlock,
    /// `>` block-folded: collapses single newlines to spaces.
    FoldedBlock,
}

/// Synth-job origin discriminator (v1 — Gap #8). Set on
/// `Job.synth_origin` when `Job.type == Synth`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SynthOrigin {
    /// Generated from a GitLab `after_script:` block.
    AfterScript { parent_job_id: String },
    /// Generated from a GitLab `before_script:` block (rare — usually
    /// kept as `Job.before_steps` instead).
    BeforeScript { parent_job_id: String },
    /// Generated from a Jenkins `post { success {} … }` block.
    PostBlock {
        parent_job_id: String,
        kind: PostKind,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PostKind {
    Success,
    Failure,
    Always,
    Unstable,
    Aborted,
    Cleanup,
}

/// Container/runtime image spec (v1 — Gap #2). Spec §2.1.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ImageSpec {
    pub name: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub entrypoint: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub command: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pull_policy: Option<String>,
    #[serde(default, skip_serializing_if = "IndexMap::is_empty")]
    pub opaque: IndexMap<String, serde_json::Value>,
}

impl ImageSpec {
    /// Construct from just the image name (common case: `image: <string>`).
    #[must_use]
    pub fn from_name(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            entrypoint: Vec::new(),
            command: Vec::new(),
            pull_policy: None,
            opaque: IndexMap::new(),
        }
    }
}

/// Typed `rules:` entry (v1 — Gap #3). Combines `if:` / `when:`
/// / `changes:` / `exists:` / `variables:` into a single semantic
/// unit, preserving list order.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct RulesEntry {
    #[serde(default, skip_serializing_if = "Option::is_none", rename = "if")]
    pub r#if: Option<crate::conditions::Condition>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub when: Option<WhenAction>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub changes: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub exists: Vec<String>,
    #[serde(default, skip_serializing_if = "IndexMap::is_empty")]
    pub variables: IndexMap<String, VariableSpec>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allow_failure: Option<AllowFailure>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum WhenAction {
    OnSuccess,
    OnFailure,
    Always,
    Manual,
    Delayed { duration: String },
    Never,
}

impl WhenAction {
    /// Parse from the GitLab YAML scalar form (`"manual"`, `"on_success"` …).
    #[must_use]
    pub fn from_scalar(s: &str) -> Option<Self> {
        match s.trim() {
            "on_success" => Some(Self::OnSuccess),
            "on_failure" => Some(Self::OnFailure),
            "always" => Some(Self::Always),
            "manual" => Some(Self::Manual),
            "never" => Some(Self::Never),
            other if other.starts_with("delayed") => Some(Self::Delayed {
                duration: other.into(),
            }),
            _ => None,
        }
    }

    #[must_use]
    pub fn as_scalar(&self) -> String {
        match self {
            Self::OnSuccess => "on_success".into(),
            Self::OnFailure => "on_failure".into(),
            Self::Always => "always".into(),
            Self::Manual => "manual".into(),
            Self::Never => "never".into(),
            Self::Delayed { duration } => format!("delayed (start_in={duration})"),
        }
    }
}

/// `include:` reference (v1 — Gap #7). Spec §2.0 — populated by
/// forward when `include:` is present in the CST; resolver
/// expansion is M4-scope.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum IncludeSpec {
    Local {
        path: String,
    },
    Project {
        project: String,
        file: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        r#ref: Option<String>,
    },
    Remote {
        url: String,
    },
    Template {
        template: String,
    },
}

/// Typed materialiser-event in a Job's `correspondence_trail`.
/// Spec §2.1.3.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum CorrespondenceEvent {
    RenameFrom {
        previous_name: String,
    },
    InsertedFor {
        source_job_id: String,
        at_step_index: usize,
    },
    SplitOf {
        source_job_id: String,
        part: SplitPart,
        split_after: usize,
    },
    MergedInto {
        target_job_id: String,
    },
    TggMaterialized {
        rule_id: String,
        /// Timestamp as RFC3339 string. We keep it as String (not a
        /// chrono type) to avoid a deps for typed time in the IR layer.
        at: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SplitPart {
    First,
    Second,
}

// ────────────────────────────────────────────────────────────────────
// 2.2 Trigger
// ────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Trigger {
    pub id: String,
    pub kind: TriggerKind,
    pub provenance: Provenance,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub branch_filter: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub path_filter: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tag_filter: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schedule_expr: Option<String>,
    #[serde(default, skip_serializing_if = "IndexMap::is_empty")]
    pub event_payload_filter: IndexMap<String, String>,
    #[serde(default, skip_serializing_if = "IndexMap::is_empty")]
    pub opaque: IndexMap<String, serde_json::Value>,
}

/// Per spec §2.2 + §5.4 (Jenkins-specific kinds promoted in v0.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TriggerKind {
    Push,
    MergeRequest,
    Tag,
    Schedule,
    Manual,
    External,
    RepositoryDispatch,
    Webhook,
    PollScm,
}

// ────────────────────────────────────────────────────────────────────
// 2.3 Environment
// ────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Environment {
    pub id: String,
    pub name: String,
    pub provenance: Provenance,

    #[serde(default)]
    pub approval_required: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub approval_reviewers: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub protection_rules: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url_template: Option<String>,
    #[serde(default, skip_serializing_if = "IndexMap::is_empty")]
    pub opaque: IndexMap<String, serde_json::Value>,
}

// ────────────────────────────────────────────────────────────────────
// 2.4 RecipeBoundary
// ────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RecipeBoundary {
    pub id: String,
    pub recipe_id: String,
    pub recipe_version: String,
    #[serde(default)]
    pub input_ports: Vec<RecipePort>,
    #[serde(default)]
    pub output_ports: Vec<RecipePort>,
    pub provenance: Provenance,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RecipePort {
    pub name: String,
    pub kind: String,
}

// ────────────────────────────────────────────────────────────────────
// 2.6 Lockfile
// ────────────────────────────────────────────────────────────────────

/// Persistent identity registry beside the pipeline source.
/// Spec §2.6. Lockfile lives on disk as `.pipeline.lock` (YAML).
///
/// Extends the spec with `content_signatures` — needed for the
/// composed identity strategy's Levenshtein-recovery path
/// (`docs/plans/2026-05-14-m3-interfaces.md` §8). Maps `uuid →
/// normalised script text`; absent for new lockfiles.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Lockfile {
    pub schema_version: String,
    #[serde(default)]
    pub paths: IndexMap<String, String>,
    #[serde(default)]
    pub renames: Vec<RenameRecord>,
    /// Default 0.7 (per M0-spike-empfehlung); see
    /// `docs/plans/2026-05-14-m3-interfaces.md` §8.
    #[serde(default = "default_content_recovery_threshold")]
    pub content_recovery_threshold: f32,
    /// Normalised script content per uuid — used by content-recovery
    /// to compute Levenshtein similarity on path-miss. M3-extension
    /// over the v0.1 spec (additive, harmless).
    #[serde(default, skip_serializing_if = "IndexMap::is_empty")]
    pub content_signatures: IndexMap<String, String>,
}

fn default_content_recovery_threshold() -> f32 {
    0.7
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RenameRecord {
    pub from: String,
    pub to: String,
    /// Edit-id or ISO-8601 timestamp.
    pub at: String,
}

impl Lockfile {
    #[must_use]
    pub fn empty() -> Self {
        Self {
            schema_version: "0.1.0".into(),
            paths: IndexMap::new(),
            renames: Vec::new(),
            content_recovery_threshold: default_content_recovery_threshold(),
            content_signatures: IndexMap::new(),
        }
    }
}

// ────────────────────────────────────────────────────────────────────
// 2.7 Permissions
// ────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PermissionsSpec {
    #[serde(default)]
    pub scopes: IndexMap<String, PermissionLevel>,
    #[serde(default)]
    pub branches: Vec<BranchProtection>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rbac_role: Option<String>,
    #[serde(default, skip_serializing_if = "IndexMap::is_empty")]
    pub opaque: IndexMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PermissionLevel {
    None,
    Read,
    Write,
    Admin,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BranchProtection {
    pub pattern: String,
    #[serde(default)]
    pub require_approval: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub required_roles: Vec<String>,
}

// ────────────────────────────────────────────────────────────────────
// 2.8 Concurrency
// ────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConcurrencySpec {
    pub group: String,
    #[serde(default)]
    pub cancel_in_progress: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resource: Option<String>,
}

// ────────────────────────────────────────────────────────────────────
// 3.5 VariableScope
// ────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VariableScope {
    pub id: String,
    #[serde(default)]
    pub variables: IndexMap<String, VariableSpec>,
    pub applies_to: ScopeFilter,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ScopeFilter {
    AllJobs,
    InStages(Vec<String>),
    InJobs(Vec<String>),
    MatchingTags(Vec<String>),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VariableSpec {
    pub value: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default)]
    pub expanded: bool,
}

impl VariableSpec {
    #[must_use]
    pub fn literal(value: impl Into<String>) -> Self {
        Self {
            value: value.into(),
            description: None,
            expanded: false,
        }
    }
}

// ────────────────────────────────────────────────────────────────────
// Edges (3.1, 3.2)
// ────────────────────────────────────────────────────────────────────

/// One edge in the pipeline graph. Spec §3.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Edge {
    pub from: String,
    pub to: String,
    pub kind: EdgeKind,
    pub provenance: Provenance,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum EdgeKind {
    /// Job→Job: hard dependency (B waits for A's success).
    DependsOnHard,
    /// Job→Job: soft dependency (B starts after A regardless of status).
    DependsOnSoft,
    /// Job→Job: synthetic from `stages:` order. Backward serialiser
    /// omits these (they're implicit in YAML's stages:-declaration).
    StageImplicit,
    /// Job→Pipeline: child-pipeline trigger.
    Triggers(TriggerStrategy),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TriggerStrategy {
    /// Parent waits for child completion.
    Depend,
    /// Parent fires and forgets.
    FireAndForget,
    /// Parent waits and inherits child's status.
    MirrorStatus,
}

// ────────────────────────────────────────────────────────────────────
// Annotation types
// ────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MatrixSpec {
    pub axes: IndexMap<String, Vec<String>>,
    #[serde(default)]
    pub include: Vec<IndexMap<String, String>>,
    #[serde(default)]
    pub exclude: Vec<IndexMap<String, String>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RetrySpec {
    pub max: u32,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub when: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub exit_codes: Vec<i32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CacheSpec {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub paths: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub policy: Option<String>,
    #[serde(default, skip_serializing_if = "IndexMap::is_empty")]
    pub opaque: IndexMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ArtifactSpec {
    pub name: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub paths: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub when: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expire_in: Option<Duration>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sub_type: Vec<String>,
    #[serde(default, skip_serializing_if = "IndexMap::is_empty")]
    pub opaque: IndexMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ArtifactRef {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from_job: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ServiceSpec {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alias: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub command: Vec<String>,
    #[serde(default, skip_serializing_if = "IndexMap::is_empty")]
    pub opaque: IndexMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ParameterSpec {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// `allow_failure: true` / `allow_failure: false` / `allow_failure:
/// exit_codes: [42]`. Spec §2.1.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum AllowFailure {
    Flag(bool),
    ExitCodes { exit_codes: Vec<i32> },
}

/// How the DAG-edges of this job were derived. Spec §2.1.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum DagSource {
    ExplicitNeeds,
    StageImplicit,
    Mixed,
}

/// Time duration preserved as its source string (e.g. `"5 min"`,
/// `"1h30m"`). Hub-IR keeps the original to round-trip exactly; the
/// resolver/evaluator (M4) parses it semantically.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Duration(pub String);

impl Duration {
    /// Construct from a source string (e.g. `"5 min"`, `"1h30m"`).
    #[must_use]
    pub fn new(s: impl Into<String>) -> Self {
        Self(s.into())
    }
}
