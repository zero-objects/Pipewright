//! GENERATED from catalog/classify/*.toml by
//! catalog/gen_classification.py. Do not edit; regenerate.
//!
//! Per-platform construct classification tables. Each one
//! maps a mapping-entry key to the IR construct kind the
//! entry's value-mapping represents — the seeder tags the
//! `cst:Mapping` accordingly so TGG rules can anchor on it.

pub mod argo;
pub mod aws_codebuild;
pub mod aws_codepipeline;
pub mod azure;
pub mod bitbucket;
pub mod buildkite;
pub mod circleci;
pub mod dagger;
pub mod drone;
pub mod earthly;
pub mod github;
pub mod gitlab;
pub mod google_cloudbuild;
pub mod jenkins;
pub mod tekton;
pub mod travis;
pub mod woodpecker;

/// Look up a platform's classification table by name.
#[must_use]
pub fn for_platform(name: &str) -> Option<crate::Classify<'static>> {
    match name {
        "gitlab" => Some(gitlab::CONSTRUCT_KEYS),
        "github" => Some(github::CONSTRUCT_KEYS),
        "azure" => Some(azure::CONSTRUCT_KEYS),
        "circleci" => Some(circleci::CONSTRUCT_KEYS),
        "travis" => Some(travis::CONSTRUCT_KEYS),
        "bitbucket" => Some(bitbucket::CONSTRUCT_KEYS),
        "buildkite" => Some(buildkite::CONSTRUCT_KEYS),
        "drone" => Some(drone::CONSTRUCT_KEYS),
        "woodpecker" => Some(woodpecker::CONSTRUCT_KEYS),
        "tekton" => Some(tekton::CONSTRUCT_KEYS),
        "argo" => Some(argo::CONSTRUCT_KEYS),
        "google_cloudbuild" => Some(google_cloudbuild::CONSTRUCT_KEYS),
        "aws_codebuild" => Some(aws_codebuild::CONSTRUCT_KEYS),
        "aws_codepipeline" => Some(aws_codepipeline::CONSTRUCT_KEYS),
        "jenkins" => Some(jenkins::CONSTRUCT_KEYS),
        "dagger" => Some(dagger::CONSTRUCT_KEYS),
        "earthly" => Some(earthly::CONSTRUCT_KEYS),
        _ => None,
    }
}

/// Look up a platform's list-canonical construct-field keys.
#[must_use]
pub fn list_fields_for_platform(name: &str) -> &'static [&'static str] {
    match name {
        "gitlab" => gitlab::LIST_CONSTRUCT_KEYS,
        "github" => github::LIST_CONSTRUCT_KEYS,
        "azure" => azure::LIST_CONSTRUCT_KEYS,
        "circleci" => circleci::LIST_CONSTRUCT_KEYS,
        "travis" => travis::LIST_CONSTRUCT_KEYS,
        "bitbucket" => bitbucket::LIST_CONSTRUCT_KEYS,
        "buildkite" => buildkite::LIST_CONSTRUCT_KEYS,
        "drone" => drone::LIST_CONSTRUCT_KEYS,
        "woodpecker" => woodpecker::LIST_CONSTRUCT_KEYS,
        "tekton" => tekton::LIST_CONSTRUCT_KEYS,
        "argo" => argo::LIST_CONSTRUCT_KEYS,
        "google_cloudbuild" => google_cloudbuild::LIST_CONSTRUCT_KEYS,
        "aws_codebuild" => aws_codebuild::LIST_CONSTRUCT_KEYS,
        "aws_codepipeline" => aws_codepipeline::LIST_CONSTRUCT_KEYS,
        "jenkins" => jenkins::LIST_CONSTRUCT_KEYS,
        "dagger" => dagger::LIST_CONSTRUCT_KEYS,
        "earthly" => earthly::LIST_CONSTRUCT_KEYS,
        _ => &[],
    }
}
