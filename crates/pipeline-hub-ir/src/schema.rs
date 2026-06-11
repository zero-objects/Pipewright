#![allow(
    clippy::doc_markdown,
    reason = "generated module; prose mentions TGG / acronyms"
)]

//! Hub-IR schema — GENERATED from catalog/hub_schema.toml by
//! catalog/gen_hub_rs.py. Do not edit; regenerate.
//!
//! The platform-neutral IR is a typed graph. This module is
//! its model: node-kind / edge-kind id constants and the
//! `SCHEMA` table that the forward/backward TGG rule
//! generators and graph validation build against.

/// A field of a node-kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldKind {
    /// A scalar attribute carried on the node.
    Scalar,
    /// An edge to another node-kind (the target node-kind id).
    Ref(&'static str),
}

/// One field of a node-kind.
#[derive(Debug, Clone, Copy)]
pub struct Field {
    pub name: &'static str,
    pub kind: FieldKind,
}

/// One node-kind of the IR graph.
#[derive(Debug, Clone, Copy)]
pub struct NodeKind {
    pub id: &'static str,
    /// Lexical kinds (comment, anchor, …) carry round-trip
    /// fidelity, not pipeline semantics.
    pub lexical: bool,
    pub fields: &'static [Field],
}

/// Node-kind id constants.
pub mod node {
    pub const AGENT: &str = "hub:agent";
    pub const ARTIFACT: &str = "hub:artifact";
    pub const ATTR: &str = "hub:attr";
    pub const CACHE: &str = "hub:cache";
    pub const CONCURRENCY: &str = "hub:concurrency";
    pub const CONDITION: &str = "hub:condition";
    pub const DEPENDENCY_EDGE: &str = "hub:dependency_edge";
    pub const DEPLOYMENT: &str = "hub:deployment";
    pub const HOOK: &str = "hub:hook";
    pub const IMAGE: &str = "hub:image";
    pub const JOB: &str = "hub:job";
    pub const MATRIX: &str = "hub:matrix";
    pub const NOTIFICATION: &str = "hub:notification";
    pub const PARAMETER: &str = "hub:parameter";
    pub const PERMISSIONS: &str = "hub:permissions";
    pub const PIPELINE: &str = "hub:pipeline";
    pub const RESOURCE: &str = "hub:resource";
    pub const RETRY: &str = "hub:retry";
    pub const SECRET: &str = "hub:secret";
    pub const SERVICE: &str = "hub:service";
    pub const STAGE: &str = "hub:stage";
    pub const STEP: &str = "hub:step";
    pub const TEMPLATE: &str = "hub:template";
    pub const TRIGGER: &str = "hub:trigger";
    pub const VARIABLE: &str = "hub:variable";
    pub const WORKSPACE: &str = "hub:workspace";
    pub const ANCHOR: &str = "hub:anchor";
    pub const BLANK_RUN: &str = "hub:blank_run";
    pub const COMMENT: &str = "hub:comment";
    pub const KEY_ORDER: &str = "hub:key_order";
    pub const PROVENANCE: &str = "hub:provenance";
    pub const SCALAR_STYLE: &str = "hub:scalar_style";
}

/// Edge-kind id constants.
pub mod edge {
    pub const HAS_AGENT: &str = "hub:has_agent";
    pub const HAS_ARTIFACT: &str = "hub:has_artifact";
    pub const HAS_ATTR: &str = "hub:has_attr";
    pub const HAS_CACHE: &str = "hub:has_cache";
    pub const HAS_CONCURRENCY: &str = "hub:has_concurrency";
    pub const HAS_CONDITION: &str = "hub:has_condition";
    pub const HAS_DEPENDENCY_EDGE: &str = "hub:has_dependency_edge";
    pub const HAS_DEPLOYMENT: &str = "hub:has_deployment";
    pub const HAS_HOOK: &str = "hub:has_hook";
    pub const HAS_IMAGE: &str = "hub:has_image";
    pub const HAS_JOB: &str = "hub:has_job";
    pub const HAS_MATRIX: &str = "hub:has_matrix";
    pub const HAS_NOTIFICATION: &str = "hub:has_notification";
    pub const HAS_PARAMETER: &str = "hub:has_parameter";
    pub const HAS_PERMISSIONS: &str = "hub:has_permissions";
    pub const HAS_PIPELINE: &str = "hub:has_pipeline";
    pub const HAS_RESOURCE: &str = "hub:has_resource";
    pub const HAS_RETRY: &str = "hub:has_retry";
    pub const HAS_SECRET: &str = "hub:has_secret";
    pub const HAS_SERVICE: &str = "hub:has_service";
    pub const HAS_STAGE: &str = "hub:has_stage";
    pub const HAS_STEP: &str = "hub:has_step";
    pub const HAS_TEMPLATE: &str = "hub:has_template";
    pub const HAS_TRIGGER: &str = "hub:has_trigger";
    pub const HAS_VARIABLE: &str = "hub:has_variable";
}

/// The full IR schema — every node-kind and its fields.
pub const SCHEMA: &[NodeKind] = &[
    NodeKind {
        id: "hub:agent",
        lexical: false,
        fields: &[
            Field {
                name: "config",
                kind: FieldKind::Scalar,
            },
            Field {
                name: "container",
                kind: FieldKind::Scalar,
            },
            Field {
                name: "labels",
                kind: FieldKind::Scalar,
            },
            Field {
                name: "permissions",
                kind: FieldKind::Ref("hub:permissions"),
            },
            Field {
                name: "platform",
                kind: FieldKind::Scalar,
            },
            Field {
                name: "resources",
                kind: FieldKind::Ref("hub:resource"),
            },
            Field {
                name: "security",
                kind: FieldKind::Scalar,
            },
            Field {
                name: "selector",
                kind: FieldKind::Scalar,
            },
            Field {
                name: "tolerations",
                kind: FieldKind::Scalar,
            },
        ],
    },
    NodeKind {
        id: "hub:artifact",
        lexical: false,
        fields: &[
            Field {
                name: "access",
                kind: FieldKind::Scalar,
            },
            Field {
                name: "base_directory",
                kind: FieldKind::Scalar,
            },
            Field {
                name: "coordinates",
                kind: FieldKind::Scalar,
            },
            Field {
                name: "depth",
                kind: FieldKind::Scalar,
            },
            Field {
                name: "direction",
                kind: FieldKind::Scalar,
            },
            Field {
                name: "discard_paths",
                kind: FieldKind::Scalar,
            },
            Field {
                name: "encryption",
                kind: FieldKind::Scalar,
            },
            Field {
                name: "exclude",
                kind: FieldKind::Scalar,
            },
            Field {
                name: "expire_in",
                kind: FieldKind::Scalar,
            },
            Field {
                name: "expose_as",
                kind: FieldKind::Scalar,
            },
            Field {
                name: "kind",
                kind: FieldKind::Scalar,
            },
            Field {
                name: "location",
                kind: FieldKind::Scalar,
            },
            Field {
                name: "name",
                kind: FieldKind::Scalar,
            },
            Field {
                name: "paths",
                kind: FieldKind::Scalar,
            },
            Field {
                name: "prefix",
                kind: FieldKind::Scalar,
            },
            Field {
                name: "reports",
                kind: FieldKind::Scalar,
            },
            Field {
                name: "repository",
                kind: FieldKind::Scalar,
            },
            Field {
                name: "secondary",
                kind: FieldKind::Scalar,
            },
            Field {
                name: "source",
                kind: FieldKind::Scalar,
            },
            Field {
                name: "store_type",
                kind: FieldKind::Scalar,
            },
            Field {
                name: "subtypes",
                kind: FieldKind::Scalar,
            },
            Field {
                name: "symlinks",
                kind: FieldKind::Scalar,
            },
            Field {
                name: "untracked",
                kind: FieldKind::Scalar,
            },
            Field {
                name: "version",
                kind: FieldKind::Scalar,
            },
            Field {
                name: "when",
                kind: FieldKind::Scalar,
            },
        ],
    },
    NodeKind {
        id: "hub:attr",
        lexical: false,
        fields: &[
            Field {
                name: "name",
                kind: FieldKind::Scalar,
            },
            Field {
                name: "value",
                kind: FieldKind::Scalar,
            },
        ],
    },
    NodeKind {
        id: "hub:cache",
        lexical: false,
        fields: &[
            Field {
                name: "fallback_keys",
                kind: FieldKind::Scalar,
            },
            Field {
                name: "key",
                kind: FieldKind::Scalar,
            },
            Field {
                name: "paths",
                kind: FieldKind::Scalar,
            },
            Field {
                name: "policy",
                kind: FieldKind::Scalar,
            },
            Field {
                name: "unprotect",
                kind: FieldKind::Scalar,
            },
            Field {
                name: "untracked",
                kind: FieldKind::Scalar,
            },
            Field {
                name: "when",
                kind: FieldKind::Scalar,
            },
        ],
    },
    NodeKind {
        id: "hub:concurrency",
        lexical: false,
        fields: &[
            Field {
                name: "cancel_in_progress",
                kind: FieldKind::Scalar,
            },
            Field {
                name: "group",
                kind: FieldKind::Scalar,
            },
            Field {
                name: "limit",
                kind: FieldKind::Scalar,
            },
            Field {
                name: "queue",
                kind: FieldKind::Scalar,
            },
        ],
    },
    NodeKind {
        id: "hub:condition",
        lexical: false,
        fields: &[
            Field {
                name: "before",
                kind: FieldKind::Scalar,
            },
            Field {
                name: "branch",
                kind: FieldKind::Scalar,
            },
            Field {
                name: "change_request",
                kind: FieldKind::Scalar,
            },
            Field {
                name: "changes",
                kind: FieldKind::Scalar,
            },
            Field {
                name: "combinator",
                kind: FieldKind::Scalar,
            },
            Field {
                name: "cron",
                kind: FieldKind::Scalar,
            },
            Field {
                name: "env_match",
                kind: FieldKind::Scalar,
            },
            Field {
                name: "event",
                kind: FieldKind::Scalar,
            },
            Field {
                name: "expr",
                kind: FieldKind::Scalar,
            },
            Field {
                name: "gate",
                kind: FieldKind::Scalar,
            },
            Field {
                name: "kind",
                kind: FieldKind::Scalar,
            },
            Field {
                name: "matrix_filter",
                kind: FieldKind::Scalar,
            },
            Field {
                name: "platform",
                kind: FieldKind::Scalar,
            },
            Field {
                name: "ref",
                kind: FieldKind::Scalar,
            },
            Field {
                name: "repo",
                kind: FieldKind::Scalar,
            },
            Field {
                name: "state",
                kind: FieldKind::Scalar,
            },
            Field {
                name: "tag",
                kind: FieldKind::Scalar,
            },
            Field {
                name: "target_branch",
                kind: FieldKind::Scalar,
            },
            Field {
                name: "triggered_by",
                kind: FieldKind::Scalar,
            },
        ],
    },
    NodeKind {
        id: "hub:dependency_edge",
        lexical: false,
        fields: &[
            Field {
                name: "arguments",
                kind: FieldKind::Scalar,
            },
            Field {
                name: "branches",
                kind: FieldKind::Scalar,
            },
            Field {
                name: "condition",
                kind: FieldKind::Ref("hub:condition"),
            },
            Field {
                name: "depends_on",
                kind: FieldKind::Ref("hub:dependency_edge"),
            },
            Field {
                name: "fail_fast",
                kind: FieldKind::Scalar,
            },
            Field {
                name: "fan_out",
                kind: FieldKind::Scalar,
            },
            Field {
                name: "id",
                kind: FieldKind::Scalar,
            },
            Field {
                name: "kind",
                kind: FieldKind::Scalar,
            },
            Field {
                name: "name",
                kind: FieldKind::Scalar,
            },
            Field {
                name: "tasks",
                kind: FieldKind::Scalar,
            },
            Field {
                name: "template_ref",
                kind: FieldKind::Scalar,
            },
        ],
    },
    NodeKind {
        id: "hub:deployment",
        lexical: false,
        fields: &[
            Field {
                name: "deployment",
                kind: FieldKind::Ref("hub:deployment"),
            },
            Field {
                name: "increments",
                kind: FieldKind::Scalar,
            },
            Field {
                name: "max_parallel",
                kind: FieldKind::Scalar,
            },
            Field {
                name: "name",
                kind: FieldKind::Scalar,
            },
            Field {
                name: "strategy_phases",
                kind: FieldKind::Scalar,
            },
            Field {
                name: "triggers",
                kind: FieldKind::Ref("hub:trigger"),
            },
            Field {
                name: "url",
                kind: FieldKind::Scalar,
            },
        ],
    },
    NodeKind {
        id: "hub:hook",
        lexical: false,
        fields: &[
            Field {
                name: "agent",
                kind: FieldKind::Ref("hub:agent"),
            },
            Field {
                name: "job",
                kind: FieldKind::Ref("hub:job"),
            },
            Field {
                name: "nested",
                kind: FieldKind::Scalar,
            },
            Field {
                name: "phase",
                kind: FieldKind::Scalar,
            },
            Field {
                name: "pre_script",
                kind: FieldKind::Scalar,
            },
            Field {
                name: "steps",
                kind: FieldKind::Ref("hub:step"),
            },
        ],
    },
    NodeKind {
        id: "hub:image",
        lexical: false,
        fields: &[
            Field {
                name: "credentials",
                kind: FieldKind::Scalar,
            },
            Field {
                name: "docker_socket",
                kind: FieldKind::Scalar,
            },
            Field {
                name: "endpoint",
                kind: FieldKind::Scalar,
            },
            Field {
                name: "env",
                kind: FieldKind::Scalar,
            },
            Field {
                name: "kind",
                kind: FieldKind::Scalar,
            },
            Field {
                name: "local",
                kind: FieldKind::Scalar,
            },
            Field {
                name: "mount_read_only",
                kind: FieldKind::Scalar,
            },
            Field {
                name: "name",
                kind: FieldKind::Scalar,
            },
            Field {
                name: "options",
                kind: FieldKind::Scalar,
            },
            Field {
                name: "ports",
                kind: FieldKind::Scalar,
            },
            Field {
                name: "run_as",
                kind: FieldKind::Scalar,
            },
            Field {
                name: "trigger",
                kind: FieldKind::Ref("hub:trigger"),
            },
            Field {
                name: "volumes",
                kind: FieldKind::Scalar,
            },
        ],
    },
    NodeKind {
        id: "hub:job",
        lexical: false,
        fields: &[
            Field {
                name: "after_steps",
                kind: FieldKind::Ref("hub:step"),
            },
            Field {
                name: "agent",
                kind: FieldKind::Ref("hub:agent"),
            },
            Field {
                name: "allow_failure",
                kind: FieldKind::Scalar,
            },
            Field {
                name: "artifacts",
                kind: FieldKind::Ref("hub:artifact"),
            },
            Field {
                name: "backend",
                kind: FieldKind::Scalar,
            },
            Field {
                name: "before_steps",
                kind: FieldKind::Ref("hub:step"),
            },
            Field {
                name: "cache",
                kind: FieldKind::Ref("hub:cache"),
            },
            Field {
                name: "concurrency",
                kind: FieldKind::Ref("hub:concurrency"),
            },
            Field {
                name: "condition",
                kind: FieldKind::Ref("hub:condition"),
            },
            Field {
                name: "defaults",
                kind: FieldKind::Scalar,
            },
            Field {
                name: "deployment",
                kind: FieldKind::Ref("hub:deployment"),
            },
            Field {
                name: "description",
                kind: FieldKind::Scalar,
            },
            Field {
                name: "extends",
                kind: FieldKind::Scalar,
            },
            Field {
                name: "hooks",
                kind: FieldKind::Ref("hub:hook"),
            },
            Field {
                name: "id",
                kind: FieldKind::Scalar,
            },
            Field {
                name: "image",
                kind: FieldKind::Ref("hub:image"),
            },
            Field {
                name: "matrix",
                kind: FieldKind::Ref("hub:matrix"),
            },
            Field {
                name: "name",
                kind: FieldKind::Scalar,
            },
            Field {
                name: "needs",
                kind: FieldKind::Ref("hub:dependency_edge"),
            },
            Field {
                name: "options",
                kind: FieldKind::Scalar,
            },
            Field {
                name: "parameters",
                kind: FieldKind::Ref("hub:parameter"),
            },
            Field {
                name: "permissions",
                kind: FieldKind::Ref("hub:permissions"),
            },
            Field {
                name: "retry",
                kind: FieldKind::Ref("hub:retry"),
            },
            Field {
                name: "secrets",
                kind: FieldKind::Ref("hub:secret"),
            },
            Field {
                name: "services",
                kind: FieldKind::Ref("hub:service"),
            },
            Field {
                name: "stage",
                kind: FieldKind::Ref("hub:stage"),
            },
            Field {
                name: "steps",
                kind: FieldKind::Ref("hub:step"),
            },
            Field {
                name: "timeout",
                kind: FieldKind::Scalar,
            },
            Field {
                name: "toolchain",
                kind: FieldKind::Scalar,
            },
            Field {
                name: "trigger",
                kind: FieldKind::Ref("hub:trigger"),
            },
            Field {
                name: "uses",
                kind: FieldKind::Scalar,
            },
            Field {
                name: "variables",
                kind: FieldKind::Ref("hub:variable"),
            },
            Field {
                name: "working_dir",
                kind: FieldKind::Scalar,
            },
        ],
    },
    NodeKind {
        id: "hub:matrix",
        lexical: false,
        fields: &[
            Field {
                name: "adjustments",
                kind: FieldKind::Scalar,
            },
            Field {
                name: "axes",
                kind: FieldKind::Scalar,
            },
            Field {
                name: "exclude",
                kind: FieldKind::Scalar,
            },
            Field {
                name: "skip",
                kind: FieldKind::Scalar,
            },
            Field {
                name: "soft_fail",
                kind: FieldKind::Scalar,
            },
            Field {
                name: "stages",
                kind: FieldKind::Ref("hub:stage"),
            },
        ],
    },
    NodeKind {
        id: "hub:notification",
        lexical: false,
        fields: &[
            Field {
                name: "channel",
                kind: FieldKind::Scalar,
            },
            Field {
                name: "condition",
                kind: FieldKind::Ref("hub:condition"),
            },
            Field {
                name: "message",
                kind: FieldKind::Scalar,
            },
        ],
    },
    NodeKind {
        id: "hub:parameter",
        lexical: false,
        fields: &[
            Field {
                name: "artifacts",
                kind: FieldKind::Ref("hub:artifact"),
            },
            Field {
                name: "constraint",
                kind: FieldKind::Scalar,
            },
            Field {
                name: "default",
                kind: FieldKind::Scalar,
            },
            Field {
                name: "deprecated",
                kind: FieldKind::Scalar,
            },
            Field {
                name: "description",
                kind: FieldKind::Scalar,
            },
            Field {
                name: "gate",
                kind: FieldKind::Scalar,
            },
            Field {
                name: "multiple",
                kind: FieldKind::Scalar,
            },
            Field {
                name: "name",
                kind: FieldKind::Scalar,
            },
            Field {
                name: "options",
                kind: FieldKind::Scalar,
            },
            Field {
                name: "parameters",
                kind: FieldKind::Ref("hub:parameter"),
            },
            Field {
                name: "required",
                kind: FieldKind::Scalar,
            },
            Field {
                name: "result",
                kind: FieldKind::Scalar,
            },
            Field {
                name: "type",
                kind: FieldKind::Scalar,
            },
            Field {
                name: "value",
                kind: FieldKind::Scalar,
            },
        ],
    },
    NodeKind {
        id: "hub:permissions",
        lexical: false,
        fields: &[
            Field {
                name: "prompt",
                kind: FieldKind::Scalar,
            },
            Field {
                name: "scopes",
                kind: FieldKind::Scalar,
            },
        ],
    },
    NodeKind {
        id: "hub:pipeline",
        lexical: false,
        fields: &[
            Field {
                name: "agent",
                kind: FieldKind::Ref("hub:agent"),
            },
            Field {
                name: "anchors",
                kind: FieldKind::Scalar,
            },
            Field {
                name: "artifact_store",
                kind: FieldKind::Scalar,
            },
            Field {
                name: "artifacts",
                kind: FieldKind::Ref("hub:artifact"),
            },
            Field {
                name: "cache",
                kind: FieldKind::Ref("hub:cache"),
            },
            Field {
                name: "concurrency",
                kind: FieldKind::Ref("hub:concurrency"),
            },
            Field {
                name: "defaults",
                kind: FieldKind::Scalar,
            },
            Field {
                name: "dependencies",
                kind: FieldKind::Scalar,
            },
            Field {
                name: "deployments",
                kind: FieldKind::Ref("hub:deployment"),
            },
            Field {
                name: "description",
                kind: FieldKind::Scalar,
            },
            Field {
                name: "entrypoint",
                kind: FieldKind::Scalar,
            },
            Field {
                name: "execution_mode",
                kind: FieldKind::Scalar,
            },
            Field {
                name: "experimental",
                kind: FieldKind::Scalar,
            },
            Field {
                name: "export",
                kind: FieldKind::Scalar,
            },
            Field {
                name: "hooks",
                kind: FieldKind::Ref("hub:hook"),
            },
            Field {
                name: "image",
                kind: FieldKind::Ref("hub:image"),
            },
            Field {
                name: "includes",
                kind: FieldKind::Scalar,
            },
            Field {
                name: "jobs",
                kind: FieldKind::Ref("hub:job"),
            },
            Field {
                name: "logs",
                kind: FieldKind::Scalar,
            },
            Field {
                name: "matrix",
                kind: FieldKind::Ref("hub:matrix"),
            },
            Field {
                name: "name",
                kind: FieldKind::Scalar,
            },
            Field {
                name: "notifications",
                kind: FieldKind::Ref("hub:notification"),
            },
            Field {
                name: "options",
                kind: FieldKind::Scalar,
            },
            Field {
                name: "parameters",
                kind: FieldKind::Ref("hub:parameter"),
            },
            Field {
                name: "permissions",
                kind: FieldKind::Ref("hub:permissions"),
            },
            Field {
                name: "pipelines",
                kind: FieldKind::Ref("hub:pipeline"),
            },
            Field {
                name: "project",
                kind: FieldKind::Scalar,
            },
            Field {
                name: "resources",
                kind: FieldKind::Ref("hub:resource"),
            },
            Field {
                name: "role",
                kind: FieldKind::Scalar,
            },
            Field {
                name: "secrets",
                kind: FieldKind::Ref("hub:secret"),
            },
            Field {
                name: "services",
                kind: FieldKind::Ref("hub:service"),
            },
            Field {
                name: "stages",
                kind: FieldKind::Ref("hub:stage"),
            },
            Field {
                name: "steps",
                kind: FieldKind::Ref("hub:step"),
            },
            Field {
                name: "tags",
                kind: FieldKind::Scalar,
            },
            Field {
                name: "targets",
                kind: FieldKind::Scalar,
            },
            Field {
                name: "templates",
                kind: FieldKind::Ref("hub:template"),
            },
            Field {
                name: "timeout",
                kind: FieldKind::Scalar,
            },
            Field {
                name: "toolchain",
                kind: FieldKind::Scalar,
            },
            Field {
                name: "triggers",
                kind: FieldKind::Ref("hub:trigger"),
            },
            Field {
                name: "variables",
                kind: FieldKind::Ref("hub:variable"),
            },
            Field {
                name: "version",
                kind: FieldKind::Scalar,
            },
            Field {
                name: "workflows",
                kind: FieldKind::Scalar,
            },
        ],
    },
    NodeKind {
        id: "hub:resource",
        lexical: false,
        fields: &[
            Field {
                name: "checkout",
                kind: FieldKind::Scalar,
            },
            Field {
                name: "connection",
                kind: FieldKind::Scalar,
            },
            Field {
                name: "container_config",
                kind: FieldKind::Scalar,
            },
            Field {
                name: "data",
                kind: FieldKind::Scalar,
            },
            Field {
                name: "filters",
                kind: FieldKind::Scalar,
            },
            Field {
                name: "kind",
                kind: FieldKind::Scalar,
            },
            Field {
                name: "name",
                kind: FieldKind::Scalar,
            },
            Field {
                name: "pools",
                kind: FieldKind::Scalar,
            },
            Field {
                name: "ref",
                kind: FieldKind::Scalar,
            },
            Field {
                name: "repositories",
                kind: FieldKind::Scalar,
            },
            Field {
                name: "trigger",
                kind: FieldKind::Ref("hub:trigger"),
            },
        ],
    },
    NodeKind {
        id: "hub:retry",
        lexical: false,
        fields: &[
            Field {
                name: "exit_status",
                kind: FieldKind::Scalar,
            },
            Field {
                name: "limit",
                kind: FieldKind::Scalar,
            },
            Field {
                name: "signal",
                kind: FieldKind::Scalar,
            },
            Field {
                name: "signal_reason",
                kind: FieldKind::Scalar,
            },
        ],
    },
    NodeKind {
        id: "hub:secret",
        lexical: false,
        fields: &[
            Field {
                name: "backend",
                kind: FieldKind::Scalar,
            },
            Field {
                name: "key_ref",
                kind: FieldKind::Scalar,
            },
            Field {
                name: "kind",
                kind: FieldKind::Scalar,
            },
            Field {
                name: "name",
                kind: FieldKind::Scalar,
            },
            Field {
                name: "ref",
                kind: FieldKind::Scalar,
            },
            Field {
                name: "signature",
                kind: FieldKind::Scalar,
            },
            Field {
                name: "target",
                kind: FieldKind::Scalar,
            },
            Field {
                name: "value",
                kind: FieldKind::Scalar,
            },
        ],
    },
    NodeKind {
        id: "hub:service",
        lexical: false,
        fields: &[
            Field {
                name: "command",
                kind: FieldKind::Scalar,
            },
            Field {
                name: "condition",
                kind: FieldKind::Ref("hub:condition"),
            },
            Field {
                name: "credentials",
                kind: FieldKind::Scalar,
            },
            Field {
                name: "env",
                kind: FieldKind::Scalar,
            },
            Field {
                name: "image",
                kind: FieldKind::Ref("hub:image"),
            },
            Field {
                name: "kind",
                kind: FieldKind::Scalar,
            },
            Field {
                name: "name",
                kind: FieldKind::Scalar,
            },
            Field {
                name: "options",
                kind: FieldKind::Scalar,
            },
            Field {
                name: "ports",
                kind: FieldKind::Scalar,
            },
            Field {
                name: "pull",
                kind: FieldKind::Scalar,
            },
            Field {
                name: "resources",
                kind: FieldKind::Ref("hub:resource"),
            },
            Field {
                name: "volumes",
                kind: FieldKind::Scalar,
            },
            Field {
                name: "working_dir",
                kind: FieldKind::Scalar,
            },
        ],
    },
    NodeKind {
        id: "hub:stage",
        lexical: false,
        fields: &[
            Field {
                name: "blockers",
                kind: FieldKind::Scalar,
            },
            Field {
                name: "condition",
                kind: FieldKind::Ref("hub:condition"),
            },
            Field {
                name: "depends_on",
                kind: FieldKind::Ref("hub:dependency_edge"),
            },
            Field {
                name: "deployment",
                kind: FieldKind::Ref("hub:deployment"),
            },
            Field {
                name: "fail_fast",
                kind: FieldKind::Scalar,
            },
            Field {
                name: "group",
                kind: FieldKind::Scalar,
            },
            Field {
                name: "hooks",
                kind: FieldKind::Ref("hub:hook"),
            },
            Field {
                name: "id",
                kind: FieldKind::Scalar,
            },
            Field {
                name: "jobs",
                kind: FieldKind::Ref("hub:job"),
            },
            Field {
                name: "name",
                kind: FieldKind::Scalar,
            },
            Field {
                name: "nested_stage",
                kind: FieldKind::Ref("hub:stage"),
            },
            Field {
                name: "nested_stages",
                kind: FieldKind::Ref("hub:stage"),
            },
            Field {
                name: "notify",
                kind: FieldKind::Scalar,
            },
            Field {
                name: "parameters",
                kind: FieldKind::Ref("hub:parameter"),
            },
            Field {
                name: "skip",
                kind: FieldKind::Scalar,
            },
            Field {
                name: "trigger",
                kind: FieldKind::Ref("hub:trigger"),
            },
        ],
    },
    NodeKind {
        id: "hub:step",
        lexical: false,
        fields: &[
            Field {
                name: "agent",
                kind: FieldKind::Ref("hub:agent"),
            },
            Field {
                name: "branches",
                kind: FieldKind::Scalar,
            },
            Field {
                name: "condition",
                kind: FieldKind::Ref("hub:condition"),
            },
            Field {
                name: "continue_on_error",
                kind: FieldKind::Scalar,
            },
            Field {
                name: "depends_on",
                kind: FieldKind::Ref("hub:dependency_edge"),
            },
            Field {
                name: "enabled",
                kind: FieldKind::Scalar,
            },
            Field {
                name: "env",
                kind: FieldKind::Scalar,
            },
            Field {
                name: "gate",
                kind: FieldKind::Scalar,
            },
            Field {
                name: "id",
                kind: FieldKind::Scalar,
            },
            Field {
                name: "image",
                kind: FieldKind::Ref("hub:image"),
            },
            Field {
                name: "kind",
                kind: FieldKind::Scalar,
            },
            Field {
                name: "name",
                kind: FieldKind::Scalar,
            },
            Field {
                name: "nested_steps",
                kind: FieldKind::Ref("hub:step"),
            },
            Field {
                name: "on_error",
                kind: FieldKind::Scalar,
            },
            Field {
                name: "parameters",
                kind: FieldKind::Ref("hub:parameter"),
            },
            Field {
                name: "permissions",
                kind: FieldKind::Ref("hub:permissions"),
            },
            Field {
                name: "resources",
                kind: FieldKind::Ref("hub:resource"),
            },
            Field {
                name: "retry",
                kind: FieldKind::Ref("hub:retry"),
            },
            Field {
                name: "run",
                kind: FieldKind::Scalar,
            },
            Field {
                name: "shell",
                kind: FieldKind::Scalar,
            },
            Field {
                name: "teams",
                kind: FieldKind::Scalar,
            },
            Field {
                name: "timeout",
                kind: FieldKind::Scalar,
            },
            Field {
                name: "uses",
                kind: FieldKind::Scalar,
            },
            Field {
                name: "volume_mounts",
                kind: FieldKind::Scalar,
            },
            Field {
                name: "with",
                kind: FieldKind::Scalar,
            },
            Field {
                name: "working_dir",
                kind: FieldKind::Scalar,
            },
        ],
    },
    NodeKind {
        id: "hub:template",
        lexical: false,
        fields: &[
            Field {
                name: "body",
                kind: FieldKind::Scalar,
            },
            Field {
                name: "parameters",
                kind: FieldKind::Ref("hub:parameter"),
            },
            Field {
                name: "ref",
                kind: FieldKind::Scalar,
            },
            Field {
                name: "resources",
                kind: FieldKind::Ref("hub:resource"),
            },
            Field {
                name: "triggers",
                kind: FieldKind::Ref("hub:trigger"),
            },
            Field {
                name: "variables",
                kind: FieldKind::Ref("hub:variable"),
            },
        ],
    },
    NodeKind {
        id: "hub:trigger",
        lexical: false,
        fields: &[
            Field {
                name: "async",
                kind: FieldKind::Scalar,
            },
            Field {
                name: "branches",
                kind: FieldKind::Scalar,
            },
            Field {
                name: "condition",
                kind: FieldKind::Ref("hub:condition"),
            },
            Field {
                name: "depends_on",
                kind: FieldKind::Ref("hub:dependency_edge"),
            },
            Field {
                name: "id",
                kind: FieldKind::Scalar,
            },
            Field {
                name: "kind",
                kind: FieldKind::Scalar,
            },
            Field {
                name: "name",
                kind: FieldKind::Scalar,
            },
            Field {
                name: "parent",
                kind: FieldKind::Scalar,
            },
            Field {
                name: "paths",
                kind: FieldKind::Scalar,
            },
            Field {
                name: "schedule",
                kind: FieldKind::Scalar,
            },
            Field {
                name: "skip",
                kind: FieldKind::Scalar,
            },
            Field {
                name: "soft_fail",
                kind: FieldKind::Scalar,
            },
            Field {
                name: "target_pipeline",
                kind: FieldKind::Ref("hub:pipeline"),
            },
        ],
    },
    NodeKind {
        id: "hub:variable",
        lexical: false,
        fields: &[
            Field {
                name: "bindings",
                kind: FieldKind::Scalar,
            },
            Field {
                name: "exported",
                kind: FieldKind::Scalar,
            },
            Field {
                name: "from_store",
                kind: FieldKind::Scalar,
            },
            Field {
                name: "git_credentials",
                kind: FieldKind::Scalar,
            },
            Field {
                name: "parameters",
                kind: FieldKind::Ref("hub:parameter"),
            },
            Field {
                name: "shell",
                kind: FieldKind::Scalar,
            },
        ],
    },
    NodeKind {
        id: "hub:workspace",
        lexical: false,
        fields: &[
            Field {
                name: "clean",
                kind: FieldKind::Scalar,
            },
            Field {
                name: "description",
                kind: FieldKind::Scalar,
            },
            Field {
                name: "mount_path",
                kind: FieldKind::Scalar,
            },
            Field {
                name: "name",
                kind: FieldKind::Scalar,
            },
            Field {
                name: "optional",
                kind: FieldKind::Scalar,
            },
            Field {
                name: "read_only_mounts",
                kind: FieldKind::Scalar,
            },
            Field {
                name: "source",
                kind: FieldKind::Scalar,
            },
            Field {
                name: "sub_path",
                kind: FieldKind::Scalar,
            },
        ],
    },
    NodeKind {
        id: "hub:anchor",
        lexical: true,
        fields: &[],
    },
    NodeKind {
        id: "hub:blank_run",
        lexical: true,
        fields: &[],
    },
    NodeKind {
        id: "hub:comment",
        lexical: true,
        fields: &[],
    },
    NodeKind {
        id: "hub:key_order",
        lexical: true,
        fields: &[],
    },
    NodeKind {
        id: "hub:provenance",
        lexical: true,
        fields: &[],
    },
    NodeKind {
        id: "hub:scalar_style",
        lexical: true,
        fields: &[],
    },
];
