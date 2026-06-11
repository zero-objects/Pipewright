//! Render a value tree into a platform-specific serialisation.
//!
//! YAML covers the tier-1/2/3 platforms (14 of 17). Tier-4 (dagger,
//! earthly) need their own renderers — those are stubbed and the
//! chaos test only covers YAML platforms for now.

pub mod earthfile;
pub mod jenkinsfile;
pub mod yaml;
