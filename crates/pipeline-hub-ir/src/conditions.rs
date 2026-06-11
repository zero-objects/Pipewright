//! `Condition` AST per spec §4 — platform-neutral expression structure.
//!
//! Hub-IR normalises every platform's condition language (GitLab
//! `rules:if`, GitHub `if:`, Azure `condition:`, Jenkins `when {}`)
//! into this common AST. Expressions that cannot be normalised are
//! preserved as `PlatformOpaque(platform, expr_src)` so round-trip
//! still works and the M4 evaluator can mark them
//! "not statically evaluable".

use serde::{Deserialize, Serialize};

/// Platform-neutral condition AST. Spec §4.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Condition {
    And(Box<Condition>, Box<Condition>),
    Or(Box<Condition>, Box<Condition>),
    Not(Box<Condition>),
    Compare {
        lhs: Operand,
        op: CompareOp,
        rhs: Operand,
    },
    Match {
        lhs: Operand,
        regex: String,
    },
    Defined {
        var: VariableRef,
    },
    /// Catch-all: expression not normalisable into the structured
    /// variants above. Holds the original platform tag and source
    /// string for round-trip and human inspection.
    PlatformOpaque {
        platform: String,
        expr_src: String,
    },
}

/// Operand on either side of a `Compare` or `Match`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Operand {
    Literal(String),
    Var(VariableRef),
    /// Platform-predefined variable (e.g., `CI_COMMIT_BRANCH`,
    /// `github.event.pull_request.draft`). Name is verbatim.
    Predefined(String),
    /// Field of the trigger event payload. `json_path` is a dotted
    /// path (e.g., `github.event.pull_request.draft`).
    EventField(String),
}

/// Reference to a user-defined variable. `scope` is optional; when
/// `None`, scope is resolved by surrounding job/pipeline lookup
/// (M4 resolver).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct VariableRef {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
}

impl VariableRef {
    /// Construct a `VariableRef` with no explicit scope.
    #[must_use]
    pub fn unscoped(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            scope: None,
        }
    }

    /// Construct a `VariableRef` with explicit scope id.
    #[must_use]
    pub fn scoped(name: impl Into<String>, scope: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            scope: Some(scope.into()),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum CompareOp {
    Eq,
    Neq,
    Lt,
    Lte,
    Gt,
    Gte,
}

impl Condition {
    /// Combine two conditions with logical AND.
    #[must_use]
    pub fn and(lhs: Condition, rhs: Condition) -> Self {
        Condition::And(Box::new(lhs), Box::new(rhs))
    }

    /// Combine two conditions with logical OR.
    #[must_use]
    pub fn or(lhs: Condition, rhs: Condition) -> Self {
        Condition::Or(Box::new(lhs), Box::new(rhs))
    }

    /// Negate a condition.
    #[must_use]
    pub fn negate(inner: Condition) -> Self {
        Condition::Not(Box::new(inner))
    }

    /// Construct an equality comparison.
    #[must_use]
    pub fn eq(lhs: Operand, rhs: Operand) -> Self {
        Condition::Compare {
            lhs,
            op: CompareOp::Eq,
            rhs,
        }
    }

    /// Construct an inequality comparison.
    #[must_use]
    pub fn ne(lhs: Operand, rhs: Operand) -> Self {
        Condition::Compare {
            lhs,
            op: CompareOp::Neq,
            rhs,
        }
    }

    /// Construct a regex match (`=~ /pattern/`).
    #[must_use]
    pub fn matches(lhs: Operand, regex: impl Into<String>) -> Self {
        Condition::Match {
            lhs,
            regex: regex.into(),
        }
    }

    /// Construct a defined-check (`$VAR != null` in GitLab, `defined()`
    /// in others).
    #[must_use]
    pub fn defined(var: VariableRef) -> Self {
        Condition::Defined { var }
    }

    /// Construct a platform-opaque marker for unparseable expressions.
    #[must_use]
    pub fn opaque(platform: impl Into<String>, expr_src: impl Into<String>) -> Self {
        Condition::PlatformOpaque {
            platform: platform.into(),
            expr_src: expr_src.into(),
        }
    }
}
