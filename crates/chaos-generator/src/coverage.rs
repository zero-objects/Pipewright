//! Rule-coverage instrumentation.
//!
//! Each `cascade_step` records its firing rule's name in
//! [`Cascade::entries`] under `Origin::Rule`. By driving a chaos
//! run and collecting the union of rule names seen, we can tell
//! which rules in the platform's ruleset never got exercised — a
//! direct gap report.
//!
//! Two reasons a rule could be uncovered:
//!  * the rule is dead — generator never produces a fixture that
//!    matches its L-pattern;
//!  * the rule was generated but is actually unreachable (e.g.
//!    a `union` arm that the catalog declares but pipeline-cst
//!    won't ever produce). Reporting both shapes is the point —
//!    we want the chaos suite to find the false-positive rules
//!    too.

use seesaw_core::engine::{cascade_step, Cascade, Rule, TerminationState};
use seesaw_core::graph::TypedGraph;
use seesaw_core::ops::Origin;
use seesaw_core::rule::compile::compile;
use seesaw_core::rule::instantiate::instantiate;
use seesaw_core::rule::spec::RuleSetSpec;
use std::collections::BTreeSet;

/// Run a cascade to completion and return the names of every rule
/// that fired at least once.
pub fn run_and_record(
    g: &mut TypedGraph,
    ruleset: &RuleSetSpec,
    budget: usize,
) -> BTreeSet<String> {
    let compiled: Vec<_> = ruleset
        .rules
        .iter()
        .map(|r| compile(r).expect("compile"))
        .collect();
    let rules: Vec<Box<dyn Rule>> = compiled.iter().map(instantiate).collect();
    let refs: Vec<&dyn Rule> = rules.iter().map(AsRef::as_ref).collect();
    let mut cascade = Cascade::new();
    let mut seen = BTreeSet::new();
    let mut last = 0usize;
    for _ in 0..budget {
        match cascade_step(&mut cascade, g, &refs).expect("cascade") {
            TerminationState::Running => {
                while last < cascade.entries.len() {
                    if let Origin::Rule { rule_id } = &cascade.entries[last].origin {
                        seen.insert(rule_id.clone());
                    }
                    last += 1;
                }
            }
            _ => {
                while last < cascade.entries.len() {
                    if let Origin::Rule { rule_id } = &cascade.entries[last].origin {
                        seen.insert(rule_id.clone());
                    }
                    last += 1;
                }
                break;
            }
        }
    }
    seen
}

/// Total rule set for a platform (every rule in the ruleset, by
/// name).
pub fn all_rule_names(ruleset: &RuleSetSpec) -> BTreeSet<String> {
    ruleset.rules.iter().map(|r| r.name.clone()).collect()
}

/// Aggregate coverage across multiple cascade runs. Useful when
/// the same ruleset is exercised by many chaos seeds — `merge`
/// folds the per-seed sets into a single "ever-seen" set.
pub fn merge(into: &mut BTreeSet<String>, from: BTreeSet<String>) {
    into.extend(from);
}

/// `(seen, all)` → list of rules that never fired.
pub fn unseen(seen: &BTreeSet<String>, all: &BTreeSet<String>) -> Vec<String> {
    all.difference(seen).cloned().collect()
}
