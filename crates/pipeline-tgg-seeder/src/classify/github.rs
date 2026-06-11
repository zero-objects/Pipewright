//! Construct classification for github. Generated.

/// `(key, ir-construct)`.
pub const CONSTRUCT_KEYS: &[(&str, &str)] = &[
    ("concurrency", "concurrency"),
    ("deployment", "deployment"),
    ("image", "image"),
    ("jobs", "map:job"),
    ("needs", "dependency_edge"),
    ("permissions", "permissions"),
    ("secrets", "map:secret"),
    ("services", "map:service"),
    ("steps", "step"),
];

/// Construct-field keys whose value is canonically a LIST
/// (the single-mapping form is sugar for a one-item list).
pub const LIST_CONSTRUCT_KEYS: &[&str] = &["needs", "on", "steps"];
