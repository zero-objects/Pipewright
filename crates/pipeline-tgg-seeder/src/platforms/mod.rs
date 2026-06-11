//! Per-platform top-level dispatchers.
//!
//! All recursive walking lives in [`crate`] — these modules only
//! encode where a given platform places its jobs and how to
//! classify its top-level keys. One function per platform:
//! `seed_from_document(doc, source_file) -> SeededGraph`.

use crate::SeededGraph;
use pipeline_cst::Document;

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

/// Seed a CST document using the dispatcher for `platform`.
/// Returns `None` if the platform is unknown.
#[must_use]
pub fn seed(platform: &str, doc: &Document, source_file: &str) -> Option<SeededGraph> {
    match platform {
        "argo" => Some(argo::seed_from_document(doc, source_file)),
        "aws_codebuild" => Some(aws_codebuild::seed_from_document(doc, source_file)),
        "aws_codepipeline" => Some(aws_codepipeline::seed_from_document(doc, source_file)),
        "azure" => Some(azure::seed_from_document(doc, source_file)),
        "bitbucket" => Some(bitbucket::seed_from_document(doc, source_file)),
        "buildkite" => Some(buildkite::seed_from_document(doc, source_file)),
        "circleci" => Some(circleci::seed_from_document(doc, source_file)),
        "dagger" => Some(dagger::seed_from_document(doc, source_file)),
        "drone" => Some(drone::seed_from_document(doc, source_file)),
        "earthly" => Some(earthly::seed_from_document(doc, source_file)),
        "github" => Some(github::seed_from_document(doc, source_file)),
        "gitlab" => Some(gitlab::seed_from_document(doc, source_file)),
        "google_cloudbuild" => Some(google_cloudbuild::seed_from_document(doc, source_file)),
        "jenkins" => Some(jenkins::seed_from_document(doc, source_file)),
        "tekton" => Some(tekton::seed_from_document(doc, source_file)),
        "travis" => Some(travis::seed_from_document(doc, source_file)),
        "woodpecker" => Some(woodpecker::seed_from_document(doc, source_file)),
        _ => None,
    }
}
