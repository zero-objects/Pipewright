//! Construct classification for tekton. Generated.

/// `(key, ir-construct)`.
pub const CONSTRUCT_KEYS: &[(&str, &str)] = &[
    ("image", "image"),
    ("matrix", "map:matrix"),
    ("steps", "step"),
    ("tasks", "job"),
];

/// Construct-field keys whose value is canonically a LIST
/// (the single-mapping form is sugar for a one-item list).
pub const LIST_CONSTRUCT_KEYS: &[&str] = &[
    "finally",
    "params",
    "results",
    "runAfter",
    "sidecars",
    "steps",
    "tasks",
    "when",
    "workspaces",
];
