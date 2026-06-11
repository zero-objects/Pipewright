//! pipeline-cst — Comment-preserving YAML CST for GitLab pipelines.
//!
//! Replaces the M0-spike's hand-tokenizer with a properly typed
//! tokenizer + tree. Round-trip byte-identity by construction
//! (parse stores source verbatim; serialize returns it).

pub mod anchor;
pub mod cst;
pub mod merge;
pub mod tag;
pub mod tokenizer;

pub use anchor::AnchorTable;
pub use cst::{parse, serialize, CommentKind, Document, Node, NodeKind, ParseError};
pub use merge::{mapping_entries_logical, top_level_logical, EntrySource, LogicalEntry};
pub use tag::{collect_tags, resolve_tag, ResolvedTag};
pub use tokenizer::{tokenize, ScalarStyle, Span, Token, TokenizerError};
