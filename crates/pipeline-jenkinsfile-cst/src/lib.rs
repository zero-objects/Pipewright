//! Jenkinsfile declarative-pipeline DSL → `pipeline_cst::Document`.
//!
//! Produces the same CST shape YAML does, so the seed + TGG cascade
//! machinery on top stays unchanged. Only the `pipeline-jenkins-tgg`
//! rule-set + extractor will need to understand the Jenkins-specific
//! semantics inside that shared CST shape.

pub mod parse;
pub mod tokenize;

pub use parse::{parse, ParseError};
pub use tokenize::{tokenize, Span, Token, TokenKind, TokenizeError};
