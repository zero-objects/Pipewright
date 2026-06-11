# Changelog

All notable changes to this project are documented here. The format is
based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and
this project adheres to [Semantic Versioning](https://semver.org/).

## [Unreleased]

## [1.0.0-rc.1]

First public release candidate.

### Added

- **17-platform support** for inspect, render, document, and migrate:
  argo, aws_codebuild, aws_codepipeline, azure, bitbucket, buildkite,
  circleci, dagger, drone, earthly, github, gitlab, google_cloudbuild,
  jenkins, tekton, travis, woodpecker.
- **Neutral Hub-IR**: every config is lifted (via a Triple Graph Grammar
  cascade) into one typed graph; all views and exports are projections
  of it.
- **Local Docker runner** (`pipewright run`) for the 11 container-shell
  platforms — bind-mounts the repo, passes env, evaluates `rules:if` /
  `when`, starts `services:` as sidecars, streams logs. The mount is
  **read-only by default** (a foreign pipeline is untrusted code);
  `--rw-copy` runs writable on a throwaway copy, `--rw` writes in place.
  The 6 translate-only platforms (k8s CRDs, code-defined, server-runtime)
  refuse honestly with a reason.
- **Migration** between platforms, including cross-structural families
  (job-based ↔ step-flat) via model synthesis.
- **`pipewright plan`** (dependency-ordered dry run, no Docker) and a
  human-readable runbook (`render --format md`).
- **Qt6/QML desktop UI** over a C-ABI FFI.
- Verification gates: real-config corpus (59 fixtures), chaos round-trip
  stress (17 × 30 seeds), cross-platform interop matrix (272 pairs).

[Unreleased]: ../../compare/v1.0.0-rc.1...HEAD
[1.0.0-rc.1]: ../../releases/tag/v1.0.0-rc.1
