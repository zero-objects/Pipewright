//! Construct classification for google_cloudbuild. Generated.

/// `(key, ir-construct)`.
pub const CONSTRUCT_KEYS: &[(&str, &str)] = &[
    ("artifacts", "artifact"),
    ("goModules", "artifact"),
    ("mavenArtifacts", "artifact"),
    ("npmPackages", "artifact"),
    ("pythonPackages", "artifact"),
    ("secrets", "secret"),
    ("steps", "step"),
];

/// Construct-field keys whose value is canonically a LIST
/// (the single-mapping form is sugar for a one-item list).
pub const LIST_CONSTRUCT_KEYS: &[&str] = &[
    "args",
    "env",
    "goModules",
    "images",
    "mavenArtifacts",
    "npmPackages",
    "pythonPackages",
    "secretEnv",
    "secrets",
    "steps",
    "volumes",
    "waitFor",
];
