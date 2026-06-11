//! Read-only view of the Hub-IR over a `seesaw_core::graph::TypedGraph`.
//!
//! The Hub-IR is natively a typed graph: the schema in
//! [`crate::schema`] is the spec, the forward TGG cascade is the
//! producer, the consumers (CLI / migrate / lint / FFI) are the
//! readers. This module is the read-side API the consumers use, so
//! they don't each have to walk the graph by hand.
//!
//! Naming: every accessor is a noun (`pipeline_root`, `jobs`,
//! `attr`), not a verb. It returns a `GhostId` or a borrowed string
//! — never an owned struct. Owned struct-IR construction is in
//! [`crate::model`], gradually being retired as consumers move over.
//!
//! Unified satellite model (post `scalar_attr/seq_attr/block_attr
//! unified via hub:value`, see `catalog/hub_schema.toml`). EVERY field
//! — scalar, sequence, or nested block — is one `hub:attr{name}` child
//! reached via a `hub:has_attr` edge, and the attr's payload hangs off
//! a single `hub:has_value` edge:
//! - **scalar** field: `attr -has_value-> hub:value{text}` — the value
//!   is `value.text`.
//! - **sequence** field (`vkind="seq"`): `attr -has_value-> hub:collection`,
//!   whose `hub:has_item` children are the elements (constructs or
//!   `hub:value` leaves).
//! - **block / single ref** field: `attr -has_value-> <construct>`
//!   directly (e.g. `triggers -> hub:trigger`, `pull_request ->
//!   hub:pull_request`).
//! - **map** field: `attr -has_value-> hub:collection`, whose
//!   `hub:has_item` children are themselves `hub:attr{name=key}` nodes.
//!
//! There are no `hub:has_<kind>` edges; every traversal goes through
//! `has_attr → has_value → (has_item)`.

use seesaw_core::graph::{GhostId, TypedGraph};

use crate::schema::{node, FieldKind};

/// Edge kind linking a construct to one of its field satellites.
pub const HAS_ATTR: &str = "hub:has_attr";

/// Edge kind linking a `hub:attr` to its payload (a `hub:value`,
/// `hub:collection`, or a nested construct).
pub const HAS_VALUE: &str = "hub:has_value";

/// Edge kind linking a `hub:collection` to each of its elements.
pub const HAS_ITEM: &str = "hub:has_item";

/// Node kind for a field satellite (carries `name`, `vkind`).
pub const ATTR_KIND: &str = "hub:attr";

/// Node kind for a scalar payload leaf (carries `text`).
pub const VALUE_KIND: &str = "hub:value";

/// Node kind for a sequence / map payload container.
pub const COLLECTION_KIND: &str = "hub:collection";

/// The outermost `hub:pipeline` node in `g`. Returns `None` if the
/// graph holds no pipeline (e.g. an empty fixture).
///
/// Convention: the outermost pipeline is the one no other
/// `hub:pipeline` has as a `hub:has_pipeline` target. The catalog's
/// only self-referential field is `pipeline.pipelines` (the
/// bitbucket case, see `catalog/ir.toml`); for every other platform
/// there is exactly one `hub:pipeline` and the lookup is unambiguous.
#[must_use]
pub fn pipeline_root(g: &TypedGraph) -> Option<GhostId> {
    let candidates: Vec<_> = g
        .matchable_nodes_by_kind(node::PIPELINE)
        .map(|n| n.id)
        .collect();
    if candidates.is_empty() {
        return None;
    }
    let mut nested = std::collections::HashSet::new();
    for (_, t, edge) in g.iter_edges() {
        if edge.type_id == "hub:has_pipeline" {
            nested.insert(t);
        }
    }
    candidates.into_iter().find(|c| !nested.contains(c))
}

/// Every `hub:pipeline` node in `g` — for the bitbucket case this is
/// `[outer, sub_pipeline_1, …]`; for every other platform a one-
/// element vec. Order is graph-iteration order (deterministic per
/// run, not necessarily source order).
#[must_use]
pub fn pipelines(g: &TypedGraph) -> Vec<GhostId> {
    g.matchable_nodes_by_kind(node::PIPELINE)
        .map(|n| n.id)
        .collect()
}

/// All children of `parent` reachable via the given `edge_kind`, in
/// the order the graph iterates them. Use [`schema`](crate::schema)
/// to look up the right edge for a field.
#[must_use]
pub fn children(g: &TypedGraph, parent: GhostId, edge_kind: &str) -> Vec<GhostId> {
    g.iter_edges()
        .into_iter()
        .filter(|(s, _, e)| *s == parent && e.type_id == edge_kind)
        .map(|(_, t, _)| t)
        .collect()
}

/// The single out-neighbour of `src` along `edge_kind`, if any. The
/// unified model attaches exactly one payload per `has_value`, so this
/// is the natural primitive for "follow the value edge".
#[must_use]
fn one(g: &TypedGraph, src: GhostId, edge_kind: &str) -> Option<GhostId> {
    g.iter_edges()
        .into_iter()
        .find(|(s, _, e)| *s == src && e.type_id == edge_kind)
        .map(|(_, t, _)| t)
}

/// The `hub:attr` satellite for `field` on `construct`, if set.
#[must_use]
pub fn attr_node(g: &TypedGraph, construct: GhostId, field: &str) -> Option<GhostId> {
    children(g, construct, HAS_ATTR).into_iter().find(|a| {
        g.get_node(a).is_some_and(|nd| {
            nd.type_id == ATTR_KIND && nd.attrs.get("name").map(String::as_str) == Some(field)
        })
    })
}

/// Read a scalar field from a construct by its IR field name. Walks
/// `has_attr → attr{name=field} → has_value → hub:value` and returns
/// `value.text`. Falls back to an inline `value` attr on the `hub:attr`
/// node (a few helper satellites still carry their scalar inline).
/// Returns `None` if the field is unset or is not a scalar.
#[must_use]
pub fn attr<'a>(g: &'a TypedGraph, construct: GhostId, field: &str) -> Option<&'a str> {
    let a = attr_node(g, construct, field)?;
    if let Some(v) = one(g, a, HAS_VALUE) {
        if let Some(vn) = g.get_node(&v) {
            if vn.type_id == VALUE_KIND {
                if let Some(t) = vn.attrs.get("text") {
                    return Some(t.as_str());
                }
            }
        }
    }
    g.get_node(&a)?.attrs.get("value").map(String::as_str)
}

/// The common convenience: the `name` field of any construct.
#[must_use]
pub fn name(g: &TypedGraph, construct: GhostId) -> Option<&str> {
    attr(g, construct, "name")
}

/// Walk every scalar field set on a construct, regardless of name.
/// Yields `(field_name, value)` pairs in graph-iteration order, for
/// every satellite whose payload is a scalar `hub:value`. Sequence /
/// block / map fields are skipped (use [`refs`] / [`items`] for those).
pub fn all_attrs(g: &TypedGraph, construct: GhostId) -> impl Iterator<Item = (&str, &str)> + '_ {
    children(g, construct, HAS_ATTR)
        .into_iter()
        .filter_map(move |a| {
            let an = g.get_node(&a)?;
            if an.type_id != ATTR_KIND {
                return None;
            }
            let n = an.attrs.get("name")?.as_str();
            if let Some(v) = one(g, a, HAS_VALUE) {
                if let Some(vn) = g.get_node(&v) {
                    if vn.type_id == VALUE_KIND {
                        if let Some(t) = vn.attrs.get("text") {
                            return Some((n, t.as_str()));
                        }
                    }
                }
            }
            an.attrs.get("value").map(|v| (n, v.as_str()))
        })
}

/// The elements of a sequence field: `has_attr → attr{name=field} →
/// has_value → hub:collection → has_item → elements`. Empty if the
/// field is unset or not a collection.
#[must_use]
pub fn items(g: &TypedGraph, construct: GhostId, field: &str) -> Vec<GhostId> {
    let Some(a) = attr_node(g, construct, field) else {
        return vec![];
    };
    let Some(coll) = one(g, a, HAS_VALUE) else {
        return vec![];
    };
    if g.get_node(&coll)
        .is_some_and(|n| n.type_id == COLLECTION_KIND)
    {
        children(g, coll, HAS_ITEM)
    } else {
        vec![]
    }
}

/// The scalar text of a `hub:value` leaf node, if it carries one
/// directly. For list elements that wrap their text behind a
/// `has_value` / `item_value` edge (`hub:item`, double-`hub:value`),
/// use [`leaf_text`].
#[must_use]
pub fn text(g: &TypedGraph, value: GhostId) -> Option<&str> {
    let n = g.get_node(&value)?;
    if n.type_id == VALUE_KIND {
        n.attrs.get("text").map(String::as_str)
    } else {
        None
    }
}

/// The scalar text of a sequence element, tolerating the seeder's
/// several leaf conventions: a `hub:value{text}` directly; a wrapper
/// `hub:value`/`hub:item` whose `text` hangs behind a `hub:has_value`
/// or `hub:item_value` edge. Returns the first `text` found within a
/// couple of hops.
#[must_use]
pub fn leaf_text(g: &TypedGraph, element: GhostId) -> Option<&str> {
    fn walk(g: &TypedGraph, id: GhostId, depth: u8) -> Option<&str> {
        let n = g.get_node(&id)?;
        if let Some(t) = n.attrs.get("text") {
            return Some(t.as_str());
        }
        if depth == 0 {
            return None;
        }
        for (s, t, e) in g.iter_edges() {
            if s == id && matches!(e.type_id.as_str(), HAS_VALUE | "hub:item_value") {
                if let Some(found) = walk(g, t, depth - 1) {
                    return Some(found);
                }
            }
        }
        None
    }
    walk(g, element, 3)
}

/// All `hub:job` elements under a pipeline or stage node. Tolerates
/// both producer conventions: a direct `hub:has_job` edge (gitlab-style
/// top-level jobs) and the `jobs` sequence field (github/azure-style).
/// A grouping `hub:item` inside the collection (bitbucket `- parallel:`
/// → `item{vkind=parallel}`) is transparent: its `hub:job` children are
/// expanded in place; any other item is kept as-is.
#[must_use]
pub fn jobs(g: &TypedGraph, parent: GhostId) -> Vec<GhostId> {
    let mut v = children(g, parent, "hub:has_job");
    for it in items(g, parent, "jobs") {
        let grouped: Vec<GhostId> = children(g, it, HAS_ITEM)
            .into_iter()
            .filter(|j| g.get_node(j).is_some_and(|n| n.type_id == "hub:job"))
            .collect();
        if grouped.is_empty() {
            v.push(it);
        } else {
            v.extend(grouped);
        }
    }
    v
}

/// Job GROUPS under a pipeline's named-selector field (bitbucket
/// `pipelines.branches/tags/…` → `attr{name=<field>}` → collection →
/// `item{vkind=…, name=<key>}` → jobs). Returns `(group, jobs)` pairs;
/// the group node carries `name` and `vkind` attrs.
#[must_use]
pub fn job_groups(g: &TypedGraph, parent: GhostId, field: &str) -> Vec<(GhostId, Vec<GhostId>)> {
    items(g, parent, field)
        .into_iter()
        .map(|grp| {
            // Direct member jobs, plus one transparent level of nested
            // grouping (a `- parallel:` item inside the selector group).
            let mut js: Vec<GhostId> = Vec::new();
            for c in children(g, grp, HAS_ITEM) {
                match g.get_node(&c).map(|n| n.type_id.as_str()) {
                    Some("hub:job") => js.push(c),
                    Some("hub:item") => js.extend(
                        children(g, c, HAS_ITEM)
                            .into_iter()
                            .filter(|j| g.get_node(j).is_some_and(|n| n.type_id == "hub:job")),
                    ),
                    _ => {}
                }
            }
            (grp, js)
        })
        .collect()
}

/// All `hub:step` elements under a job (or stage) node — direct
/// `hub:has_step` edges and/or the `steps` sequence field.
#[must_use]
pub fn steps(g: &TypedGraph, parent: GhostId) -> Vec<GhostId> {
    let mut v = children(g, parent, "hub:has_step");
    v.extend(items(g, parent, "steps"));
    v
}

/// All `hub:stage` elements under a pipeline node — direct
/// `hub:has_stage` edges and/or the `stages` sequence field.
#[must_use]
pub fn stages(g: &TypedGraph, pipeline: GhostId) -> Vec<GhostId> {
    let mut v = children(g, pipeline, "hub:has_stage");
    v.extend(items(g, pipeline, "stages"));
    v
}

/// Generic ref-field lookup by IR field name. Follows `has_attr →
/// attr{name=field} → has_value`; if the payload is a `hub:collection`
/// its `has_item` children are returned (sequence ref), otherwise the
/// single payload construct is returned (block / single ref). For
/// scalar fields, use [`attr`] instead.
#[must_use]
pub fn refs(g: &TypedGraph, construct: GhostId, field: &str) -> Vec<GhostId> {
    let Some(a) = attr_node(g, construct, field) else {
        return vec![];
    };
    let Some(payload) = one(g, a, HAS_VALUE) else {
        return vec![];
    };
    match g.get_node(&payload) {
        Some(n) if n.type_id == COLLECTION_KIND => children(g, payload, HAS_ITEM),
        // A bare scalar value is not a ref; only block constructs count.
        Some(n) if n.type_id == VALUE_KIND => vec![],
        Some(_) => vec![payload],
        None => vec![],
    }
}

/// Human-readable summary of a Hub-IR graph — pure read-side
/// consumer: never builds a struct, never allocates a `Pipeline`.
/// This is the canonical "how a consumer reads the graph" example;
/// the cli's `inspect` and `migrate` subcommands will migrate to
/// reading like this as the consumer-migration phase proceeds.
///
/// The summary is platform-neutral: identical code produces a
/// readable dump for any of the 17 catalogued platforms, because
/// the Hub-IR schema is the common denominator.
#[must_use]
pub fn summary(g: &TypedGraph) -> String {
    use std::fmt::Write;
    let mut out = String::new();
    let roots = pipelines(g);
    let _ = writeln!(out, "pipelines: {}", roots.len());
    let mut seen = std::collections::HashSet::new();
    for p in &roots {
        write_construct(g, *p, 1, &mut seen, &mut out);
    }
    let total = g.matchable_nodes_by_kind(node::ATTR).count();
    let _ = writeln!(out, "({total} hub:attr satellite nodes total)");
    out
}

fn write_construct(
    g: &TypedGraph,
    id: GhostId,
    depth: usize,
    seen: &mut std::collections::HashSet<GhostId>,
    out: &mut String,
) {
    use std::fmt::Write;
    if !seen.insert(id) {
        // Already printed somewhere else in the tree — IR schema
        // sometimes has multiple ref fields on one edge kind
        // (job.after_steps, job.before_steps and job.steps all
        // map to hub:has_step), so the same child appears under
        // several parent fields. Don't print it again.
        return;
    }
    let indent = "  ".repeat(depth);
    let Some(nd) = g.get_node(&id) else { return };
    let kind = nd.type_id.strip_prefix("hub:").unwrap_or(&nd.type_id);
    let n = name(g, id).unwrap_or("<unnamed>");
    let _ = write!(out, "{indent}{kind} {n:?}");
    // Inline scalar attributes (other than `name`).
    let attrs: Vec<_> = all_attrs(g, id).filter(|(k, _)| *k != "name").collect();
    if !attrs.is_empty() {
        let _ = write!(out, " {{");
        for (i, (k, v)) in attrs.iter().enumerate() {
            if i > 0 {
                let _ = write!(out, ", ");
            }
            let v_short = if v.len() > 40 {
                format!("{}…", &v[..40])
            } else {
                (*v).to_string()
            };
            let _ = write!(out, "{k}={v_short:?}");
        }
        let _ = write!(out, "}}");
    }
    out.push('\n');
    // Recurse through ref-fields. Depth guard plus seen-set prevents
    // unbounded recursion through self-referential fields like the
    // bitbucket `pipeline.pipelines` edge or any other cycle the
    // cascade leaves in the graph.
    if depth > 30 {
        let _ = writeln!(out, "{indent}  (max depth reached)");
        return;
    }
    let Some(node_kind) = crate::schema::SCHEMA
        .iter()
        .find(|nk| nk.id.strip_prefix("hub:").unwrap_or(nk.id) == kind)
    else {
        return;
    };
    for field in node_kind.fields {
        if let FieldKind::Ref(_) = field.kind {
            let kids = refs(g, id, field.name);
            for kid in kids {
                write_construct(g, kid, depth + 1, seen, out);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use seesaw_core::graph::TypedGraph;
    use std::collections::BTreeMap;

    fn attrs(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| ((*k).into(), (*v).into()))
            .collect()
    }

    #[test]
    fn pipeline_root_returns_unnested_pipeline() {
        let mut g = TypedGraph::new();
        let outer = g.add_baseline_node(node::PIPELINE, "outer", attrs(&[]));
        let inner = g.add_baseline_node(node::PIPELINE, "inner", attrs(&[]));
        g.add_edge(
            outer,
            inner,
            "hub:has_pipeline",
            attrs(&[]),
            seesaw_core::graph::Status::Solid,
        );
        let root = pipeline_root(&g).expect("a root pipeline");
        assert_eq!(root, outer);
    }

    /// Attach a scalar field `name=value` to `construct` in the unified
    /// model: `construct -has_attr-> attr{name} -has_value-> value{text}`.
    fn add_scalar(g: &mut TypedGraph, construct: GhostId, field: &str, value: &str) {
        let a = g.add_baseline_node(
            ATTR_KIND,
            field,
            attrs(&[("name", field), ("vkind", "scalar")]),
        );
        let v = g.add_baseline_node(VALUE_KIND, "v", attrs(&[("text", value)]));
        g.add_edge(
            construct,
            a,
            HAS_ATTR,
            attrs(&[]),
            seesaw_core::graph::Status::Solid,
        );
        g.add_edge(
            a,
            v,
            HAS_VALUE,
            attrs(&[]),
            seesaw_core::graph::Status::Solid,
        );
    }

    /// Attach a sequence field of constructs: `construct -has_attr->
    /// attr{name,vkind=seq} -has_value-> collection -has_item-> items`.
    fn add_seq(g: &mut TypedGraph, construct: GhostId, field: &str, items: &[GhostId]) {
        let a = g.add_baseline_node(
            ATTR_KIND,
            field,
            attrs(&[("name", field), ("vkind", "seq")]),
        );
        let coll = g.add_baseline_node(COLLECTION_KIND, "c", attrs(&[]));
        g.add_edge(
            construct,
            a,
            HAS_ATTR,
            attrs(&[]),
            seesaw_core::graph::Status::Solid,
        );
        g.add_edge(
            a,
            coll,
            HAS_VALUE,
            attrs(&[]),
            seesaw_core::graph::Status::Solid,
        );
        for it in items {
            g.add_edge(
                coll,
                *it,
                HAS_ITEM,
                attrs(&[]),
                seesaw_core::graph::Status::Solid,
            );
        }
    }

    #[test]
    fn attr_reads_unified_scalar_value() {
        let mut g = TypedGraph::new();
        let job = g.add_baseline_node(node::JOB, "job", attrs(&[]));
        add_scalar(&mut g, job, "name", "build");
        assert_eq!(attr(&g, job, "name"), Some("build"));
        assert_eq!(attr(&g, job, "missing"), None);
        assert_eq!(name(&g, job), Some("build"));
    }

    #[test]
    fn jobs_follows_seq_collection() {
        let mut g = TypedGraph::new();
        let p = g.add_baseline_node(node::PIPELINE, "p", attrs(&[]));
        let j1 = g.add_baseline_node(node::JOB, "j1", attrs(&[]));
        let j2 = g.add_baseline_node(node::JOB, "j2", attrs(&[]));
        add_seq(&mut g, p, "jobs", &[j1, j2]);
        let js = jobs(&g, p);
        assert_eq!(js.len(), 2);
        assert!(js.contains(&j1));
        assert!(js.contains(&j2));
    }

    #[test]
    fn refs_resolves_seq_field_to_items() {
        let mut g = TypedGraph::new();
        let p = g.add_baseline_node(node::PIPELINE, "p", attrs(&[]));
        let j = g.add_baseline_node(node::JOB, "j", attrs(&[]));
        add_seq(&mut g, p, "jobs", &[j]);
        let found = refs(&g, p, "jobs");
        assert_eq!(found, vec![j]);
    }

    #[test]
    fn refs_resolves_block_single_ref() {
        let mut g = TypedGraph::new();
        let p = g.add_baseline_node(node::PIPELINE, "p", attrs(&[]));
        let trig = g.add_baseline_node(node::TRIGGER, "t", attrs(&[]));
        let a = g.add_baseline_node(ATTR_KIND, "triggers", attrs(&[("name", "triggers")]));
        g.add_edge(
            p,
            a,
            HAS_ATTR,
            attrs(&[]),
            seesaw_core::graph::Status::Solid,
        );
        g.add_edge(
            a,
            trig,
            HAS_VALUE,
            attrs(&[]),
            seesaw_core::graph::Status::Solid,
        );
        assert_eq!(refs(&g, p, "triggers"), vec![trig]);
    }
}
