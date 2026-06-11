//! Anchor + alias resolution.
//!
//! Walks the tree once, collects all nodes that have an `anchor`.
//! Aliases (`NodeKind::Alias { name }`) can then be resolved by
//! looking up `name` in the table.

use std::collections::HashMap;

use crate::cst::{Document, Node, NodeKind};

#[derive(Debug, Clone)]
pub struct AnchorTable<'doc> {
    map: HashMap<String, &'doc Node>,
}

impl<'doc> AnchorTable<'doc> {
    #[must_use]
    pub fn collect(doc: &'doc Document) -> Self {
        let mut map = HashMap::new();
        walk_collect(doc.root(), &mut map);
        Self { map }
    }

    #[must_use]
    pub fn resolve(&self, alias_name: &str) -> Option<&'doc Node> {
        self.map.get(alias_name).copied()
    }

    #[must_use]
    pub fn names(&self) -> Vec<&str> {
        let mut names: Vec<&str> = self.map.keys().map(String::as_str).collect();
        names.sort_unstable();
        names
    }
}

fn walk_collect<'doc>(node: &'doc Node, map: &mut HashMap<String, &'doc Node>) {
    if let Some(name) = &node.anchor {
        // For MappingEntry, the anchor is logically attached to the
        // entry's VALUE — that's what aliases reference. Insert the
        // value (second child) if present, else the entry itself.
        let target =
            if matches!(node.kind, NodeKind::MappingEntry { .. }) && node.children.len() >= 2 {
                &node.children[1]
            } else {
                node
            };
        map.entry(name.clone()).or_insert(target);
    }
    for child in &node.children {
        walk_collect(child, map);
    }
}
