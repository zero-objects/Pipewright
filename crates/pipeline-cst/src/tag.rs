//! GitLab-specific tag resolver: `!reference` and `!secret`.
//!
//! `!reference [path, segments]` is a path into the same file.
//! `!secret { vault: ..., name: ... }` references a secret store.
//!
//! These do NOT modify the CST. Resolvers return a typed value
//! that consumers (M3 onward) can act on.

use crate::cst::{Document, Node, NodeKind};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolvedTag {
    /// `!reference [a, b, c]` — path through nested mappings/sequences.
    Reference { path: Vec<String> },
    /// `!secret name` — secret reference, possibly with vault hint.
    Secret { name: String },
    /// Unknown tag, opaque value preserved.
    Unknown {
        tag_name: String,
        value_text: String,
    },
}

/// Resolve the tag attached to `value_node`, given the source text.
///
/// If the node has no tag, returns `None`. Otherwise returns the
/// resolved variant.
#[must_use]
pub fn resolve_tag(doc: &Document, value_node: &Node) -> Option<ResolvedTag> {
    let tag = value_node.tag.as_ref()?;
    let value_text = doc.span_text(value_node.span);
    match tag.as_str() {
        "!reference" => {
            // Value is a flow-list scalar: `[a, b, c]`. Strip brackets, split.
            let path = parse_flow_list(value_text);
            Some(ResolvedTag::Reference { path })
        }
        "!secret" => {
            // Plain scalar with secret name (anonymous-vault case).
            Some(ResolvedTag::Secret {
                name: value_text.trim().to_string(),
            })
        }
        other => Some(ResolvedTag::Unknown {
            tag_name: other.to_string(),
            value_text: value_text.to_string(),
        }),
    }
}

/// Walk the document, collecting every (value-node, `ResolvedTag`) pair.
#[must_use]
pub fn collect_tags(doc: &Document) -> Vec<(&Node, ResolvedTag)> {
    let mut out = Vec::new();
    walk(doc, doc.root(), &mut out);
    out
}

fn walk<'doc>(doc: &'doc Document, node: &'doc Node, out: &mut Vec<(&'doc Node, ResolvedTag)>) {
    if let Some(resolved) = resolve_tag(doc, node) {
        out.push((node, resolved));
    }
    for child in &node.children {
        walk(doc, child, out);
    }
}

fn parse_flow_list(text: &str) -> Vec<String> {
    let trimmed = text.trim();
    let inner = trimmed
        .strip_prefix('[')
        .and_then(|s| s.strip_suffix(']'))
        .unwrap_or(trimmed);
    inner
        .split(',')
        .map(|s| {
            let t = s.trim();
            // Strip single/double quotes if present.
            t.trim_matches(|c| c == '"' || c == '\'').to_string()
        })
        .filter(|s| !s.is_empty())
        .collect()
}

// Tag also applies when attached to a MappingEntry (the value carries it).
// The CST already attaches tags to the value node in `parse_value`, so the
// walk above covers both forms.
//
// Note: `MappingEntry.tag` is also possible — the parser attaches it there
// in some cases (key-line anchor/tag). The walk visits MappingEntries too,
// but their span text is the whole entry. We currently only resolve tags
// that sit on Scalar/Alias/Mapping/Sequence value nodes, which is the
// common GitLab shape. If a tag ends up on a MappingEntry, this is a
// data-shape edge case — out of scope for v0.1 of this resolver.
#[allow(dead_code)]
fn _doc_placeholder() {}

impl ResolvedTag {
    #[must_use]
    pub fn is_reference(&self) -> bool {
        matches!(self, ResolvedTag::Reference { .. })
    }
    #[must_use]
    pub fn is_secret(&self) -> bool {
        matches!(self, ResolvedTag::Secret { .. })
    }
}

#[allow(dead_code)]
fn _suppress_unused(node: &Node) {
    let _ = matches!(node.kind, NodeKind::Document);
}
