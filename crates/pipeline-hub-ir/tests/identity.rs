//! Tests for `JobIdentity` constructors, equality, hashing, and serde.

use pipeline_hub_ir::{JobIdentity, JobIdentityOrigin};
use pretty_assertions::assert_eq;
use std::collections::HashSet;

#[test]
fn fresh_constructor_sets_origin() {
    let id = JobIdentity::fresh("uuid-1");
    assert_eq!(id.uuid, "uuid-1");
    assert_eq!(id.origin, JobIdentityOrigin::FreshlyAssigned);
}

#[test]
fn from_lockfile_constructor() {
    let id = JobIdentity::from_lockfile("uuid-2");
    assert_eq!(id.origin, JobIdentityOrigin::FromLockfile);
}

#[test]
fn recovered_constructor_stores_score_as_basis_points() {
    let id = JobIdentity::recovered("uuid-3", 0.82, 1);
    match id.origin {
        JobIdentityOrigin::RecoveredFromContent {
            similarity_score_bp,
            candidate_count,
        } => {
            assert_eq!(similarity_score_bp, 8200);
            assert_eq!(candidate_count, 1);
        }
        _ => panic!("expected RecoveredFromContent"),
    }
}

#[test]
fn score_to_bp_clamps_above_one() {
    assert_eq!(JobIdentityOrigin::score_to_bp(1.5), 10_000);
}

#[test]
fn score_to_bp_clamps_below_zero() {
    assert_eq!(JobIdentityOrigin::score_to_bp(-0.5), 0);
}

#[test]
fn score_to_bp_handles_nan() {
    assert_eq!(JobIdentityOrigin::score_to_bp(f32::NAN), 0);
}

#[test]
fn score_to_bp_handles_infinity() {
    assert_eq!(JobIdentityOrigin::score_to_bp(f32::INFINITY), 0);
}

#[test]
fn score_to_bp_rounds_to_nearest() {
    assert_eq!(JobIdentityOrigin::score_to_bp(0.700_05), 7001);
    assert_eq!(JobIdentityOrigin::score_to_bp(0.700_04), 7000);
}

#[test]
fn bp_to_score_inverts_score_to_bp() {
    let cases = [0.0_f32, 0.7, 0.25, 0.999, 1.0];
    for s in cases {
        let bp = JobIdentityOrigin::score_to_bp(s);
        let back = JobIdentityOrigin::bp_to_score(bp);
        assert!(
            (back - s).abs() < 1e-3,
            "round-trip failed for {s}: bp={bp} back={back}"
        );
    }
}

#[test]
fn uncertain_constructor() {
    let id = JobIdentity::uncertain("uuid-4");
    assert_eq!(id.origin, JobIdentityOrigin::Uncertain);
}

#[test]
fn equal_uuids_with_same_origin_are_equal() {
    let a = JobIdentity::fresh("x");
    let b = JobIdentity::fresh("x");
    assert_eq!(a, b);
}

#[test]
fn equal_uuids_with_different_origin_are_not_equal() {
    let a = JobIdentity::fresh("x");
    let b = JobIdentity::from_lockfile("x");
    assert_ne!(a, b);
}

#[test]
fn job_identity_is_hashable() {
    let mut set: HashSet<JobIdentity> = HashSet::new();
    set.insert(JobIdentity::fresh("a"));
    set.insert(JobIdentity::fresh("b"));
    set.insert(JobIdentity::fresh("a")); // duplicate
    assert_eq!(set.len(), 2);
}

#[test]
fn serde_round_trip_fresh() {
    let id = JobIdentity::fresh("uuid-1");
    let json = serde_json::to_string(&id).expect("ser");
    let round: JobIdentity = serde_json::from_str(&json).expect("de");
    assert_eq!(id, round);
}

#[test]
fn serde_round_trip_all_origin_variants() {
    let cases = vec![
        JobIdentity::fresh("a"),
        JobIdentity::from_lockfile("b"),
        JobIdentity::recovered("c", 0.91, 3),
        JobIdentity {
            uuid: "d".into(),
            origin: JobIdentityOrigin::UserConfirmed,
        },
        JobIdentity::uncertain("e"),
    ];
    for id in &cases {
        let json = serde_json::to_string(id).expect("ser");
        let round: JobIdentity = serde_json::from_str(&json).expect("de");
        assert_eq!(id, &round);
    }
}
