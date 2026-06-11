//! Bitbucket Pipelines: jobs live under `pipelines.default`,
//! `pipelines.branches.<branch>`, `pipelines.pull-requests.*`, …
//! The `pipelines:` block seeds as meta (classify tags its value
//! `construct=pipeline`); a post-pass then tags each inline
//! `- step:` body as `construct=job` with a name carrier from its
//! `name:` entry — under `default:` (incl. `- parallel:` groups)
//! and under the five event-selector maps — so the catalog job
//! rules (R_bitbucket_job + field hooks) plus the bespoke
//! containment rules materialise named, contained hub:jobs.
//! Anchor-referenced steps (`- step: *lint`) seed as scalar
//! aliases, not mappings, so they are skipped structurally (the
//! remaining residual: shared-body steps need reference semantics
//! in the hub — one body, N use sites — which the emit walker's
//! first-reach-wins body rule cannot express today).

use crate::{
    classify::bitbucket::CONSTRUCT_KEYS, cst_attr, seed_top_entry_as_meta, seed_top_level,
    synthesize_name_carrier, SeededGraph, CST_MAPPING, CST_MAPPING_ENTRY, CST_SCALAR, CST_SEQUENCE,
    CST_SEQUENCE_ITEM,
};
use pipeline_cst::{Document, Node, NodeKind};
use seesaw_core::graph::{GhostId, TypedGraph};
use std::collections::HashMap;

#[must_use]
pub fn seed_from_document(doc: &Document, source_file: &str) -> SeededGraph {
    // `- step: *lint` use sites resolve to a CLONE of the anchored template
    // body BEFORE seeding (canonicalize_document pattern), so the existing
    // tag pass + containment rules see real inline bodies. See
    // [`resolve_step_aliases`] for the trade-off.
    let doc = resolve_step_aliases(doc);
    let mut seeded = seed_top_level(
        &doc,
        source_file,
        CONSTRUCT_KEYS,
        |graph, parent_map, entry_node, key, value, source, anchors, classify| {
            if key == "definitions" && matches!(value.kind, NodeKind::Mapping) {
                // `definitions:` is a transparent reusable-defs wrapper. HOIST
                // its `services:` (a map<service> of named service definitions)
                // to the pipeline level so pipeline.services ← services links
                // each definition as a hub:service. Without this the whole
                // services map was tagged as ONE construct=service orphan that
                // dropped on emit. (caches/steps under definitions are
                // anchor-referenced templates, left as-is.)
                for child in &value.children {
                    if let NodeKind::MappingEntry { key_text } = &child.kind {
                        if key_text == "services" && child.children.len() >= 2 {
                            seed_top_entry_as_meta(
                                graph,
                                parent_map,
                                child,
                                key_text,
                                &child.children[1],
                                source,
                                anchors,
                                classify,
                            );
                        }
                    }
                }
            } else {
                seed_top_entry_as_meta(
                    graph, parent_map, entry_node, key, value, source, anchors, classify,
                );
            }
        },
    );
    tag_default_steps_as_jobs(&mut seeded.graph);
    seeded
}

/// Replace every `step: *alias` VALUE with a clone of the anchored
/// template body (the `&lint` definitions under `definitions.steps`),
/// producing a rewritten [`Document`] for seeding.
///
/// Why inline instead of reference semantics in the hub: one shared body
/// with N use sites cannot round-trip through the emit walker (its
/// first-reach-wins body rule writes a multiply-reached body at ONE site
/// and drops the rest). Inlining makes each use site a self-contained
/// job — the fixpoint holds by construction (emit writes the bodies
/// inline; re-forward tags the inline bodies identically). The honest
/// trade-off: emitted YAML is denormalised (aliases expanded, the meta
/// `definitions:` templates dropped as before) — semantically equal, and
/// unlike a bare `*lint` without its anchor definition, actually valid.
/// Cloned bodies keep the TEMPLATE's spans, so jump-to-source points at
/// the definition (correct); GhostIds stay distinct via the parent hash.
fn resolve_step_aliases(doc: &Document) -> Document {
    let mut anchors: HashMap<String, Node> = HashMap::new();
    collect_anchors(doc.root(), &mut anchors);
    let root = if anchors.is_empty() {
        doc.root().clone()
    } else {
        resolve_node(doc.root(), &anchors)
    };
    Document::from_parts(doc.source().to_string(), root)
}

fn collect_anchors(n: &Node, out: &mut HashMap<String, Node>) {
    if let Some(a) = &n.anchor {
        out.insert(a.clone(), n.clone());
    }
    for c in &n.children {
        collect_anchors(c, out);
    }
}

fn resolve_node(n: &Node, anchors: &HashMap<String, Node>) -> Node {
    let mut out = n.clone();
    out.children = n
        .children
        .iter()
        .map(|c| resolve_node(c, anchors))
        .collect();
    if let NodeKind::MappingEntry { key_text } = &n.kind {
        if key_text == "step" && out.children.len() >= 2 {
            if let NodeKind::Alias { name } = &out.children[1].kind {
                if let Some(body) = anchors.get(name.as_str()) {
                    let mut body = body.clone();
                    // The clone must not REDEFINE the anchor at the use site.
                    body.anchor = None;
                    out.children[1] = body;
                }
            }
        }
    }
    out
}

/// Targets of `edge_kind` edges out of `id` whose node has `type_id == kind`.
fn children_of_kind(g: &TypedGraph, id: GhostId, edge_kind: &str, kind: &str) -> Vec<GhostId> {
    let mut out: Vec<GhostId> = g
        .iter_edges()
        .into_iter()
        .filter(|(s, t, e)| {
            *s == id && e.type_id == edge_kind && g.get_node(t).is_some_and(|n| n.type_id == kind)
        })
        .map(|(_, t, _)| t)
        .collect();
    out.sort();
    out
}

fn node_attr(g: &TypedGraph, id: GhostId, attr: &str) -> Option<String> {
    g.get_node(&id).and_then(|n| n.attrs.get(attr)).cloned()
}

/// The event-selector keys under `pipelines:` — each maps named
/// sub-pipeline step lists (branch glob / tag glob / custom name).
const SELECTORS: [&str; 5] = ["branches", "tags", "bookmarks", "pull-requests", "custom"];

/// F2: walk the `pipelines:` block and tag each INLINE `- step:` body
/// mapping as `construct=job`, with a name carrier synthesised from its
/// `name:` entry. Covered (stage 1+2): the `default:` list, `- parallel:`
/// groups inside it, and the five event-selector maps. Alias-valued steps
/// (`- step: *lint`) seed as `cst:Scalar` and fall out of the mapping
/// match; `- parallel:` inside a SELECTOR list stays meta (no containment
/// rule for that nesting yet).
fn tag_default_steps_as_jobs(g: &mut TypedGraph) {
    // The `pipelines:` value mapping (classify-tagged construct=pipeline).
    let Some(pipelines) = g
        .iter_nodes()
        .find(|n| {
            n.type_id == CST_MAPPING
                && n.attrs.get(cst_attr::CONSTRUCT).map(String::as_str) == Some("pipeline")
                && n.attrs.get(cst_attr::PARENT_KEY).map(String::as_str) == Some("pipelines")
        })
        .map(|n| n.id)
    else {
        return;
    };
    for entry in children_of_kind(g, pipelines, crate::CST_HAS_CHILD, CST_MAPPING_ENTRY) {
        match node_attr(g, entry, cst_attr::KEY).as_deref() {
            Some("default") => {
                for seq in children_of_kind(g, entry, crate::CST_VALUE_OF, CST_SEQUENCE) {
                    tag_step_items(g, seq, true);
                }
            }
            Some(sel) if SELECTORS.contains(&sel) => {
                let sel = sel.to_string();
                for map in children_of_kind(g, entry, crate::CST_VALUE_OF, CST_MAPPING) {
                    let mut any = false;
                    for be in children_of_kind(g, map, crate::CST_HAS_CHILD, CST_MAPPING_ENTRY) {
                        let mut tagged = false;
                        for seq in children_of_kind(g, be, crate::CST_VALUE_OF, CST_SEQUENCE) {
                            tagged |= tag_step_items(g, seq, true);
                        }
                        if tagged {
                            // Identity anchor for R_bitbucket_<sel>_group:
                            // the named entry IS the group.
                            g.set_node_attr(&be, "selector", &sel);
                            any = true;
                        }
                    }
                    if any {
                        // GhostId discriminator for the selector containment
                        // rule's reverse direction (mirrors wrapper=step).
                        g.set_node_attr(&map, "wrapper", &sel);
                    }
                }
            }
            _ => {}
        }
    }
}

/// Tag every `- step:` item of `seq`; with `allow_parallel`, recurse ONE
/// level into `- parallel:` groups (their value is another step-item
/// sequence) and mark the group wrapper. Both the `default:` list and the
/// selector lists have parallel containment rules; parallel-inside-parallel
/// has none, so the recursion passes `false` (stays meta rather than
/// producing orphan jobs the emit can't re-derive). Returns whether
/// anything was tagged.
fn tag_step_items(g: &mut TypedGraph, seq: GhostId, allow_parallel: bool) -> bool {
    let mut any = false;
    for item in children_of_kind(g, seq, crate::CST_HAS_CHILD, CST_SEQUENCE_ITEM) {
        for wrapper in children_of_kind(g, item, crate::CST_VALUE_OF, CST_MAPPING) {
            let entries = children_of_kind(g, wrapper, crate::CST_HAS_CHILD, CST_MAPPING_ENTRY);
            // Wrapper discipline: a step/parallel item carries exactly one key.
            let [only] = entries.as_slice() else { continue };
            match node_attr(g, *only, cst_attr::KEY).as_deref() {
                Some("step") => {
                    let mut tagged = false;
                    for body in children_of_kind(g, *only, crate::CST_VALUE_OF, CST_MAPPING) {
                        tag_step_body_as_job(g, body);
                        tagged = true;
                    }
                    if tagged {
                        // GhostId discriminator for the containment rule's
                        // reverse direction (see R_bitbucket_pipeline_jobs_default).
                        g.set_node_attr(&wrapper, "wrapper", "step");
                        any = true;
                    }
                }
                Some("parallel") if allow_parallel => {
                    // LIST form: `- parallel: [- step: …]`.
                    let mut tagged = false;
                    for inner in children_of_kind(g, *only, crate::CST_VALUE_OF, CST_SEQUENCE) {
                        tagged |= tag_step_items(g, inner, false);
                    }
                    if tagged {
                        g.set_node_attr(&wrapper, "wrapper", "parallel");
                        any = true;
                    }
                    // EXPANDED form: `- parallel: {fail-fast: …, steps: [- step: …]}`.
                    // Mutually exclusive with the list form (value is a
                    // mapping); a DISTINCT wrapper tag keeps the two rule
                    // families disjoint backward.
                    let mut tagged_x = false;
                    for pm in children_of_kind(g, *only, crate::CST_VALUE_OF, CST_MAPPING) {
                        for pse in children_of_kind(g, pm, crate::CST_HAS_CHILD, CST_MAPPING_ENTRY)
                        {
                            if node_attr(g, pse, cst_attr::KEY).as_deref() != Some("steps") {
                                continue;
                            }
                            for seq in children_of_kind(g, pse, crate::CST_VALUE_OF, CST_SEQUENCE) {
                                tagged_x |= tag_step_items(g, seq, false);
                            }
                        }
                    }
                    if tagged_x {
                        g.set_node_attr(&wrapper, "wrapper", "parallel_expanded");
                        any = true;
                    }
                }
                _ => {}
            }
        }
    }
    any
}

/// `construct=job` + name carrier from the body's `name:` scalar entry.
fn tag_step_body_as_job(g: &mut TypedGraph, body: GhostId) {
    g.set_node_attr(&body, cst_attr::CONSTRUCT, "job");
    let name = children_of_kind(g, body, crate::CST_HAS_CHILD, CST_MAPPING_ENTRY)
        .into_iter()
        .find(|e| node_attr(g, *e, cst_attr::KEY).as_deref() == Some("name"))
        .and_then(|e| {
            children_of_kind(g, e, crate::CST_VALUE_OF, CST_SCALAR)
                .first()
                .and_then(|v| node_attr(g, *v, cst_attr::TEXT))
        });
    if let Some(name) = name {
        let trimmed = name.trim();
        let trimmed = trimmed
            .strip_prefix('"')
            .and_then(|s| s.strip_suffix('"'))
            .or_else(|| {
                trimmed
                    .strip_prefix('\'')
                    .and_then(|s| s.strip_suffix('\''))
            })
            .unwrap_or(trimmed);
        let (start, end) = (
            node_attr(g, body, cst_attr::SPAN_START).unwrap_or_default(),
            node_attr(g, body, cst_attr::SPAN_END).unwrap_or_default(),
        );
        let (start, end) = (start.parse().unwrap_or(0), end.parse().unwrap_or(0));
        synthesize_name_carrier(g, body, "job", trimmed, start, end);
    }
}
