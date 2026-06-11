//! `JobIdentity` — opaque stable string + origin metadata.
//!
//! A required field on a job (Hub-IR v0.1 §2.1.1). Empirically motivated
//! by an early identity spike: no single identity strategy survives
//! every edit scenario, so `JobIdentity`
//! carries — alongside the UUID — a `JobIdentityOrigin` marker that
//! records *how* the identity was established.
//!
//! Trust level depends on `origin`:
//! - `FromLockfile`            — fully trusted, path matched
//! - `FreshlyAssigned`         — fully trusted, new job
//! - `UserConfirmed`           — fully trusted, the user confirmed it
//! - `RecoveredFromContent`    — recovered via Levenshtein, with a score
//! - `Uncertain`               — nothing matched; a marker for the UI/tool
//!
//! ## Implementation note on the similarity score
//!
//! The spec (Hub-IR v0.1 §2.1.1) lists `similarity_score: f32`. In Rust
//! `f32` is not `Eq`/`Hash` (because of `NaN` asymmetry), so the score is
//! stored as basis points instead: a `u32` in `0..=10000` corresponds to
//! `0.0..=1.0` with four decimal places of precision. The `as_f32()`
//! helper is provided for consumer convenience.

use serde::{Deserialize, Serialize};

/// Stable identity of a job across edit history.
///
/// `uuid` is opaque — consumers should treat it as a string token,
/// not parse it. `origin` explains how this identity was established.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct JobIdentity {
    pub uuid: String,
    pub origin: JobIdentityOrigin,
}

/// How a `JobIdentity` was established.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum JobIdentityOrigin {
    /// First time we see this job — new UUID, no prior reference.
    FreshlyAssigned,
    /// UUID loaded from `.pipeline.lock` by path-key match.
    FromLockfile,
    /// Path-key didn't match; recovered via Levenshtein content-similarity
    /// against known lockfile jobs.
    RecoveredFromContent {
        /// Similarity score in basis points (0..=10000 = 0.0..=1.0).
        /// See module docs for the f32-vs-bp rationale.
        similarity_score_bp: u32,
        /// Number of lockfile candidates that scored above threshold.
        candidate_count: u32,
    },
    /// User explicitly confirmed this identity (e.g., via interactive
    /// rename-dialog). Highest trust over content-recovery.
    UserConfirmed,
    /// Neither path nor content matched any known job — marker for UI/Tool
    /// to ask the user.
    Uncertain,
}

impl JobIdentityOrigin {
    /// Convert an f32 similarity score (0.0..=1.0) to basis points.
    /// Clamps to the valid range; `NaN` and infinities map to 0.
    #[must_use]
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "scaled is checked finite, non-negative, and < 10_000 before the cast"
    )]
    pub fn score_to_bp(score: f32) -> u32 {
        if !score.is_finite() || score < 0.0 {
            return 0;
        }
        let scaled = (score * 10_000.0).round();
        if scaled >= 10_000.0 {
            10_000
        } else {
            scaled as u32
        }
    }

    /// Convert basis-points back to an f32 in `0.0..=1.0`.
    #[must_use]
    pub fn bp_to_score(bp: u32) -> f32 {
        f32::from(u16::try_from(bp.min(10_000)).unwrap_or(10_000)) / 10_000.0
    }
}

impl JobIdentity {
    /// Construct a `JobIdentity` with `FreshlyAssigned` origin.
    #[must_use]
    pub fn fresh(uuid: impl Into<String>) -> Self {
        Self {
            uuid: uuid.into(),
            origin: JobIdentityOrigin::FreshlyAssigned,
        }
    }

    /// Construct a `JobIdentity` loaded from a lockfile path-key match.
    #[must_use]
    pub fn from_lockfile(uuid: impl Into<String>) -> Self {
        Self {
            uuid: uuid.into(),
            origin: JobIdentityOrigin::FromLockfile,
        }
    }

    /// Construct a `JobIdentity` recovered via content-similarity.
    /// `score` is an f32 in 0.0..=1.0, internally stored as basis-points.
    #[must_use]
    pub fn recovered(uuid: impl Into<String>, score: f32, candidates: u32) -> Self {
        Self {
            uuid: uuid.into(),
            origin: JobIdentityOrigin::RecoveredFromContent {
                similarity_score_bp: JobIdentityOrigin::score_to_bp(score),
                candidate_count: candidates,
            },
        }
    }

    /// Construct an `Uncertain` `JobIdentity` (marker — UI/Tool should
    /// ask the user or surface the ambiguity).
    #[must_use]
    pub fn uncertain(uuid: impl Into<String>) -> Self {
        Self {
            uuid: uuid.into(),
            origin: JobIdentityOrigin::Uncertain,
        }
    }
}
