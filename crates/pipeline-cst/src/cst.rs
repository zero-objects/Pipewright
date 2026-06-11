//! CST data + parse/serialize.
//!
//! Round-trip strategy: store source bytes verbatim. The tree lives
//! ALONGSIDE the source, not as a replacement. `serialize` returns
//! the stored bytes; structural queries use the tree.

use thiserror::Error;

use crate::tokenizer::{tokenize, ScalarStyle, Span, Token, TokenizerError};

#[derive(Debug, Error)]
pub enum ParseError {
    #[error("tokenizer error: {0}")]
    Tokenizer(#[from] TokenizerError),
    #[error("unexpected token at byte {pos}: {msg}")]
    UnexpectedToken { pos: usize, msg: String },
}

#[derive(Debug, Clone)]
pub struct Document {
    source: String,
    root: Node,
}

impl Document {
    /// Construct a Document from a source string and a pre-built
    /// root node. Used by alternate front-ends (e.g.
    /// `pipeline-jenkinsfile-cst`) that produce CST trees without
    /// going through the YAML tokenizer.
    #[must_use]
    pub fn from_parts(source: impl Into<String>, root: Node) -> Self {
        Self {
            source: source.into(),
            root,
        }
    }

    #[must_use]
    pub fn source(&self) -> &str {
        &self.source
    }

    #[must_use]
    pub fn root(&self) -> &Node {
        &self.root
    }

    /// Convenience: span text from the source.
    #[must_use]
    pub fn span_text(&self, span: Span) -> &str {
        &self.source[span.start..span.end]
    }
}

#[derive(Debug, Clone)]
pub struct Node {
    pub kind: NodeKind,
    pub span: Span,
    pub children: Vec<Node>,
    pub anchor: Option<String>,
    pub tag: Option<String>,
}

#[derive(Debug, Clone)]
pub enum NodeKind {
    Document,
    Mapping,
    /// Children: [key (Scalar), value (any)].
    MappingEntry {
        key_text: String,
    },
    Sequence,
    SequenceItem,
    Scalar {
        style: ScalarStyle,
    },
    Alias {
        name: String,
    },
    Comment {
        kind: CommentKind,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommentKind {
    FullLine,
    Trailing,
}

pub fn parse(source: &str) -> Result<Document, ParseError> {
    let tokens = tokenize(source).map_err(ParseError::from)?;
    let root = build_document(source, &tokens)?;
    Ok(Document {
        source: source.to_string(),
        root,
    })
}

#[must_use]
pub fn serialize(doc: &Document) -> String {
    doc.source.clone()
}

// ────────────────────────────────────────────────────────────────────
// Tree-builder
//
// Single-pass recursive-descent over the token stream. We track
// indent levels; entering a new mapping/sequence pushes; dedenting
// pops. Comments and Newlines are skipped at the structural level
// but collected at the top-level Document for round-trip queries.

fn build_document(source: &str, tokens: &[Token]) -> Result<Node, ParseError> {
    let mut parser = Parser::new(source, tokens);
    let children = parser.parse_top_level()?;
    Ok(Node {
        kind: NodeKind::Document,
        span: Span {
            start: 0,
            end: source.len(),
        },
        children,
        anchor: None,
        tag: None,
    })
}

struct Parser<'a> {
    source: &'a str,
    tokens: &'a [Token],
    pos: usize,
}

impl<'a> Parser<'a> {
    fn new(source: &'a str, tokens: &'a [Token]) -> Self {
        Self {
            source,
            tokens,
            pos: 0,
        }
    }

    fn peek(&self) -> &Token {
        &self.tokens[self.pos]
    }

    fn advance(&mut self) {
        if self.pos + 1 < self.tokens.len() {
            self.pos += 1;
        }
    }

    /// Promote each `Token::Comment` between two significant
    /// tokens to a `NodeKind::Comment` child of the enclosing
    /// container, in document order. Consumers iterate `.children`
    /// with `if let MappingEntry/...` destructuring, so the extra
    /// children skip past them without changes.
    fn drain_trivia_into(&mut self, dest: &mut Vec<Node>) {
        loop {
            match *self.peek() {
                Token::Comment { span, leading } => {
                    dest.push(Node {
                        kind: NodeKind::Comment {
                            kind: if leading {
                                CommentKind::FullLine
                            } else {
                                CommentKind::Trailing
                            },
                        },
                        span,
                        children: Vec::new(),
                        anchor: None,
                        tag: None,
                    });
                    self.advance();
                }
                Token::Newline | Token::Indent(_) => self.advance(),
                _ => break,
            }
        }
    }

    /// Returns the indent of the current line (the most recent Indent
    /// token before any non-trivia token). 0 if none seen.
    fn current_line_indent(&self) -> usize {
        // Walk backwards to the most recent Indent token, but only
        // through trivia (Indent/Comment/Newline are line-context).
        // For our purposes the immediately-preceding Indent is what
        // matters; trees only enter on a non-trivia token at the start
        // of a line.
        let mut k = self.pos;
        while k > 0 {
            k -= 1;
            match &self.tokens[k] {
                Token::Indent(n) => return *n,
                Token::Newline | Token::Comment { .. } => {}
                _ => return 0,
            }
        }
        0
    }

    /// Top-level: zero-or-more `MappingEntries` OR `SequenceItems` at indent 0.
    fn parse_top_level(&mut self) -> Result<Vec<Node>, ParseError> {
        let mut top = Vec::new();
        self.drain_trivia_into(&mut top);
        if matches!(self.peek(), Token::Eof) {
            return Ok(top);
        }
        match self.peek() {
            Token::MappingKey { .. } => {
                top.push(self.parse_mapping(0)?);
            }
            Token::SequenceDash => {
                top.push(self.parse_sequence(0)?);
            }
            _ => {
                // Single scalar or unknown — treat the whole rest as opaque.
                let start = self.peek_span().map_or(0, |s| s.start);
                let end = self.source.len();
                top.push(Node {
                    kind: NodeKind::Scalar {
                        style: ScalarStyle::Plain,
                    },
                    span: Span { start, end },
                    children: Vec::new(),
                    anchor: None,
                    tag: None,
                });
                self.pos = self.tokens.len() - 1; // skip to EOF
                return Ok(top);
            }
        }
        // Trailing top-level comments (after the root container ends).
        self.drain_trivia_into(&mut top);
        Ok(top)
    }

    fn peek_span(&self) -> Option<Span> {
        match self.peek() {
            Token::MappingKey { key_span } => Some(*key_span),
            Token::Scalar { span, .. } | Token::Comment { span, .. } => Some(*span),
            Token::Anchor { name_span } | Token::Alias { name_span } | Token::Tag { name_span } => {
                Some(*name_span)
            }
            _ => None,
        }
    }

    fn parse_mapping(&mut self, my_indent: usize) -> Result<Node, ParseError> {
        let mapping_start = self.peek_span().map_or(0, |s| s.start);
        let mut entries = Vec::new();
        loop {
            self.drain_trivia_into(&mut entries);
            if matches!(self.peek(), Token::Eof) {
                break;
            }
            let cur_indent = self.current_line_indent();
            if cur_indent < my_indent {
                break;
            }
            if cur_indent > my_indent {
                // Over-indented token that isn't a child of any
                // entry we just parsed: in practice this is a
                // continuation line of a multi-line plain scalar
                // we don't model byte-perfectly (e.g. a long shell
                // command broken across several indented lines).
                // Drop the orphan token rather than aborting the
                // mapping — otherwise every job after the offending
                // line silently disappears.
                self.advance();
                continue;
            }
            match self.peek().clone() {
                Token::MappingKey { key_span } => {
                    let key_text = self.source[key_span.start..key_span.end].to_string();
                    self.advance();
                    // Anchor and Tag tokens after the key get attached to the
                    // value node by parse_value itself.
                    let value = self.parse_value(my_indent)?;
                    let entry_end = value.span.end;
                    let entry = Node {
                        kind: NodeKind::MappingEntry {
                            key_text: key_text.clone(),
                        },
                        span: Span {
                            start: key_span.start,
                            end: entry_end,
                        },
                        children: vec![
                            Node {
                                kind: NodeKind::Scalar {
                                    style: ScalarStyle::Plain,
                                },
                                span: key_span,
                                children: Vec::new(),
                                anchor: None,
                                tag: None,
                            },
                            value,
                        ],
                        anchor: None,
                        tag: None,
                    };
                    entries.push(entry);
                }
                _ => break,
            }
        }
        let end = entries.last().map_or(mapping_start, |e| e.span.end);
        Ok(Node {
            kind: NodeKind::Mapping,
            span: Span {
                start: mapping_start,
                end,
            },
            children: entries,
            anchor: None,
            tag: None,
        })
    }

    fn parse_sequence(&mut self, my_indent: usize) -> Result<Node, ParseError> {
        let seq_start = self.peek_span().map_or(0, |s| s.start);
        let mut items = Vec::new();
        loop {
            self.drain_trivia_into(&mut items);
            if matches!(self.peek(), Token::Eof) {
                break;
            }
            let cur_indent = self.current_line_indent();
            if cur_indent < my_indent {
                break;
            }
            if cur_indent > my_indent && !matches!(self.peek(), Token::SequenceDash) {
                // Continuation of a previous item's multi-line plain
                // scalar that we don't model byte-perfectly. Drop the
                // orphan instead of aborting the sequence — otherwise
                // every sibling item (and everything below) goes
                // silently missing.
                self.advance();
                continue;
            }
            if !matches!(self.peek(), Token::SequenceDash) {
                break;
            }
            // SequenceDash already consumed its trailing space.
            self.advance();
            let item_value = self.parse_value(my_indent)?;
            let item = Node {
                kind: NodeKind::SequenceItem,
                span: item_value.span,
                children: vec![item_value],
                anchor: None,
                tag: None,
            };
            items.push(item);
        }
        let end = items.last().map_or(seq_start, |i| i.span.end);
        Ok(Node {
            kind: NodeKind::Sequence,
            span: Span {
                start: seq_start,
                end,
            },
            children: items,
            anchor: None,
            tag: None,
        })
    }

    /// Expand a plain flow list (`[a, b, c]`) into a block-sequence node of
    /// scalar items. Returns `None` — leaving the caller to keep the opaque
    /// flow scalar — when the content nests another flow collection or contains
    /// quotes, since the simple comma split can't parse those safely. Item
    /// spans point at the trimmed scalar text in the source.
    fn expand_flow_scalar_list(&self, span: Span) -> Option<Node> {
        let text = self.source.get(span.start..span.end)?;
        let inner = text.strip_prefix('[')?.strip_suffix(']')?;
        if inner.contains(['[', ']', '{', '}', '\'', '"']) {
            return None;
        }
        let base = span.start + 1; // first byte after '['
        let mut items = Vec::new();
        let mut offset = 0usize;
        for piece in inner.split(',') {
            let piece_start = base + offset;
            offset += piece.len() + 1; // advance past the piece and its comma
            let trimmed = piece.trim();
            if trimmed.is_empty() {
                continue;
            }
            let lead = piece.len() - piece.trim_start().len();
            let item_span = Span {
                start: piece_start + lead,
                end: piece_start + lead + trimmed.len(),
            };
            let scalar = Node {
                kind: NodeKind::Scalar {
                    style: ScalarStyle::Plain,
                },
                span: item_span,
                children: Vec::new(),
                anchor: None,
                tag: None,
            };
            items.push(Node {
                kind: NodeKind::SequenceItem,
                span: item_span,
                children: vec![scalar],
                anchor: None,
                tag: None,
            });
        }
        if items.is_empty() {
            return None;
        }
        Some(Node {
            kind: NodeKind::Sequence,
            span,
            children: items,
            anchor: None,
            tag: None,
        })
    }

    /// Parse a value at any position. Could be:
    /// - Inline scalar / quoted / block / flow on the same logical line
    /// - Alias `*name`
    /// - A new mapping or sequence on a deeper-indented next line
    fn parse_value(&mut self, parent_indent: usize) -> Result<Node, ParseError> {
        // Check for tag/anchor prefix at value position.
        let mut anchor: Option<String> = None;
        let mut tag: Option<String> = None;
        if let Token::Anchor { name_span } = self.peek().clone() {
            anchor = Some(self.source[name_span.start..name_span.end].to_string());
            self.advance();
        }
        if let Token::Tag { name_span } = self.peek().clone() {
            tag = Some(self.source[name_span.start..name_span.end].to_string());
            self.advance();
        }
        match self.peek().clone() {
            Token::Scalar { span, style } => {
                self.advance();
                // A flow list of plain scalars (`[build, lint]`) is expanded
                // into a real block-sequence node, so downstream sees the same
                // shape as `- build\n- lint` — otherwise the items are lost
                // (seeders only walk Sequence nodes). Flow lists that nest
                // maps/lists (`[{name: x}]`) are left as an opaque scalar: the
                // simple comma split can't parse them and they're rare.
                if style == ScalarStyle::FlowList {
                    if let Some(seq) = self.expand_flow_scalar_list(span) {
                        return Ok(Node { anchor, tag, ..seq });
                    }
                }
                Ok(Node {
                    kind: NodeKind::Scalar { style },
                    span,
                    children: Vec::new(),
                    anchor,
                    tag,
                })
            }
            Token::MappingKey { .. } => {
                // Inline mapping on the same line as the parent dash
                // (e.g. `- local: foo.yml`). We can't use parse_mapping
                // here because its indent-check walks back through the
                // SequenceDash token (which isn't trivia) and returns
                // 0 spuriously. Instead, parse_inline_mapping consumes
                // the first entry unconditionally and uses the
                // first-key's column for subsequent entries.
                let mut n = self.parse_inline_mapping()?;
                n.anchor = anchor.or(n.anchor);
                n.tag = tag.or(n.tag);
                Ok(n)
            }
            Token::Alias { name_span } => {
                let name = self.source[name_span.start..name_span.end].to_string();
                self.advance();
                Ok(Node {
                    kind: NodeKind::Alias { name },
                    span: name_span,
                    children: Vec::new(),
                    anchor,
                    tag,
                })
            }
            Token::Newline | Token::Comment { .. } => {
                // Value is on subsequent indented line(s). Capture
                // any comments sitting between the key and the
                // indented value — they belong to the child
                // container (they sit at the child's indent), so
                // prepend them to its children once we know what
                // it is. Carrier-comments (`# @hub:...`) typically
                // live here.
                let mut leading_comments = Vec::new();
                self.drain_trivia_into(&mut leading_comments);
                let cur_indent = self.current_line_indent();
                // A block SEQUENCE that is a mapping value may sit at the SAME
                // indent as its key — `stages:\n- build` is valid YAML (the
                // dash can't be a sibling mapping entry, so it unambiguously
                // belongs to the key). A block MAPPING value, by contrast, must
                // be more-indented (a same-indent key IS a sibling). So accept
                // a same-indent SequenceDash here; everything else still needs
                // deeper indentation to count as a nested value.
                let same_indent_seq =
                    cur_indent == parent_indent && matches!(self.peek(), Token::SequenceDash);
                if cur_indent > parent_indent || same_indent_seq {
                    match self.peek() {
                        Token::MappingKey { .. } => {
                            let mut n = self.parse_mapping(cur_indent)?;
                            n.anchor = anchor.or(n.anchor);
                            n.tag = tag.or(n.tag);
                            leading_comments.append(&mut n.children);
                            n.children = leading_comments;
                            Ok(n)
                        }
                        Token::SequenceDash => {
                            let mut n = self.parse_sequence(cur_indent)?;
                            n.anchor = anchor.or(n.anchor);
                            n.tag = tag.or(n.tag);
                            leading_comments.append(&mut n.children);
                            n.children = leading_comments;
                            Ok(n)
                        }
                        _ => Ok(empty_scalar(self.peek_span())),
                    }
                } else {
                    // Empty value (e.g., `key:` on its own line with no
                    // indented content below).
                    Ok(empty_scalar(self.peek_span()))
                }
            }
            _ => Ok(empty_scalar(self.peek_span())),
        }
    }

    /// Parse an inline mapping starting at the current `MappingKey`.
    /// Unlike `parse_mapping`, this:
    ///   - consumes the FIRST entry unconditionally (the caller has
    ///     already verified `peek == MappingKey`),
    ///   - uses the first key's column-position to decide when to
    ///     stop accepting subsequent entries (`!=` column → end of
    ///     mapping).
    ///
    /// Used by `parse_value`'s `MappingKey` arm for the
    /// `- key: value\n  another: thing` GitLab pattern, where the
    /// surrounding context (a `SequenceDash`) prevents
    /// `current_line_indent()` from giving us a useful answer.
    fn parse_inline_mapping(&mut self) -> Result<Node, ParseError> {
        let first_key_span = match self.peek() {
            Token::MappingKey { key_span } => *key_span,
            _ => unreachable!("parse_inline_mapping requires MappingKey at peek"),
        };
        let first_col = column_of(self.source, first_key_span.start);
        let mapping_start = first_key_span.start;
        let mut entries = Vec::new();
        let mut mapping_end = mapping_start;

        loop {
            self.drain_trivia_into(&mut entries);
            match self.peek().clone() {
                Token::MappingKey { key_span } => {
                    if entries
                        .iter()
                        .any(|n| matches!(n.kind, NodeKind::MappingEntry { .. }))
                        && column_of(self.source, key_span.start) != first_col
                    {
                        break;
                    }
                    let key_text = self.source[key_span.start..key_span.end].to_string();
                    self.advance();
                    let value = self.parse_value(first_col)?;
                    let entry_end = value.span.end;
                    mapping_end = entry_end;
                    let key_node = Node {
                        kind: NodeKind::Scalar {
                            style: ScalarStyle::Plain,
                        },
                        span: key_span,
                        children: Vec::new(),
                        anchor: None,
                        tag: None,
                    };
                    entries.push(Node {
                        kind: NodeKind::MappingEntry { key_text },
                        span: Span {
                            start: key_span.start,
                            end: entry_end,
                        },
                        children: vec![key_node, value],
                        anchor: None,
                        tag: None,
                    });
                }
                _ => break,
            }
        }

        Ok(Node {
            kind: NodeKind::Mapping,
            span: Span {
                start: mapping_start,
                end: mapping_end,
            },
            children: entries,
            anchor: None,
            tag: None,
        })
    }
}

/// Byte-column of `byte` within its line (0 = column-0).
/// Assumes byte is at a UTF-8 char boundary; treats bytes-since-last-
/// newline as columns (matches YAML's typical ASCII-indent usage).
fn column_of(source: &str, byte: usize) -> usize {
    let line_start = source[..byte].rfind('\n').map_or(0, |i| i + 1);
    byte - line_start
}

fn empty_scalar(at: Option<Span>) -> Node {
    let at = at.unwrap_or(Span { start: 0, end: 0 });
    Node {
        kind: NodeKind::Scalar {
            style: ScalarStyle::Plain,
        },
        span: Span {
            start: at.start,
            end: at.start,
        },
        children: Vec::new(),
        anchor: None,
        tag: None,
    }
}
