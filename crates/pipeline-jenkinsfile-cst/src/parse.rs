#![allow(
    clippy::too_many_lines,
    clippy::match_same_arms,
    clippy::map_unwrap_or,
    clippy::assigning_clones,
    clippy::doc_markdown,
    clippy::similar_names,
    reason = "recursive-descent parser; arm/ident similarity (axis/axes) and clone-assign reads cleaner than the suggested rewrites in this code"
)]

//! Jenkinsfile parser — token stream → `pipeline_cst::Document`.
//!
//! The grammar we accept is a strict subset of the declarative
//! pipeline DSL (see the M8.5 plan-doc). Anything outside that
//! subset is best-effort: the parser tries to produce a sensible
//! CST tree and never panics on input it doesn't recognise — it
//! falls through to a leaf scalar.

use pipeline_cst::{Document, Node, NodeKind, ScalarStyle, Span as CstSpan};
use thiserror::Error;

use crate::tokenize::{tokenize, Token, TokenKind, TokenizeError};

/// Jenkins declarative blocks that are ITEM LISTS (each child is a list
/// element), not directive mappings. Forced to a CST Sequence even when the
/// first element is nameless (`stage { … }`) — content-detection
/// (`block_is_item_list`) catches the paren-call / bare-string forms but not a
/// leading nameless block. Directive blocks (steps/agent/tools/when/post/…)
/// are deliberately absent: they parse as Mappings.
const SEQUENCE_KEYS: &[&str] = &["stages", "parallel", "parameters", "triggers", "libraries"];

#[derive(Debug, Error)]
pub enum ParseError {
    #[error("tokenize: {0}")]
    Tokenize(#[from] TokenizeError),
    #[error("unexpected end of input")]
    UnexpectedEof,
    #[error("expected `{expected}` at byte {pos}, got `{got}`")]
    Expected {
        pos: usize,
        expected: &'static str,
        got: String,
    },
}

/// Parse a Jenkinsfile into a [`pipeline_cst::Document`].
pub fn parse(source: &str) -> Result<Document, ParseError> {
    let tokens = tokenize(source)?;
    let mut p = Parser::new(&tokens);
    let entries = p.parse_statements(0, source.len())?;
    let root_span = CstSpan {
        start: 0,
        end: source.len(),
    };
    let mapping = Node {
        kind: NodeKind::Mapping,
        span: root_span,
        children: entries,
        anchor: None,
        tag: None,
    };
    let document = Node {
        kind: NodeKind::Document,
        span: root_span,
        children: vec![mapping],
        anchor: None,
        tag: None,
    };
    Ok(Document::from_parts(source, document))
}

struct Parser<'a> {
    toks: &'a [Token],
    pos: usize,
}

impl<'a> Parser<'a> {
    fn new(toks: &'a [Token]) -> Self {
        Self { toks, pos: 0 }
    }

    fn peek(&self) -> Option<&Token> {
        self.toks.get(self.pos)
    }

    fn peek_at(&self, n: usize) -> Option<&Token> {
        self.toks.get(self.pos + n)
    }

    /// True when the block we are about to parse is an ITEM LIST rather than a
    /// directive mapping: its first statement is either a paren-call item
    /// (`stage('build') { … }`, `parameter('x') { … }` — IDENT immediately
    /// followed by `(`) or a bare string literal (`libraries { 'lib' … }`).
    /// Both forms render a Sequence; a directive block (`agent { docker { … } }`,
    /// `tools { maven 'M3' }`) stays a Mapping.
    fn block_is_item_list(&self) -> bool {
        match self.peek().map(|t| &t.kind) {
            Some(TokenKind::String(_)) => true,
            Some(TokenKind::Ident(_)) => {
                matches!(self.peek_at(1).map(|t| &t.kind), Some(TokenKind::LParen))
            }
            _ => false,
        }
    }

    fn bump(&mut self) -> Option<&Token> {
        let t = self.toks.get(self.pos);
        if t.is_some() {
            self.pos += 1;
        }
        t
    }

    fn skip_stmt_ends(&mut self) {
        while matches!(
            self.peek().map(|t| &t.kind),
            Some(TokenKind::StmtEnd | TokenKind::LineComment(_))
        ) {
            self.pos += 1;
        }
    }

    /// Parse a list of statements until `}` or end-of-input. Each
    /// statement becomes a CST `MappingEntry` (key + value child).
    fn parse_statements(
        &mut self,
        block_start: usize,
        block_end: usize,
    ) -> Result<Vec<Node>, ParseError> {
        let mut entries = Vec::new();
        self.skip_stmt_ends();
        while let Some(t) = self.peek() {
            if matches!(t.kind, TokenKind::RBrace) {
                break;
            }
            entries.push(self.parse_statement()?);
            self.skip_stmt_ends();
        }
        let _ = (block_start, block_end);
        Ok(entries)
    }

    /// One statement. Recognised shapes:
    ///   IDENT                       → MappingEntry(IDENT, Scalar(""))
    ///   IDENT VALUE                 → MappingEntry(IDENT, Scalar(VALUE))
    ///   IDENT VAL1, VAL2, …         → MappingEntry(IDENT, Scalar spanning all)
    ///   IDENT = VALUE               → MappingEntry(IDENT, Scalar(VALUE))
    ///   IDENT { … }                 → MappingEntry(IDENT, Mapping)
    ///   IDENT(ARG)                  → MappingEntry(IDENT, Scalar(ARG))
    ///   IDENT(ARG) { … }            → MappingEntry(IDENT, Mapping with "name" entry)
    ///   stages { stage(N) { … } … } → MappingEntry(stages, Sequence)
    ///   def IDENT = VALUE           → MappingEntry(IDENT, Scalar(VALUE))
    ///                                 (the `def` keyword is skipped — Groovy
    ///                                 variable declarations like
    ///                                 `def failFast = false`)
    fn parse_statement(&mut self) -> Result<Node, ParseError> {
        let mut key_tok = self.bump().ok_or(ParseError::UnexpectedEof)?.clone();
        let mut key = match &key_tok.kind {
            TokenKind::Ident(s) => s.clone(),
            other => {
                return Err(ParseError::Expected {
                    pos: key_tok.span.start,
                    expected: "identifier",
                    got: format!("{other:?}"),
                });
            }
        };
        // Groovy `def IDENT = EXPR` variable declarations — strip the
        // `def` keyword and treat the next ident as the real key.
        if key == "def" {
            if let Some(t) = self.peek() {
                if let TokenKind::Ident(name) = &t.kind {
                    key = name.clone();
                    key_tok = t.clone();
                    self.bump();
                }
            }
        }
        let stmt_start = key_tok.span.start;

        // Optional arguments in parens: ident(arg, arg, …)
        // Each arg keeps its source-span so the seeder (and any
        // other downstream consumer) can read the original bytes.
        //
        // We track paren depth so nested calls — common in
        // Jenkinsfile setup like
        //   `properties([buildDiscarder(logRotator(numToKeepStr: '50'))])`
        // — don't leak their inner `)` into the outer statement
        // stream. Only depth-0 args are collected; the body of
        // nested calls is opaque to the CST.
        let mut paren_args: Vec<(String, crate::tokenize::Span)> = Vec::new();
        if matches!(self.peek().map(|t| &t.kind), Some(TokenKind::LParen)) {
            self.bump();
            let mut depth: usize = 0;
            while let Some(t) = self.peek() {
                match &t.kind {
                    TokenKind::RParen if depth == 0 => {
                        self.bump();
                        break;
                    }
                    TokenKind::RParen => {
                        depth -= 1;
                        self.bump();
                    }
                    TokenKind::LParen => {
                        depth += 1;
                        self.bump();
                    }
                    TokenKind::Comma => {
                        self.bump();
                    }
                    TokenKind::String(s) | TokenKind::Ident(s) => {
                        if depth == 0 {
                            paren_args.push((s.clone(), t.span));
                        }
                        self.bump();
                    }
                    _ => {
                        self.bump();
                    }
                }
            }
        }

        // `=`?
        let is_assign = matches!(self.peek().map(|t| &t.kind), Some(TokenKind::Eq));
        if is_assign {
            self.bump();
            // Line-continuation after `=` — Groovy lets
            //   def x =
            //     value
            // and especially `def x = [\n  a,\n  b,\n]` (multi-line
            // list literal). Skip newlines so the RHS lookup finds
            // the actual value.
            self.skip_stmt_ends();
        }

        // Block?
        let has_block = matches!(self.peek().map(|t| &t.kind), Some(TokenKind::LBrace));
        if has_block {
            // Consume `{`
            let lbrace = self.bump().unwrap().clone();
            self.skip_stmt_ends();
            // An ITEM-LIST block (`stages { stage('a') { … } … }`,
            // `parameters { parameter('x') { … } }`, `libraries { 'lib' … }`)
            // → a Sequence whose items are the per-element nodes. Decided by
            // the block CONTENT (first statement) so every construct-list field
            // round-trips, not just `stages`; plus an explicit key fallback for
            // the construct-list fields whose first item may be nameless
            // (`stage { … }`), which content-detection alone would miss.
            if SEQUENCE_KEYS.contains(&key.as_str()) || self.block_is_item_list() {
                let mut items: Vec<Node> = Vec::new();
                while let Some(t) = self.peek() {
                    if matches!(t.kind, TokenKind::RBrace) {
                        break;
                    }
                    // A bare string literal is a scalar list item
                    // (`libraries { 'release' }`); anything else is a
                    // paren-call construct item (`stage('a') { … }`).
                    let item_inner = if let TokenKind::String(s) = &t.kind {
                        let (s, span) = (s.clone(), t.span);
                        self.bump();
                        scalar_node(s, span.start, span.end)
                    } else {
                        self.parse_statement()?
                    };
                    let item_span = item_inner.span;
                    items.push(Node {
                        kind: NodeKind::SequenceItem,
                        span: item_span,
                        children: vec![item_inner],
                        anchor: None,
                        tag: None,
                    });
                    self.skip_stmt_ends();
                }
                let rbrace_span = self.expect_rbrace()?;
                let seq_span = CstSpan {
                    start: lbrace.span.start,
                    end: rbrace_span.end,
                };
                let seq = Node {
                    kind: NodeKind::Sequence,
                    span: seq_span,
                    children: items,
                    anchor: None,
                    tag: None,
                };
                return Ok(mapping_entry(key, stmt_start, seq_span.end, seq));
            }

            // General block: a Mapping of nested statements.
            let mut entries = self.parse_statements(lbrace.span.end, 0)?;
            let rbrace_span = self.expect_rbrace()?;
            let mapping_span = CstSpan {
                start: lbrace.span.start,
                end: rbrace_span.end,
            };
            // If the caller had paren-args (e.g. `stage('Build')`),
            // fold the first arg in as a synthetic `name:` entry so
            // the TGG side can read the stage name out of the
            // Mapping like any other key.
            if let Some((name, name_span)) = paren_args.into_iter().next() {
                let name_entry = mapping_entry(
                    "name".to_string(),
                    name_span.start,
                    name_span.end,
                    scalar_node(name, name_span.start, name_span.end),
                );
                entries.insert(0, name_entry);
            }
            let mapping_node = Node {
                kind: NodeKind::Mapping,
                span: mapping_span,
                children: entries,
                anchor: None,
                tag: None,
            };
            return Ok(mapping_entry(
                key,
                stmt_start,
                mapping_span.end,
                mapping_node,
            ));
        }

        // No block: gather the rest of the statement as a scalar
        // value. Take the first ident/string/paren-arg as the value
        // (covers `agent any`, `sh 'cmd'`, `K = 'V'`).
        let mut value_text = String::new();
        let mut value_start = stmt_start;
        let mut value_end = stmt_start;
        if let Some((s, span)) = paren_args.into_iter().next() {
            value_text = s;
            value_start = span.start;
            value_end = span.end;
        } else if let Some(t) = self.peek() {
            match &t.kind {
                TokenKind::String(s) | TokenKind::Ident(s) => {
                    value_text = s.clone();
                    value_start = t.span.start;
                    value_end = t.span.end;
                    self.bump();
                }
                _ => {}
            }
        }
        // Unified trailing-expression absorber — Groovy lets a
        // statement carry a comma-list (`values 'A','B','C'`), a
        // method-call chain (`env.X.replaceAll('a','b')`), a list
        // literal (`def X = [a,b]`), an operator chain
        // (`x = y + 'z'` — operators tokenize as unknown bytes and
        // disappear), and subscript-assigns (`builds[k] = …`). The
        // CST keeps all of these as one opaque scalar span; we just
        // need to consume the bytes so the next statement parses.
        //
        // Stops at structural boundaries *outside* of brackets/parens:
        // StmtEnd / RBrace / LBrace / EOF. Inside `(…)` or `[…]`,
        // newlines are line-continuation and get absorbed.
        let mut paren_depth: usize = 0;
        let mut bracket_depth: usize = 0;
        loop {
            let inside = paren_depth > 0 || bracket_depth > 0;
            let Some(t) = self.peek() else { break };
            let span_end = t.span.end;
            match &t.kind {
                TokenKind::LBrace | TokenKind::RBrace if !inside => break,
                TokenKind::StmtEnd if !inside => break,
                TokenKind::StmtEnd => {
                    self.bump();
                }
                TokenKind::LParen => {
                    paren_depth += 1;
                    value_end = span_end;
                    self.bump();
                }
                TokenKind::RParen => {
                    paren_depth = paren_depth.saturating_sub(1);
                    value_end = span_end;
                    self.bump();
                }
                TokenKind::LBracket => {
                    bracket_depth += 1;
                    value_end = span_end;
                    self.bump();
                }
                TokenKind::RBracket => {
                    bracket_depth = bracket_depth.saturating_sub(1);
                    value_end = span_end;
                    self.bump();
                }
                TokenKind::Comma | TokenKind::Eq => {
                    value_end = span_end;
                    self.bump();
                    // Line-continuation after a separator: if the
                    // following token (across newlines) is value-like,
                    // absorb the StmtEnds; otherwise leave them as
                    // the statement terminator.
                    let save = self.pos;
                    self.skip_stmt_ends();
                    let continues = matches!(
                        self.peek().map(|t| &t.kind),
                        Some(
                            TokenKind::String(_)
                                | TokenKind::Ident(_)
                                | TokenKind::LParen
                                | TokenKind::LBracket
                                | TokenKind::Eq
                                | TokenKind::Comma
                        )
                    );
                    if !continues {
                        self.pos = save;
                    }
                }
                TokenKind::LineComment(_) => {
                    self.bump();
                }
                TokenKind::String(_) | TokenKind::Ident(_) => {
                    value_end = span_end;
                    self.bump();
                }
                // Inside parens/brackets, braces are opaque content
                // (the outer "!inside break" handled the outer case).
                TokenKind::LBrace | TokenKind::RBrace => {
                    value_end = span_end;
                    self.bump();
                }
            }
        }
        // After the scalar/call chain, a block may still follow — e.g.
        // `axes.values().combinations { … }`. Treat it as the
        // statement's body, dropping the intermediate chain text
        // (which is opaque to the CST). Mirrors the `IDENT { … }`
        // branch above.
        if matches!(self.peek().map(|t| &t.kind), Some(TokenKind::LBrace)) {
            let lbrace = self.bump().unwrap().clone();
            let entries = self.parse_statements(lbrace.span.end, 0)?;
            let rbrace_span = self.expect_rbrace()?;
            let mapping_span = CstSpan {
                start: lbrace.span.start,
                end: rbrace_span.end,
            };
            let mapping_node = Node {
                kind: NodeKind::Mapping,
                span: mapping_span,
                children: entries,
                anchor: None,
                tag: None,
            };
            // Suppress `value_text` to silence the unused-var warning;
            // the chain identifier was never going to survive into the
            // emitted CST anyway.
            let _ = (value_text, value_start, value_end);
            return Ok(mapping_entry(
                key,
                stmt_start,
                mapping_span.end,
                mapping_node,
            ));
        }
        let stmt_end = self
            .toks
            .get(self.pos.saturating_sub(1))
            .map(|t| t.span.end)
            .unwrap_or(stmt_start);
        let value = scalar_node(value_text, value_start, value_end);
        Ok(mapping_entry(key, stmt_start, stmt_end, value))
    }

    fn expect_rbrace(&mut self) -> Result<CstSpan, ParseError> {
        match self.bump() {
            Some(Token {
                kind: TokenKind::RBrace,
                span,
            }) => Ok(CstSpan {
                start: span.start,
                end: span.end,
            }),
            Some(t) => Err(ParseError::Expected {
                pos: t.span.start,
                expected: "}",
                got: format!("{:?}", t.kind),
            }),
            None => Err(ParseError::UnexpectedEof),
        }
    }
}

/// Build a `Scalar` CST node. The text isn't stored — readers
/// recover it from the document's source via the span.
fn scalar_node(_text: String, start: usize, end: usize) -> Node {
    Node {
        kind: NodeKind::Scalar {
            style: ScalarStyle::Plain,
        },
        span: CstSpan { start, end },
        children: Vec::new(),
        anchor: None,
        tag: None,
    }
}

fn mapping_entry(key: String, start: usize, end: usize, value: Node) -> Node {
    let key_node = Node {
        kind: NodeKind::Scalar {
            style: ScalarStyle::Plain,
        },
        span: CstSpan {
            start,
            end: start + key.len(),
        },
        children: Vec::new(),
        anchor: None,
        tag: None,
    };
    Node {
        kind: NodeKind::MappingEntry { key_text: key },
        span: CstSpan { start, end },
        children: vec![key_node, value],
        anchor: None,
        tag: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn root_mapping(doc: &Document) -> &Node {
        // Document → first child is the top-level Mapping.
        &doc.root().children[0]
    }

    fn entry<'a>(map: &'a Node, key: &str) -> Option<&'a Node> {
        map.children
            .iter()
            .find(|n| matches!(&n.kind, NodeKind::MappingEntry { key_text } if key_text == key))
    }

    #[test]
    fn parses_minimal_pipeline_block() {
        let src = "pipeline { agent any }";
        let doc = parse(src).expect("parse");
        let root = root_mapping(&doc);
        assert!(matches!(root.kind, NodeKind::Mapping));
        let pipeline = entry(root, "pipeline").expect("pipeline entry");
        let value = &pipeline.children[1];
        assert!(matches!(value.kind, NodeKind::Mapping));
        let agent = entry(value, "agent").expect("agent");
        let agent_val = &agent.children[1];
        assert!(matches!(&agent_val.kind, NodeKind::Scalar { .. }));
    }

    #[test]
    fn stages_block_becomes_sequence() {
        let src = "pipeline {\n  stages {\n    stage('Build') { steps { sh 'cargo build' } }\n    stage('Test') { steps { sh 'cargo test' } }\n  }\n}\n";
        let doc = parse(src).expect("parse");
        let root = root_mapping(&doc);
        let pipeline = entry(root, "pipeline").unwrap();
        let stages = entry(&pipeline.children[1], "stages").unwrap();
        let seq = &stages.children[1];
        assert!(matches!(seq.kind, NodeKind::Sequence));
        assert_eq!(seq.children.len(), 2);
    }

    #[test]
    fn stage_paren_arg_becomes_name_entry() {
        let src = "pipeline { stages { stage('Build') { steps { sh 'go' } } } }";
        let doc = parse(src).expect("parse");
        let stages = entry(
            &entry(root_mapping(&doc), "pipeline").unwrap().children[1],
            "stages",
        )
        .unwrap();
        let seq = &stages.children[1];
        let stage_item = &seq.children[0];
        // SequenceItem → MappingEntry (the `stage` entry).
        let stage_entry = &stage_item.children[0];
        let stage_map = &stage_entry.children[1];
        let name = entry(stage_map, "name").unwrap();
        let name_value = &name.children[1];
        assert!(matches!(&name_value.kind, NodeKind::Scalar { .. }));
        // The scalar's span maps to the brace — the name text is on
        // the parent entry's key. Verify via span_text on the parent:
        // here we just assert the name entry exists.
        let _ = name_value;
    }

    #[test]
    fn sh_step_value_is_string_content() {
        let src = "pipeline { stages { stage('X') { steps { sh 'cargo build' } } } }";
        let doc = parse(src).expect("parse");
        // Drill down: pipeline → stages → seq → item → stage → map → steps → map → sh
        let pipeline = entry(root_mapping(&doc), "pipeline").unwrap();
        let stages = entry(&pipeline.children[1], "stages").unwrap();
        let stage = &stages.children[1].children[0].children[0]; // seq → item → entry
        let stage_map = &stage.children[1];
        let steps = entry(stage_map, "steps").unwrap();
        let steps_map = &steps.children[1];
        let sh = entry(steps_map, "sh").unwrap();
        let sh_val = &sh.children[1];
        assert!(matches!(&sh_val.kind, NodeKind::Scalar { .. }));
        // The scalar span covers the source position of the string token.
        assert_eq!(doc.span_text(sh_val.span), "'cargo build'");
    }

    #[test]
    fn def_keyword_skipped_groovy_decl() {
        // Real-world Jenkinsfiles start with Groovy `def` decls. Prior
        // parser threw "expected identifier, got Eq" at the `=`.
        let src = "def failFast = false\npipeline { agent any }";
        let doc = parse(src).expect("parse def decl");
        let root = root_mapping(&doc);
        // The `def` keyword is stripped; the assign appears as a
        // top-level mapping entry under the declared name.
        let fail_fast = entry(root, "failFast").expect("failFast entry");
        let value = &fail_fast.children[1];
        assert!(matches!(&value.kind, NodeKind::Scalar { .. }));
        assert_eq!(doc.span_text(value.span), "false");
        // Pipeline block still parses after the def decl.
        let pipeline = entry(root, "pipeline").expect("pipeline entry");
        let pipeline_value = &pipeline.children[1];
        assert!(matches!(&pipeline_value.kind, NodeKind::Mapping));
    }

    #[test]
    fn multiline_list_literal_and_operator_chain() {
        // Real Jenkinsfiles use Groovy expressions liberally; prior
        // parser tripped on each operator and list separator.
        let src = "def mavenOptions = [\n  '-Pdebug',\n  '-Penable-jacoco',\n]\ndef tag = 'v' + env.BUILD_TAG + '-final'\npipeline { agent any }";
        let doc = parse(src).expect("parse list + operator chain");
        let root = root_mapping(&doc);
        // Both Groovy lines parse as opaque scalars on their key.
        assert!(entry(root, "mavenOptions").is_some());
        assert!(entry(root, "tag").is_some());
        // And the `pipeline { … }` block after them still works.
        let pipeline = entry(root, "pipeline").expect("pipeline block");
        assert!(matches!(&pipeline.children[1].kind, NodeKind::Mapping));
    }

    #[test]
    fn subscript_assign_block() {
        // `builds["key"] = { closure }` — Jenkins-style closure
        // assignment into a map subscript.
        let src = "builds[\"linux-jdk21\"] = {\n  echo 'go'\n}\npipeline { agent any }";
        let doc = parse(src).expect("parse subscript assign");
        let root = root_mapping(&doc);
        // The statement key keeps the outer name; subscript drops out.
        let builds = entry(root, "builds").expect("builds entry");
        // RHS is the closure, parsed as a Mapping with `echo` inside.
        let body = &builds.children[1];
        assert!(matches!(&body.kind, NodeKind::Mapping));
        assert!(entry(body, "echo").is_some());
        // Trailing pipeline block still parses.
        assert!(entry(root, "pipeline").is_some());
    }

    #[test]
    fn comparison_expression_absorbed() {
        // `params[K] == true` inside `when { expression { … } }` —
        // Groovy comparison with `==` (two Eq tokens) and `[]`
        // subscript. Prior parser threw "expected ident, got Eq".
        let src = "pipeline {\n  stages {\n    stage('s') {\n      when {\n        expression {\n          params[KEY] == true\n        }\n      }\n      steps { sh 'go' }\n    }\n  }\n}";
        let doc = parse(src).expect("parse `==` comparison");
        // The `expression {}` block exists and contains `params` entry.
        let stage = &entry(
            &entry(root_mapping(&doc), "pipeline").unwrap().children[1],
            "stages",
        )
        .unwrap()
        .children[1]
            .children[0]
            .children[0];
        let stage_map = &stage.children[1];
        let when_entry = entry(stage_map, "when").expect("when block");
        let when_map = &when_entry.children[1];
        let expr_entry = entry(when_map, "expression").expect("expression block");
        let expr_map = &expr_entry.children[1];
        assert!(entry(expr_map, "params").is_some());
    }

    #[test]
    fn nested_paren_calls_consume_inner_parens() {
        // Real-world top-of-Jenkinsfile boilerplate. Prior parser
        // broke out at the innermost `)`, leaving outer `)` tokens
        // dangling and throwing "expected identifier, got RParen".
        let src = "properties([buildDiscarder(logRotator(numToKeepStr: '50')), disableConcurrentBuilds(abortPrevious: true)])\npipeline { agent any }";
        let doc = parse(src).expect("parse nested calls");
        let root = root_mapping(&doc);
        // The `properties` entry exists and is opaque (scalar — we
        // don't model the inner call tree).
        assert!(entry(root, "properties").is_some());
        // Crucially, the parser continued past it and saw `pipeline`.
        let pipeline = entry(root, "pipeline").expect("pipeline after properties");
        assert!(matches!(&pipeline.children[1].kind, NodeKind::Mapping));
    }

    #[test]
    fn matrix_axis_values_comma_list() {
        // Jenkins matrix axis `values "A", "B", "C"` — comma-separated
        // strings inside a single statement. Prior parser threw
        // "expected identifier, got Comma" after the first value.
        // Statements need newlines or `;` between them — in Groovy,
        // `name 'E' values 'A'` is ambiguous (one call with two args
        // vs. two calls), and we follow the parser-friendly rule.
        let src = "pipeline {\n  stages {\n    stage('m') {\n      matrix {\n        axes {\n          axis {\n            name 'E'\n            values 'A', 'B', 'C'\n          }\n        }\n        stages {\n          stage('go') {\n            steps { sh 'echo' }\n          }\n        }\n      }\n    }\n  }\n}";
        let doc = parse(src).expect("parse matrix values");
        // Drill down to the axis mapping.
        let pipeline = entry(root_mapping(&doc), "pipeline").unwrap();
        let stages = entry(&pipeline.children[1], "stages").unwrap();
        let stage = &stages.children[1].children[0].children[0];
        let stage_map = &stage.children[1];
        let matrix = entry(stage_map, "matrix").unwrap();
        let matrix_map = &matrix.children[1];
        let axes = entry(matrix_map, "axes").unwrap();
        let axes_map = &axes.children[1];
        let axis = entry(axes_map, "axis").unwrap();
        let axis_map = &axis.children[1];
        let values = entry(axis_map, "values").expect("values entry");
        let values_val = &values.children[1];
        assert!(matches!(&values_val.kind, NodeKind::Scalar { .. }));
        // Span should cover the full comma list (first 'A' through last 'C').
        let text = doc.span_text(values_val.span);
        assert!(
            text.contains("'A'") && text.contains("'C'"),
            "values span should cover all entries, got: {text}"
        );
    }

    #[test]
    fn environment_block_with_assigns() {
        let src = "pipeline { environment { FOO = 'bar' BAZ = 'qux' } }";
        let doc = parse(src).expect("parse");
        let env = entry(
            &entry(root_mapping(&doc), "pipeline").unwrap().children[1],
            "environment",
        )
        .unwrap();
        let env_map = &env.children[1];
        let foo = entry(env_map, "FOO").unwrap();
        let foo_val = &foo.children[1];
        assert!(matches!(&foo_val.kind, NodeKind::Scalar { .. }));
    }
}
