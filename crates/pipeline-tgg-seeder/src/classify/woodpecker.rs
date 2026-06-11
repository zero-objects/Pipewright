//! Construct classification for woodpecker. Generated.

/// `(key, ir-construct)`.
pub const CONSTRUCT_KEYS: &[(&str, &str)] = &[
    ("depends_on", "dependency_edge"),
    ("image", "image"),
    ("matrix", "matrix"),
    ("services", "map:service"),
    ("steps", "map:step"),
    ("variables", "variable"),
];

/// Construct-field keys whose value is canonically a LIST
/// (the single-mapping form is sugar for a one-item list).
pub const LIST_CONSTRUCT_KEYS: &[&str] = &[
    "clone",
    "depends_on",
    "runs_on",
    "services",
    "steps",
    "when",
];
