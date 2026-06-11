//! Construct classification for travis. Generated.

/// `(key, ir-construct)`.
pub const CONSTRUCT_KEYS: &[(&str, &str)] = &[
    ("cache", "map:cache"),
    ("jobs", "map:job"),
    ("matrix", "map:matrix"),
    ("notifications", "map:notification"),
    ("services", "service"),
    ("stages", "map:stage"),
];

/// Construct-field keys whose value is canonically a LIST
/// (the single-mapping form is sugar for a one-item list).
pub const LIST_CONSTRUCT_KEYS: &[&str] = &[
    "after_deploy",
    "after_failure",
    "after_script",
    "after_success",
    "before_cache",
    "before_deploy",
    "before_install",
    "before_script",
    "cache",
    "deploy",
    "env",
    "install",
    "script",
    "services",
    "stages",
];
