//! Merge-key (`<<:`) expansion as a logical view.
//!
//! Original CST is untouched. The function `mapping_entries_logical`
//! returns a list of (key, value-node) pairs where merge entries
//! have been expanded against an `AnchorTable`. YAML rule: own keys
//! override merged keys.

use crate::anchor::AnchorTable;
use crate::cst::{Document, Node, NodeKind};
use std::collections::HashSet;

/// One entry in a logical mapping view.
#[derive(Debug, Clone)]
pub struct LogicalEntry<'doc> {
    pub key: String,
    pub value: &'doc Node,
    pub source: EntrySource,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EntrySource {
    /// Key appears directly in the mapping.
    Direct,
    /// Key comes from a merge-key (`<<:`) source. Carries the anchor
    /// name the merge resolved through, so emitters can re-issue the
    /// original `<<: *name` reference instead of inlining.
    Merged { anchor: String },
}

/// Expand merge entries in a Mapping node. Returns a list with own
/// keys first (DIRECT) and merged keys after (MERGED), with own
/// keys winning on collision.
#[must_use]
pub fn mapping_entries_logical<'doc>(
    mapping: &'doc Node,
    anchors: &AnchorTable<'doc>,
) -> Vec<LogicalEntry<'doc>> {
    if !matches!(mapping.kind, NodeKind::Mapping) {
        return Vec::new();
    }
    let mut own: Vec<LogicalEntry<'doc>> = Vec::new();
    let mut merge_sources: Vec<(&'doc Node, String)> = Vec::new();

    for entry in &mapping.children {
        if let NodeKind::MappingEntry { key_text } = &entry.kind {
            if key_text == "<<" {
                // Merge entry. Value is either an alias or a sequence of aliases.
                let value = &entry.children[1];
                collect_merge_sources(value, anchors, &mut merge_sources);
            } else if entry.children.len() >= 2 {
                own.push(LogicalEntry {
                    key: key_text.clone(),
                    value: &entry.children[1],
                    source: EntrySource::Direct,
                });
            }
        }
    }

    // Now add merge-source keys that don't collide with own.
    let own_keys: HashSet<String> = own.iter().map(|e| e.key.clone()).collect();
    let mut merged: Vec<LogicalEntry<'doc>> = Vec::new();
    for (src_mapping, anchor_name) in merge_sources {
        if !matches!(src_mapping.kind, NodeKind::Mapping) {
            continue;
        }
        for entry in &src_mapping.children {
            if let NodeKind::MappingEntry { key_text } = &entry.kind {
                if key_text == "<<" {
                    continue;
                }
                if own_keys.contains(key_text) {
                    continue;
                }
                if merged.iter().any(|e| &e.key == key_text) {
                    continue; // merge-source-order winner
                }
                if entry.children.len() >= 2 {
                    merged.push(LogicalEntry {
                        key: key_text.clone(),
                        value: &entry.children[1],
                        source: EntrySource::Merged {
                            anchor: anchor_name.clone(),
                        },
                    });
                }
            }
        }
    }

    own.extend(merged);
    own
}

fn collect_merge_sources<'doc>(
    value: &'doc Node,
    anchors: &AnchorTable<'doc>,
    out: &mut Vec<(&'doc Node, String)>,
) {
    match &value.kind {
        NodeKind::Alias { name } => {
            if let Some(target) = anchors.resolve(name) {
                out.push((target, name.clone()));
            }
        }
        NodeKind::Sequence => {
            // Multi-merge: `<<: [*a, *b]`.
            for item in &value.children {
                if let NodeKind::SequenceItem = &item.kind {
                    if let Some(inner) = item.children.first() {
                        collect_merge_sources(inner, anchors, out);
                    }
                }
            }
        }
        _ => {}
    }
}

/// Convenience: list logical entries from the document's top-level mapping.
#[must_use]
pub fn top_level_logical(doc: &Document) -> Vec<LogicalEntry<'_>> {
    let table = AnchorTable::collect(doc);
    if let Some(mapping) = doc
        .root()
        .children
        .iter()
        .find(|c| matches!(c.kind, NodeKind::Mapping))
    {
        mapping_entries_logical(mapping, &table)
    } else {
        Vec::new()
    }
}
