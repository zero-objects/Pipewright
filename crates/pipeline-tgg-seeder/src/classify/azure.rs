//! Construct classification for azure. Generated.

/// `(key, ir-construct)`.
pub const CONSTRUCT_KEYS: &[(&str, &str)] = &[
    ("artifact", "artifact"),
    ("branches", "include_exclude_filter"),
    ("condition", "condition"),
    ("jobs", "job"),
    ("parameters", "map:parameter"),
    ("paths", "include_exclude_filter"),
    ("pr", "pull_request"),
    ("resources", "map:resource"),
    ("schedules", "schedule"),
    ("services", "map:service"),
    ("stages", "stage"),
    ("steps", "step"),
    ("tags", "include_exclude_filter"),
    ("trigger", "trigger"),
    ("variables", "map:variable"),
];

/// Construct-field keys whose value is canonically a LIST
/// (the single-mapping form is sugar for a one-item list).
pub const LIST_CONSTRUCT_KEYS: &[&str] = &[
    "jobs",
    "parameters",
    "phases",
    "pr",
    "resources",
    "schedules",
    "stages",
    "steps",
    "trigger",
];
