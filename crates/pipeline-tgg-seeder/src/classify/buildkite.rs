//! Construct classification for buildkite. Generated.

/// `(key, ir-construct)`.
pub const CONSTRUCT_KEYS: &[(&str, &str)] = &[
    ("cache", "map:cache"),
    ("concurrency", "concurrency"),
    ("depends_on", "map:dependency_edge"),
    ("image", "image"),
    ("matrix", "matrix"),
    ("retry", "map:retry"),
    ("secrets", "map:secret"),
    ("steps", "step"),
];

/// Construct-field keys whose value is canonically a LIST
/// (the single-mapping form is sugar for a one-item list).
pub const LIST_CONSTRUCT_KEYS: &[&str] = &[
    "agents",
    "artifact_paths",
    "branches",
    "cache",
    "command",
    "commands",
    "depends_on",
    "if_changed",
    "matrix",
    "notify",
    "script",
    "secrets",
    "steps",
];
