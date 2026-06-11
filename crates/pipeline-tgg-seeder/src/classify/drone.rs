//! Construct classification for drone. Generated.

/// `(key, ir-construct)`.
pub const CONSTRUCT_KEYS: &[(&str, &str)] = &[
    ("concurrency", "concurrency"),
    ("depends_on", "dependency_edge"),
    ("image", "image"),
    ("services", "service"),
    ("steps", "step"),
];

/// Construct-field keys whose value is canonically a LIST
/// (the single-mapping form is sugar for a one-item list).
pub const LIST_CONSTRUCT_KEYS: &[&str] = &[
    "depends_on",
    "host_aliases",
    "image_pull_secrets",
    "services",
    "steps",
    "tolerations",
    "volumes",
];
