//! Construct classification for argo. Generated.

/// `(key, ir-construct)`.
pub const CONSTRUCT_KEYS: &[(&str, &str)] = &[
    ("artifacts", "artifact"),
    ("image", "image"),
    ("parameters", "parameter"),
    ("resources", "map:resource"),
    ("steps", "step"),
    ("templates", "job"),
];

/// Construct-field keys whose value is canonically a LIST
/// (the single-mapping form is sugar for a one-item list).
pub const LIST_CONSTRUCT_KEYS: &[&str] = &[
    "artifacts",
    "dependencies",
    "parameters",
    "steps",
    "tasks",
    "templates",
    "volumeClaimTemplates",
    "volumes",
];
