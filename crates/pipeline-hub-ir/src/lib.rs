//! Hub-IR v0.1 — platform-neutral CI pipeline intermediate representation.
//!
//! Schema spec: `docs/specs/2026-05-13-hub-ir-v0.md` (v0.1.0).
//! This crate is pure data — transformations live in
//! platform-specific TGG crates (e.g., `pipeline-gitlab-tgg`).

pub const SCHEMA_VERSION: (u32, u32, u32) = (1, 1, 0);

pub mod conditions;
pub mod graph;
pub mod identity;
pub mod model;
pub mod provenance;
pub mod schema;

pub use conditions::{CompareOp, Condition, Operand, VariableRef};
pub use identity::{JobIdentity, JobIdentityOrigin};
pub use model::{
    AllowFailure, ArtifactRef, ArtifactSpec, BranchProtection, CacheSpec, ConcurrencySpec,
    CorrespondenceEvent, DagSource, Duration, Edge, EdgeKind, Environment, ImageSpec, IncludeSpec,
    Job, JobType, Lockfile, MatrixSpec, ParameterSpec, PermissionLevel, PermissionsSpec, Pipeline,
    PostKind, RecipeBoundary, RecipePort, RenameRecord, RetrySpec, RulesEntry, ScalarStyle,
    ScopeFilter, ServiceSpec, SplitPart, Step, StepKind, SynthOrigin, Trigger, TriggerKind,
    TriggerStrategy, VariableScope, VariableSpec, WhenAction,
};
pub use provenance::{Provenance, SourceRange};
