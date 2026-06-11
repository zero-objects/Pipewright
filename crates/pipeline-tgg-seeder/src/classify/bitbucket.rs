//! Construct classification for bitbucket. Generated.

/// `(key, ir-construct)`.
pub const CONSTRUCT_KEYS: &[(&str, &str)] = &[
    ("agent", "agent"),
    ("artifacts", "artifact"),
    ("condition", "condition"),
    ("deployment", "deployment"),
    ("image", "image"),
    ("permissions", "permissions"),
    ("pipelines", "pipeline"),
    ("services", "map:service"),
    ("trigger", "trigger"),
    ("triggers", "map:trigger"),
];

/// Construct-field keys whose value is canonically a LIST
/// (the single-mapping form is sugar for a one-item list).
pub const LIST_CONSTRUCT_KEYS: &[&str] = &[
    "after-script",
    "artifacts",
    "caches",
    "parallel",
    "pipelines",
    "runs-on",
    "script",
    "services",
    "steps",
    "triggers",
    "variables",
    "volumes",
];
