//! Tests for `Condition` AST construction and serde round-trip.

use pipeline_hub_ir::conditions::{CompareOp, Condition, Operand, VariableRef};
use pretty_assertions::assert_eq;

#[test]
fn variable_ref_constructors() {
    let u = VariableRef::unscoped("CI_DEBUG");
    assert_eq!(u.name, "CI_DEBUG");
    assert!(u.scope.is_none());

    let s = VariableRef::scoped("BUILD_NUM", "build_stage");
    assert_eq!(s.name, "BUILD_NUM");
    assert_eq!(s.scope.as_deref(), Some("build_stage"));
}

#[test]
fn eq_compare_construction() {
    let c = Condition::eq(
        Operand::Predefined("CI_COMMIT_BRANCH".into()),
        Operand::Literal("main".into()),
    );
    match c {
        Condition::Compare { op, .. } => assert_eq!(op, CompareOp::Eq),
        _ => panic!("expected Compare"),
    }
}

#[test]
fn nested_and_or_round_trip() {
    let c = Condition::or(
        Condition::and(
            Condition::eq(
                Operand::Predefined("CI_COMMIT_BRANCH".into()),
                Operand::Literal("main".into()),
            ),
            Condition::ne(
                Operand::Var(VariableRef::unscoped("FOO")),
                Operand::Literal("skip".into()),
            ),
        ),
        Condition::negate(Condition::defined(VariableRef::unscoped("NIGHTLY"))),
    );
    let json = serde_json::to_string(&c).expect("ser");
    let round: Condition = serde_json::from_str(&json).expect("de");
    assert_eq!(c, round);
}

#[test]
fn regex_match_round_trip() {
    let c = Condition::matches(
        Operand::Predefined("CI_COMMIT_TAG".into()),
        "^v[0-9]+\\.[0-9]+\\.[0-9]+$",
    );
    let json = serde_json::to_string(&c).expect("ser");
    let round: Condition = serde_json::from_str(&json).expect("de");
    assert_eq!(c, round);
}

#[test]
fn platform_opaque_preserves_source() {
    let c = Condition::opaque("github", "github.event.pull_request.draft == false");
    let json = serde_json::to_string(&c).expect("ser");
    let round: Condition = serde_json::from_str(&json).expect("de");
    assert_eq!(c, round);
    if let Condition::PlatformOpaque { platform, expr_src } = round {
        assert_eq!(platform, "github");
        assert_eq!(expr_src, "github.event.pull_request.draft == false");
    } else {
        panic!("expected PlatformOpaque");
    }
}

#[test]
fn operand_event_field_round_trip() {
    let c = Condition::eq(
        Operand::EventField("github.event.pull_request.draft".into()),
        Operand::Literal("false".into()),
    );
    let json = serde_json::to_string(&c).expect("ser");
    let round: Condition = serde_json::from_str(&json).expect("de");
    assert_eq!(c, round);
}

#[test]
fn all_compare_ops_round_trip() {
    for op in [
        CompareOp::Eq,
        CompareOp::Neq,
        CompareOp::Lt,
        CompareOp::Lte,
        CompareOp::Gt,
        CompareOp::Gte,
    ] {
        let c = Condition::Compare {
            lhs: Operand::Literal("a".into()),
            op,
            rhs: Operand::Literal("b".into()),
        };
        let json = serde_json::to_string(&c).expect("ser");
        let round: Condition = serde_json::from_str(&json).expect("de");
        assert_eq!(c, round);
    }
}

#[test]
fn defined_round_trip() {
    let c = Condition::defined(VariableRef::unscoped("FEATURE_X"));
    let json = serde_json::to_string(&c).expect("ser");
    let round: Condition = serde_json::from_str(&json).expect("de");
    assert_eq!(c, round);
}

#[test]
fn deep_nesting_round_trip() {
    // (A && (B || !C)) && D
    let c = Condition::and(
        Condition::and(
            Condition::defined(VariableRef::unscoped("A")),
            Condition::or(
                Condition::defined(VariableRef::unscoped("B")),
                Condition::negate(Condition::defined(VariableRef::unscoped("C"))),
            ),
        ),
        Condition::defined(VariableRef::unscoped("D")),
    );
    let json = serde_json::to_string(&c).expect("ser");
    let round: Condition = serde_json::from_str(&json).expect("de");
    assert_eq!(c, round);
}

#[test]
fn variable_ref_serde_omits_none_scope() {
    let v = VariableRef::unscoped("X");
    let json = serde_json::to_string(&v).expect("ser");
    assert!(!json.contains("\"scope\""));
}

#[test]
fn variable_ref_is_hashable() {
    use std::collections::HashSet;
    let mut s = HashSet::new();
    s.insert(VariableRef::unscoped("a"));
    s.insert(VariableRef::unscoped("a"));
    s.insert(VariableRef::unscoped("b"));
    s.insert(VariableRef::scoped("a", "scope1"));
    assert_eq!(s.len(), 3);
}
