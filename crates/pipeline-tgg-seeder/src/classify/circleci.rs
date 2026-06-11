//! Construct classification for circleci. Generated.

/// `(key, ir-construct)`.
pub const CONSTRUCT_KEYS: &[(&str, &str)] = &[
    ("jobs", "map:job"),
    ("parameters", "map:parameter"),
    ("steps", "step"),
];

/// Construct-field keys whose value is canonically a LIST
/// (the single-mapping form is sugar for a one-item list).
pub const LIST_CONSTRUCT_KEYS: &[&str] = &["docker", "steps"];
