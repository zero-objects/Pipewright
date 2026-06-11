#![allow(
    clippy::too_many_lines,
    clippy::match_same_arms,
    clippy::doc_markdown,
    reason = "single state-machine tokenizer; arms are kept distinct for readability"
)]

//! Jenkinsfile tokenizer — pipeline-DSL subset.
//!
//! The subset covers what's needed to recognise a declarative
//! `pipeline { … }` block: identifiers, single/double-quoted
//! strings, braces, parens, equals, comma, semicolon, comments,
//! line/end-of-stmt boundaries. Numbers are folded into the
//! `Ident` token for now — the parser treats them as plain values.

use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Token {
    pub kind: TokenKind,
    pub span: Span,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Span {
    pub start: usize,
    pub end: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TokenKind {
    /// `[A-Za-z_][A-Za-z0-9_]*` — keyword or identifier.
    Ident(String),
    /// String literal content (the quotes are stripped).
    String(String),
    /// `{`
    LBrace,
    /// `}`
    RBrace,
    /// `(`
    LParen,
    /// `)`
    RParen,
    /// `[` — Groovy list/map/subscript opener. Treated opaquely by
    /// the parser, but needs to be a real token so we can track
    /// depth across newlines (a `[`-opened list spans multiple lines
    /// in real Jenkinsfiles).
    LBracket,
    /// `]`
    RBracket,
    /// `=` (assignment, e.g. inside `environment { K = V }`).
    Eq,
    /// `,`
    Comma,
    /// `;` or end-of-line — both terminate a statement.
    StmtEnd,
    /// `//` line comment.
    LineComment(String),
}

#[derive(Debug, Error)]
pub enum TokenizeError {
    #[error("unterminated string literal starting at byte {start}")]
    UnterminatedString { start: usize },
    #[error("unterminated /* */ comment starting at byte {start}")]
    UnterminatedBlockComment { start: usize },
}

pub fn tokenize(source: &str) -> Result<Vec<Token>, TokenizeError> {
    let bytes = source.as_bytes();
    let mut tokens: Vec<Token> = Vec::new();
    let mut i = 0;
    let mut last_was_stmt_end = true; // suppress leading StmtEnd

    while i < bytes.len() {
        let start = i;
        match bytes[i] {
            // Whitespace (non-newline) — skip.
            b' ' | b'\t' | b'\r' => {
                i += 1;
            }
            // Newline → statement separator (coalesce runs).
            b'\n' => {
                i += 1;
                if !last_was_stmt_end && !tokens.is_empty() {
                    tokens.push(Token {
                        kind: TokenKind::StmtEnd,
                        span: Span { start, end: i },
                    });
                    last_was_stmt_end = true;
                }
            }
            // Explicit `;` → statement end.
            b';' => {
                i += 1;
                if !last_was_stmt_end {
                    tokens.push(Token {
                        kind: TokenKind::StmtEnd,
                        span: Span { start, end: i },
                    });
                    last_was_stmt_end = true;
                }
            }
            // Line comment `// …`
            b'/' if i + 1 < bytes.len() && bytes[i + 1] == b'/' => {
                let mut j = i + 2;
                while j < bytes.len() && bytes[j] != b'\n' {
                    j += 1;
                }
                tokens.push(Token {
                    kind: TokenKind::LineComment(source[i + 2..j].to_string()),
                    span: Span { start: i, end: j },
                });
                i = j;
            }
            // Block comment `/* … */`
            b'/' if i + 1 < bytes.len() && bytes[i + 1] == b'*' => {
                let comment_start = i;
                let mut j = i + 2;
                let mut closed = false;
                while j + 1 < bytes.len() {
                    if bytes[j] == b'*' && bytes[j + 1] == b'/' {
                        j += 2;
                        closed = true;
                        break;
                    }
                    j += 1;
                }
                if !closed {
                    return Err(TokenizeError::UnterminatedBlockComment {
                        start: comment_start,
                    });
                }
                // Treat block comment as whitespace — drop.
                i = j;
            }
            b'{' => {
                tokens.push(Token {
                    kind: TokenKind::LBrace,
                    span: Span {
                        start,
                        end: start + 1,
                    },
                });
                i += 1;
                last_was_stmt_end = true; // an opening brace counts as a fresh-line context
            }
            b'}' => {
                tokens.push(Token {
                    kind: TokenKind::RBrace,
                    span: Span {
                        start,
                        end: start + 1,
                    },
                });
                i += 1;
                last_was_stmt_end = false;
            }
            b'(' => {
                tokens.push(Token {
                    kind: TokenKind::LParen,
                    span: Span {
                        start,
                        end: start + 1,
                    },
                });
                i += 1;
                last_was_stmt_end = false;
            }
            b')' => {
                tokens.push(Token {
                    kind: TokenKind::RParen,
                    span: Span {
                        start,
                        end: start + 1,
                    },
                });
                i += 1;
                last_was_stmt_end = false;
            }
            b'[' => {
                tokens.push(Token {
                    kind: TokenKind::LBracket,
                    span: Span {
                        start,
                        end: start + 1,
                    },
                });
                i += 1;
                last_was_stmt_end = false;
            }
            b']' => {
                tokens.push(Token {
                    kind: TokenKind::RBracket,
                    span: Span {
                        start,
                        end: start + 1,
                    },
                });
                i += 1;
                last_was_stmt_end = false;
            }
            b'=' => {
                tokens.push(Token {
                    kind: TokenKind::Eq,
                    span: Span {
                        start,
                        end: start + 1,
                    },
                });
                i += 1;
                last_was_stmt_end = false;
            }
            b',' => {
                tokens.push(Token {
                    kind: TokenKind::Comma,
                    span: Span {
                        start,
                        end: start + 1,
                    },
                });
                i += 1;
                last_was_stmt_end = false;
            }
            // String: single or double-quoted.
            b'\'' | b'"' => {
                let quote = bytes[i];
                let mut j = i + 1;
                let mut content = String::new();
                while j < bytes.len() && bytes[j] != quote {
                    if bytes[j] == b'\\' && j + 1 < bytes.len() {
                        content.push(bytes[j + 1] as char);
                        j += 2;
                    } else {
                        content.push(bytes[j] as char);
                        j += 1;
                    }
                }
                if j >= bytes.len() {
                    return Err(TokenizeError::UnterminatedString { start: i });
                }
                tokens.push(Token {
                    kind: TokenKind::String(content),
                    span: Span {
                        start: i,
                        end: j + 1,
                    },
                });
                i = j + 1;
                last_was_stmt_end = false;
            }
            // Identifier (or number — folded into Ident text).
            c if is_ident_start(c) => {
                let mut j = i + 1;
                while j < bytes.len() && is_ident_cont(bytes[j]) {
                    j += 1;
                }
                tokens.push(Token {
                    kind: TokenKind::Ident(source[i..j].to_string()),
                    span: Span { start: i, end: j },
                });
                i = j;
                last_was_stmt_end = false;
            }
            // Unknown byte — skip defensively (don't crash the
            // parser on stray punctuation we haven't taught it about).
            _ => {
                i += 1;
            }
        }
    }
    Ok(tokens)
}

fn is_ident_start(b: u8) -> bool {
    b.is_ascii_alphabetic() || b == b'_' || b.is_ascii_digit()
}

fn is_ident_cont(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b == b'.' || b == b'-'
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(toks: &[Token]) -> Vec<&TokenKind> {
        toks.iter().map(|t| &t.kind).collect()
    }

    #[test]
    fn empty_input_yields_no_tokens() {
        assert!(tokenize("").unwrap().is_empty());
    }

    #[test]
    fn pipeline_block_skeleton() {
        let toks = tokenize("pipeline { agent any }").unwrap();
        let kinds: Vec<_> = kinds(&toks);
        assert!(matches!(kinds[0], TokenKind::Ident(s) if s == "pipeline"));
        assert!(matches!(kinds[1], TokenKind::LBrace));
        assert!(matches!(kinds[2], TokenKind::Ident(s) if s == "agent"));
        assert!(matches!(kinds[3], TokenKind::Ident(s) if s == "any"));
        assert!(matches!(kinds.last(), Some(TokenKind::RBrace)));
    }

    #[test]
    fn strings_are_unquoted() {
        let toks = tokenize("sh 'cargo build'").unwrap();
        assert!(matches!(&toks[0].kind, TokenKind::Ident(s) if s == "sh"));
        assert!(matches!(&toks[1].kind, TokenKind::String(s) if s == "cargo build"));
    }

    #[test]
    fn double_quoted_strings() {
        let toks = tokenize("sh \"cargo test\"").unwrap();
        assert!(matches!(&toks[1].kind, TokenKind::String(s) if s == "cargo test"));
    }

    #[test]
    fn newline_becomes_stmt_end() {
        let toks = tokenize("a\nb\n").unwrap();
        assert_eq!(
            toks.iter()
                .filter(|t| matches!(t.kind, TokenKind::StmtEnd))
                .count(),
            2
        );
    }

    #[test]
    fn semicolon_becomes_stmt_end() {
        let toks = tokenize("a; b; ").unwrap();
        assert_eq!(
            toks.iter()
                .filter(|t| matches!(t.kind, TokenKind::StmtEnd))
                .count(),
            2
        );
    }

    #[test]
    fn line_comment_captured() {
        let toks = tokenize("a // hello\n").unwrap();
        assert!(matches!(&toks[1].kind, TokenKind::LineComment(s) if s == " hello"));
    }

    #[test]
    fn block_comment_dropped() {
        let toks = tokenize("a /* x */ b").unwrap();
        let kinds: Vec<_> = kinds(&toks);
        assert_eq!(kinds.len(), 2);
        assert!(matches!(kinds[0], TokenKind::Ident(s) if s == "a"));
        assert!(matches!(kinds[1], TokenKind::Ident(s) if s == "b"));
    }

    #[test]
    fn equals_and_comma() {
        let toks = tokenize("K = 'V', M = 'N'").unwrap();
        let kinds: Vec<_> = kinds(&toks);
        assert!(matches!(kinds[1], TokenKind::Eq));
        assert!(matches!(kinds[3], TokenKind::Comma));
    }

    #[test]
    fn unterminated_string_errors() {
        assert!(matches!(
            tokenize("sh 'no close"),
            Err(TokenizeError::UnterminatedString { .. })
        ));
    }
}
