//! Source-position metadata per IR value.
//!
//! Every IR value that originated from a CST source carries its
//! byte-range + line/column position. Pflicht per Hub-IR v0.1 §7.

use serde::{Deserialize, Serialize};

/// Source-position metadata for an IR value.
///
/// `defined_by_reference` is reserved for M4 (resolution of `extends:`
/// / `include:`) — always `None` in M3.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Provenance {
    pub source_file: String,
    pub range: SourceRange,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub defined_by_reference: Option<Box<Provenance>>,
}

/// Byte-range and 1-indexed line/column in the source.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SourceRange {
    pub byte_start: usize,
    pub byte_end: usize,
    pub line_start: u32,
    pub col_start: u32,
    pub line_end: u32,
    pub col_end: u32,
}

impl Provenance {
    /// Build a `Provenance` from a byte-span over the given source.
    ///
    /// Computes 1-indexed line and column by scanning newlines.
    /// `defined_by_reference` is set to `None`.
    #[must_use]
    pub fn from_byte_span(source_file: &str, source: &str, span: (usize, usize)) -> Self {
        let (line_start, col_start) = byte_to_line_col(source, span.0);
        let (line_end, col_end) = byte_to_line_col(source, span.1);
        Self {
            source_file: source_file.to_string(),
            range: SourceRange {
                byte_start: span.0,
                byte_end: span.1,
                line_start,
                col_start,
                line_end,
                col_end,
            },
            defined_by_reference: None,
        }
    }
}

/// Convert a byte offset to (line, col), both 1-indexed.
///
/// Lines are separated by `\n`. The column at byte `i` is the count of
/// chars since the last newline plus one. For `i == 0` returns `(1, 1)`.
fn byte_to_line_col(source: &str, byte: usize) -> (u32, u32) {
    let mut line: u32 = 1;
    let mut col: u32 = 1;
    for (i, c) in source.char_indices() {
        if i >= byte {
            break;
        }
        if c == '\n' {
            line += 1;
            col = 1;
        } else {
            col += 1;
        }
    }
    (line, col)
}
