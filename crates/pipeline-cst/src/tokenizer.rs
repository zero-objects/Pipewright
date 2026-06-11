//! YAML tokenizer for GitLab pipelines.
//!
//! Block-style focus (99% of GitLab pipelines). Flow-style `[]`/`{}`
//! is captured as opaque inline scalar (bytes preserved, structure
//! inside not analyzed).

use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Span {
    pub start: usize,
    pub end: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Token {
    /// Leading whitespace count of a line.
    Indent(usize),
    /// `key:` — span is the key text only.
    MappingKey { key_span: Span },
    /// `-` sequence-item indicator (with following space already consumed).
    SequenceDash,
    /// Scalar value at any position.
    Scalar { span: Span, style: ScalarStyle },
    /// `&name` (followed by space, consumed).
    Anchor { name_span: Span },
    /// `*name`.
    Alias { name_span: Span },
    /// `!name` (followed by space, consumed).
    Tag { name_span: Span },
    /// `# ...` comment.
    Comment { span: Span, leading: bool },
    /// `\n`.
    Newline,
    /// End of stream.
    Eof,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ScalarStyle {
    Plain,
    SingleQuoted,
    DoubleQuoted,
    Literal,
    Folded,
    FlowList,
    FlowMap,
}

#[derive(Debug, Error)]
pub enum TokenizerError {
    #[error("unterminated string starting at {0}")]
    UnterminatedString(usize),
    #[error("unterminated flow {kind:?} starting at {start}")]
    UnterminatedFlow { kind: char, start: usize },
}

#[allow(
    clippy::too_many_lines,
    reason = "single-state-machine tokenizer; splitting hurts readability more than it helps"
)]
pub fn tokenize(source: &str) -> Result<Vec<Token>, TokenizerError> {
    let mut tokens = Vec::new();
    let bytes = source.as_bytes();
    let mut i: usize = 0;
    let mut at_line_start = true;

    while i < bytes.len() {
        if at_line_start {
            let indent_end = scan_indent(bytes, i);
            tokens.push(Token::Indent(indent_end - i));
            i = indent_end;
            at_line_start = false;
            if i >= bytes.len() {
                break;
            }
        }
        // Skip inline whitespace between tokens. Without this, the
        // default arm runs `scan_plain_or_key` on a leading space and
        // emits a spurious empty Scalar token — which then breaks the
        // surrounding mapping/sequence parse. Triggered in practice
        // by patterns like `KEY: "value"  # comment` where the two
        // spaces between the quoted scalar and the `#` got tokenised.
        if matches!(bytes[i], b' ' | b'\t') {
            i += 1;
            continue;
        }
        let b = bytes[i];
        match b {
            b'\n' => {
                tokens.push(Token::Newline);
                i += 1;
                at_line_start = true;
            }
            b'#' => {
                let start = i;
                while i < bytes.len() && bytes[i] != b'\n' {
                    i += 1;
                }
                let leading = matches!(tokens.last(), Some(Token::Indent(_)));
                tokens.push(Token::Comment {
                    span: Span { start, end: i },
                    leading,
                });
            }
            b'-' if i + 1 < bytes.len() && matches!(bytes[i + 1], b' ' | b'\n') => {
                tokens.push(Token::SequenceDash);
                i += 1;
                if i < bytes.len() && bytes[i] == b' ' {
                    i += 1;
                }
            }
            b'&' => {
                let start = i + 1;
                i += 1;
                while i < bytes.len() && is_ident_byte(bytes[i]) {
                    i += 1;
                }
                tokens.push(Token::Anchor {
                    name_span: Span { start, end: i },
                });
                if i < bytes.len() && bytes[i] == b' ' {
                    i += 1;
                }
            }
            b'*' => {
                let start = i + 1;
                i += 1;
                while i < bytes.len() && is_ident_byte(bytes[i]) {
                    i += 1;
                }
                tokens.push(Token::Alias {
                    name_span: Span { start, end: i },
                });
            }
            b'!' => {
                let start = i;
                i += 1;
                // Tag identifier: until space, newline, or `[`/`{` (flow open).
                while i < bytes.len() && !matches!(bytes[i], b' ' | b'\n' | b'[' | b'{') {
                    i += 1;
                }
                tokens.push(Token::Tag {
                    name_span: Span { start, end: i },
                });
                if i < bytes.len() && bytes[i] == b' ' {
                    i += 1;
                }
            }
            b'\'' | b'"' => {
                let style = if b == b'\'' {
                    ScalarStyle::SingleQuoted
                } else {
                    ScalarStyle::DoubleQuoted
                };
                let start = i;
                let quote = b;
                i += 1;
                while i < bytes.len() && bytes[i] != quote {
                    // Handle escape in double-quoted only.
                    if quote == b'"' && bytes[i] == b'\\' && i + 1 < bytes.len() {
                        i += 2;
                        continue;
                    }
                    i += 1;
                }
                if i >= bytes.len() {
                    return Err(TokenizerError::UnterminatedString(start));
                }
                i += 1;
                // Promote to a mapping key when the next non-space
                // char is `:` followed by space/newline/EOF — YAML
                // permits quoted-string keys (`"weird: name":`).
                let mut peek = i;
                while peek < bytes.len() && bytes[peek] == b' ' {
                    peek += 1;
                }
                let is_key = peek < bytes.len()
                    && bytes[peek] == b':'
                    && (peek + 1 >= bytes.len() || matches!(bytes[peek + 1], b' ' | b'\n'));
                if is_key {
                    // Drop the outer quote pair so downstream
                    // consumers see the logical key text. Escapes
                    // inside the quotes are kept verbatim — we
                    // don't yet need to unescape mapping keys.
                    tokens.push(Token::MappingKey {
                        key_span: Span {
                            start: start + 1,
                            end: i - 1,
                        },
                    });
                    i = peek + 1;
                    if i < bytes.len() && bytes[i] == b' ' {
                        i += 1;
                    }
                    continue;
                }
                tokens.push(Token::Scalar {
                    span: Span { start, end: i },
                    style,
                });
            }
            b'|' | b'>' if at_value_position(&tokens) => {
                let style = if b == b'|' {
                    ScalarStyle::Literal
                } else {
                    ScalarStyle::Folded
                };
                let start = i;
                let parent_indent = current_indent(&tokens);
                i = scan_block_scalar(bytes, i, parent_indent);
                tokens.push(Token::Scalar {
                    span: Span { start, end: i },
                    style,
                });
                // The block-scalar scan stops AT the start of the
                // terminating dedented line, having consumed the
                // preceding newline. The next loop iteration must
                // treat this as a fresh line so the Indent-token of
                // the dedented line is emitted and subsequent keys
                // don't include leading whitespace in their spans.
                at_line_start = true;
            }
            b'[' => {
                let start = i;
                i = scan_flow_until(bytes, i, b']')?;
                tokens.push(Token::Scalar {
                    span: Span { start, end: i },
                    style: ScalarStyle::FlowList,
                });
            }
            b'{' => {
                let start = i;
                i = scan_flow_until(bytes, i, b'}')?;
                tokens.push(Token::Scalar {
                    span: Span { start, end: i },
                    style: ScalarStyle::FlowMap,
                });
            }
            _ => {
                let (tok, new_i) = scan_plain_or_key(bytes, i);
                tokens.push(tok);
                i = new_i;
            }
        }
    }
    tokens.push(Token::Eof);
    Ok(tokens)
}

fn scan_indent(bytes: &[u8], start: usize) -> usize {
    let mut i = start;
    while i < bytes.len() && (bytes[i] == b' ' || bytes[i] == b'\t') {
        i += 1;
    }
    i
}

fn is_ident_byte(b: u8) -> bool {
    matches!(b, b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'_' | b'-' | b'.')
}

fn scan_plain_or_key(bytes: &[u8], start: usize) -> (Token, usize) {
    let mut i = start;
    let mut last_non_space = start;
    while i < bytes.len() && bytes[i] != b'\n' && bytes[i] != b'#' {
        // Check for `:` that terminates a key (followed by space/newline/EOF).
        if bytes[i] == b':' && (i + 1 >= bytes.len() || matches!(bytes[i + 1], b' ' | b'\n')) {
            let key_end = last_non_space + 1;
            let mut after = i + 1;
            if after < bytes.len() && bytes[after] == b' ' {
                after += 1;
            }
            return (
                Token::MappingKey {
                    key_span: Span {
                        start,
                        end: key_end,
                    },
                },
                after,
            );
        }
        if bytes[i] != b' ' && bytes[i] != b'\t' {
            last_non_space = i;
        }
        i += 1;
    }
    let end = last_non_space + 1;
    (
        Token::Scalar {
            span: Span { start, end },
            style: ScalarStyle::Plain,
        },
        i,
    )
}

fn scan_flow_until(bytes: &[u8], start: usize, closing: u8) -> Result<usize, TokenizerError> {
    let mut i = start + 1;
    let opener = bytes[start];
    let mut depth = 1usize;
    while i < bytes.len() && depth > 0 {
        match bytes[i] {
            b'[' | b'{' => depth += 1,
            b']' | b'}' => {
                depth -= 1;
                if depth == 0 {
                    i += 1;
                    return Ok(i);
                }
            }
            b'\'' | b'"' => {
                // Skip embedded strings.
                let quote = bytes[i];
                i += 1;
                while i < bytes.len() && bytes[i] != quote {
                    if quote == b'"' && bytes[i] == b'\\' && i + 1 < bytes.len() {
                        i += 2;
                        continue;
                    }
                    i += 1;
                }
                if i >= bytes.len() {
                    return Err(TokenizerError::UnterminatedString(i));
                }
            }
            _ => {}
        }
        i += 1;
    }
    if depth > 0 {
        let _ = closing;
        return Err(TokenizerError::UnterminatedFlow {
            kind: opener as char,
            start,
        });
    }
    Ok(i)
}

fn at_value_position(tokens: &[Token]) -> bool {
    // We're at a value position if the previous non-trivial token is a
    // MappingKey or a SequenceDash. Indent doesn't count.
    for t in tokens.iter().rev() {
        match t {
            Token::Indent(_) | Token::Comment { .. } => {}
            Token::MappingKey { .. }
            | Token::SequenceDash
            | Token::Anchor { .. }
            | Token::Tag { .. } => return true,
            _ => return false,
        }
    }
    false
}

fn current_indent(tokens: &[Token]) -> usize {
    // Walk back to the nearest Indent. The dash-offset only counts
    // when we're inside an inline mapping (`- key: value`) — there
    // the key sits two columns past the dash, so siblings keyed at
    // that column must end a block scalar's body. For pure
    // sequence-item value blocks (`- |\n  body`) the parent indent
    // IS the dash column; otherwise body lines two columns deeper
    // get eaten as if the dash were just whitespace.
    let mut saw_key_before_dash = false;
    let mut crossed_dash = false;
    for t in tokens.iter().rev() {
        match t {
            Token::Indent(n) => {
                return *n
                    + if crossed_dash && saw_key_before_dash {
                        2
                    } else {
                        0
                    };
            }
            Token::SequenceDash => crossed_dash = true,
            Token::MappingKey { .. } => {
                if !crossed_dash {
                    saw_key_before_dash = true;
                }
            }
            _ => {}
        }
    }
    0
}

fn scan_block_scalar(bytes: &[u8], start: usize, parent_indent: usize) -> usize {
    let mut i = start;
    // Consume the indicator line up to and including its newline.
    while i < bytes.len() && bytes[i] != b'\n' {
        i += 1;
    }
    if i >= bytes.len() {
        return i;
    }
    i += 1; // skip the newline
            // Block continues while indented MORE than parent_indent (or blank).
    loop {
        if i >= bytes.len() {
            break;
        }
        let line_start = i;
        let mut k = i;
        while k < bytes.len() && (bytes[k] == b' ' || bytes[k] == b'\t') {
            k += 1;
        }
        let line_indent = k - line_start;
        let is_blank = k >= bytes.len() || bytes[k] == b'\n';
        if !is_blank && line_indent <= parent_indent {
            break;
        }
        // Consume the line.
        while i < bytes.len() && bytes[i] != b'\n' {
            i += 1;
        }
        if i < bytes.len() {
            i += 1; // include newline
        }
    }
    i
}
