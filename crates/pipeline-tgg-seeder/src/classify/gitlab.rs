//! Construct classification for gitlab. Generated.

/// `(key, ir-construct)`.
pub const CONSTRUCT_KEYS: &[(&str, &str)] = &[
    ("artifacts", "artifact"),
    ("cache", "cache"),
    ("hooks", "hook"),
    ("image", "map:image"),
    ("needs", "map:dependency_edge"),
    ("retry", "map:retry"),
    ("rules", "rule_clause"),
    ("secrets", "map:secret"),
    ("services", "map:service"),
    ("stage", "stage"),
    ("stages", "stage"),
    ("trigger", "map:trigger"),
    ("variables", "map:variable"),
    ("workflow", "trigger"),
];

/// Construct-field keys whose value is canonically a LIST
/// (the single-mapping form is sugar for a one-item list).
pub const LIST_CONSTRUCT_KEYS: &[&str] = &[
    "after_script",
    "before_script",
    "cache",
    "dependencies",
    "except",
    "needs",
    "only",
    "rules",
    "run",
    "script",
    "services",
    "stage",
    "stages",
    "tags",
];
