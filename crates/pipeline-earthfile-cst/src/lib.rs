//! Earthfile DSL → `pipeline_cst::Document`.
//!
//! Earthfiles are a Dockerfile-flavoured DSL:
//!
//! ```text
//! VERSION 0.8
//! FROM rust:1.75
//! WORKDIR /app
//!
//! build:
//!     COPY . .
//!     RUN cargo build --release
//!     SAVE ARTIFACT target/release/my-binary
//!
//! test:
//!     BUILD +build
//!     RUN cargo test
//! ```
//!
//! Top-level (indent 0): either a Dockerfile-style command
//! (`VERSION x.y`, `FROM image`, `WORKDIR path`, `ARG`, `ENV`, …)
//! or a recipe declaration (`<name>:` ending in colon). A
//! recipe body is one indented command per line, each becoming a
//! one-entry mapping in a sequence — that gives the cascade a
//! `cst:MappingEntry[key=<verb>]` to classify (`RUN → step`,
//! `SAVE ARTIFACT → artifact`, `BUILD → dependency_edge`, …) via
//! the catalog table.
//!
//! Comments (`#`) are preserved as `NodeKind::Comment` children
//! of their enclosing container — same convention as the YAML
//! CST, so the carrier-comment machinery applies unchanged.
//!
//! What's NOT modeled: shell-line continuations (`\` at end of
//! line), the `--<flag>` arguments to verbs (kept as part of the
//! scalar text), `IF`/`FOR`/`WITH`/`TRY` blocks (treated as flat
//! commands; their body is currently swallowed into the scalar).
//! Good enough to validate the cascade end-to-end; richer
//! structure can grow when a real Earthly fixture demands it.

use pipeline_cst::{CommentKind, Document, Node, NodeKind, ScalarStyle, Span};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ParseError {
    #[error("empty Earthfile")]
    Empty,
}

/// Parse Earthfile source into the same `pipeline_cst::Document`
/// shape the YAML parser produces. Lossy on shell-line
/// continuations and on the inner structure of multi-line block
/// commands (IF/FOR/WITH/TRY); the immediate commands and recipe
/// boundaries are preserved verbatim.
#[allow(clippy::missing_errors_doc)]
pub fn parse(source: &str) -> Result<Document, ParseError> {
    let mut top_entries: Vec<Node> = Vec::new();
    let mut current_recipe: Option<(String, Span, Vec<Node>)> = None;

    for line in lines_with_spans(source) {
        process_line(&line, source, &mut top_entries, &mut current_recipe);
    }

    if let Some(prev) = current_recipe.take() {
        top_entries.push(finish_recipe(prev));
    }
    // Empty input → an empty Document (root mapping with no children), NOT an
    // error. The YAML parsers accept empty / `{}` and yield an empty pipeline;
    // the Earthfile parser must match so a degenerate cross-platform emit (a hub
    // the target can't populate) round-trips as a vacuous-but-stable pipeline (∅
    // / vacuous-ok) instead of panicking the fixpoint trip with a parse error
    // (the earthly interop `x`). build_root handles empty `top_entries`.
    Ok(Document::from_parts(
        source,
        build_root(source, top_entries),
    ))
}

struct LineInfo {
    start: usize,
    end: usize,
    indent: usize,
    trimmed_len: usize,
}

fn lines_with_spans(source: &str) -> Vec<LineInfo> {
    let mut out = Vec::new();
    let mut byte = 0usize;
    let bytes = source.as_bytes();
    while byte < bytes.len() {
        let line_start = byte;
        while byte < bytes.len() && bytes[byte] != b'\n' {
            byte += 1;
        }
        let line_end = byte;
        if byte < bytes.len() {
            byte += 1;
        }
        let raw_line = &source[line_start..line_end];
        let indent: usize = raw_line
            .bytes()
            .take_while(|b| *b == b' ' || *b == b'\t')
            .count();
        let trimmed_len = raw_line[indent..].trim_end().len();
        out.push(LineInfo {
            start: line_start,
            end: line_end,
            indent,
            trimmed_len,
        });
    }
    out
}

fn process_line(
    info: &LineInfo,
    source: &str,
    top_entries: &mut Vec<Node>,
    current_recipe: &mut Option<(String, Span, Vec<Node>)>,
) {
    let content_start = info.start + info.indent;
    let content_end = content_start + info.trimmed_len;
    if info.trimmed_len == 0 {
        return;
    }
    let trimmed = &source[content_start..content_end];
    if trimmed.starts_with('#') {
        let comment_node = Node {
            kind: NodeKind::Comment {
                kind: CommentKind::FullLine,
            },
            span: Span {
                start: content_start,
                end: info.end,
            },
            children: Vec::new(),
            anchor: None,
            tag: None,
        };
        if let Some((_, _, body)) = current_recipe.as_mut() {
            body.push(comment_node);
        } else {
            top_entries.push(comment_node);
        }
        return;
    }
    if info.indent == 0 {
        if let Some(prev) = current_recipe.take() {
            top_entries.push(finish_recipe(prev));
        }
        if let Some(name) = recipe_name(trimmed) {
            *current_recipe = Some((
                name.to_string(),
                Span {
                    start: content_start,
                    end: content_end,
                },
                Vec::new(),
            ));
        } else {
            top_entries.push(command_entry(trimmed, content_start, content_end));
        }
    } else if let Some((_, _, body)) = current_recipe.as_mut() {
        body.push(command_entry(trimmed, content_start, content_end));
    } else {
        top_entries.push(command_entry(trimmed, content_start, content_end));
    }
}

fn build_root(source: &str, top_entries: Vec<Node>) -> Node {
    let span_start = top_entries
        .iter()
        .filter(|n| !matches!(n.kind, NodeKind::Comment { .. }))
        .map(|n| n.span.start)
        .min()
        .unwrap_or(0);
    let span_end = top_entries
        .iter()
        .filter(|n| !matches!(n.kind, NodeKind::Comment { .. }))
        .map(|n| n.span.end)
        .max()
        .unwrap_or(source.len());
    let mapping = Node {
        kind: NodeKind::Mapping,
        span: Span {
            start: span_start,
            end: span_end,
        },
        children: top_entries,
        anchor: None,
        tag: None,
    };
    Node {
        kind: NodeKind::Document,
        span: Span {
            start: 0,
            end: source.len(),
        },
        children: vec![mapping],
        anchor: None,
        tag: None,
    }
}

/// `"build:"` → `Some("build")` if the trimmed line is a recipe
/// declaration. Recipe names contain only `[A-Za-z0-9_-]` and end
/// in a single `:`. Excludes Earthfile verbs (which are uppercase
/// and have a space-separated argument) — a verb-only line with a
/// trailing `:` (e.g. `RUN:`) is unusual but would be classified
/// as a recipe here; in practice no Earthfile verb ends `:`.
fn recipe_name(trimmed: &str) -> Option<&str> {
    let stripped = trimmed.strip_suffix(':')?;
    if stripped.is_empty() {
        return None;
    }
    if stripped
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        Some(stripped)
    } else {
        None
    }
}

/// Split a command line into `(verb, args)`. Verbs may be
/// space-joined (`SAVE ARTIFACT`, `FROM DOCKERFILE`, `GIT CLONE`)
/// — match those first, fall back to the first whitespace split.
fn split_verb(trimmed: &str) -> (String, &str) {
    const MULTI: &[&str] = &[
        "SAVE ARTIFACT",
        "SAVE IMAGE",
        "FROM DOCKERFILE",
        "GIT CLONE",
        "IF / ELSE",
    ];
    for m in MULTI {
        if let Some(rest) = trimmed.strip_prefix(m) {
            return ((*m).to_string(), rest.trim_start());
        }
    }
    match trimmed.split_once(char::is_whitespace) {
        Some((v, rest)) => (v.to_string(), rest.trim_start()),
        None => (trimmed.to_string(), ""),
    }
}

fn command_entry(trimmed: &str, span_start: usize, span_end: usize) -> Node {
    let (verb, args) = split_verb(trimmed);
    let key_span = Span {
        start: span_start,
        end: span_start + verb.len(),
    };
    let value_span_start = if args.is_empty() {
        span_end
    } else {
        span_end - args.len()
    };
    let value_span = Span {
        start: value_span_start,
        end: span_end,
    };
    Node {
        kind: NodeKind::MappingEntry { key_text: verb },
        span: Span {
            start: span_start,
            end: span_end,
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
            Node {
                kind: NodeKind::Scalar {
                    style: ScalarStyle::Plain,
                },
                span: value_span,
                children: Vec::new(),
                anchor: None,
                tag: None,
            },
        ],
        anchor: None,
        tag: None,
    }
}

/// A recipe (`name:` + indented body) becomes `MappingEntry[key=name] → value =
/// cst:Mapping`, whose children are the command `MappingEntry[VERB]`s directly.
/// This flat shape matches the earthly TGG ruleset (job-Mapping with command
/// entries as direct children) and `emit_earthfile`, so the recipe round-trips
/// AND the forward cascade lifts its targets to `hub:job` / steps.
fn finish_recipe((name, name_span, body): (String, Span, Vec<Node>)) -> Node {
    let last_end = body.last().map_or(name_span.end, |n| n.span.end);
    let key_span = Span {
        start: name_span.start,
        end: name_span.start + name.len(),
    };
    let key_node = Node {
        kind: NodeKind::Scalar {
            style: ScalarStyle::Plain,
        },
        span: key_span,
        children: Vec::new(),
        anchor: None,
        tag: None,
    };
    let recipe = Node {
        kind: NodeKind::Mapping,
        span: Span {
            start: name_span.end,
            end: last_end,
        },
        children: body
            .into_iter()
            .filter(|n| {
                matches!(
                    n.kind,
                    NodeKind::MappingEntry { .. } | NodeKind::Comment { .. }
                )
            })
            .collect(),
        anchor: None,
        tag: None,
    };
    Node {
        kind: NodeKind::MappingEntry { key_text: name },
        span: Span {
            start: name_span.start,
            end: last_end,
        },
        children: vec![key_node, recipe],
        anchor: None,
        tag: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_minimal_earthfile() {
        let src = "VERSION 0.8\nFROM rust:1.75\n";
        let doc = parse(src).unwrap();
        let mapping = &doc.root().children[0];
        assert!(matches!(mapping.kind, NodeKind::Mapping));
        assert_eq!(mapping.children.len(), 2);
        let version = &mapping.children[0];
        assert!(
            matches!(&version.kind, NodeKind::MappingEntry { key_text } if key_text == "VERSION")
        );
    }

    #[test]
    fn recipe_becomes_named_mapping() {
        // A recipe is a `cst:Mapping` whose children are the command entries
        // directly (the flat shape the ruleset + emitter expect).
        let src = "build:\n    RUN cargo build\n    SAVE ARTIFACT target/\n";
        let doc = parse(src).unwrap();
        let mapping = &doc.root().children[0];
        let build = &mapping.children[0];
        assert!(matches!(&build.kind, NodeKind::MappingEntry { key_text } if key_text == "build"));
        let body = &build.children[1];
        assert!(matches!(body.kind, NodeKind::Mapping));
        assert_eq!(body.children.len(), 2, "two command entries in the recipe");
        let run_entry = &body.children[0];
        assert!(
            matches!(&run_entry.kind, NodeKind::MappingEntry { key_text } if key_text == "RUN")
        );
    }

    #[test]
    fn multi_word_verb_save_artifact() {
        let src = "build:\n    SAVE ARTIFACT target/release/foo\n";
        let doc = parse(src).unwrap();
        let mapping = &doc.root().children[0];
        let build = &mapping.children[0];
        let body = &build.children[1];
        let entry = &body.children[0];
        assert!(
            matches!(&entry.kind, NodeKind::MappingEntry { key_text } if key_text == "SAVE ARTIFACT")
        );
    }
}
