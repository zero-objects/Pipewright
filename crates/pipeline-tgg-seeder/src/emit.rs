//! Hub-IR → YAML emit by walking the CST subgraph produced by a
//! reverse cascade.
//!
//! The reverse cascade (see [`crate::reverse`]) materialises a
//! `cst:Mapping` / `cst:Sequence` / `cst:Scalar` subgraph from a
//! Hub-IR graph. This module walks that subgraph and writes block-
//! YAML — the minimum sufficient to round-trip-test the
//! forward → reverse → emit path on every catalogued platform.
//!
//! Scope of v1:
//! - Block-style mappings (`key:\n  ...`) and sequences (`- item`).
//! - Plain scalars; values are written verbatim from the
//!   `text` attribute set by the seeder.
//! - Comments and synthetic carrier comments preserved as
//!   `# @hub:<C>.<path>=<value>`.
//! - Two-space indent (the catalog standard).
//!
//! Out of scope (deliberate, follow-up work):
//! - Anchors / merge keys — the reverse cascade currently doesn't
//!   reconstruct them.
//! - Block-scalar styles (`|`, `>`) — emitted as plain.
//! - Flow style (`[a, b]`, `{k: v}`) — only block form.
//! - Quote-style fidelity — scalars round-tripped as plain unless
//!   they need quoting for YAML well-formedness.
//!
//! Remaining gap (not an engine limitation, by-design constraint):
//! the seesaw engine only materialises R-creation nodes that are
//! the target of a creation-corr. The forward ruleset relies on
//! shared-anchor L nodes that have no corr (e.g. cst:MappingEntry,
//! cst:Sequence, cst:SequenceItem) — they exist on the L side via
//! the seeder, no rule needs to create them. After reversal those
//! same nodes flip to R-creation and would never materialise
//! without a corr. The `reverse::reverse_rule` helper now synthesises
//! a corr for each such orphan; emit walks the resulting subgraph
//! normally. The `key`-attribute on materialised cst:MappingEntry
//! nodes is not yet propagated end-to-end (the engine's attrs_to_set
//! path is in play but the value isn't surfacing in the final
//! graph — separate diagnostic).

use std::fmt::Write;

use seesaw_core::graph::{GhostId, TypedGraph};

use crate::{
    cst_attr, CST_CARRIER_COMMENT, CST_HAS_CHILD, CST_MAPPING, CST_MAPPING_ENTRY, CST_SCALAR,
    CST_SEQUENCE, CST_SEQUENCE_ITEM, CST_USER_COMMENT, CST_VALUE_OF,
};

/// Emit YAML for the subgraph rooted at `root` (typically the
/// outermost `cst:Mapping` for a pipeline).
///
/// Returns the YAML source as a string. Does not include a trailing
/// document marker; callers append `---` separators if needed.
#[must_use]
pub fn emit_yaml(graph: &TypedGraph, root: GhostId) -> String {
    let mut out = String::new();
    let mut state = EmitState::default();
    emit_node(graph, root, 0, &mut state, &mut out);
    if !out.ends_with('\n') {
        out.push('\n');
    }
    out
}

#[derive(Default)]
struct EmitState {
    /// Current recursion ancestors. Used to suppress real cycles
    /// (a mapping is its own descendant via the bitbucket
    /// self-ref or a graph artefact).
    path: std::collections::HashSet<GhostId>,
    /// Construct-tagged mappings (cst:Mapping with a `construct`
    /// attribute = an IR construct like job, step, pipeline) that
    /// have already been emitted as full content. The reverse
    /// cascade routinely produces two routes to the same
    /// sub-construct:
    ///   * a field-rule wraps it in `<field>: [- {sub}]`
    ///   * implicit-containment exposes it as `<name>: {sub}`
    ///
    /// Whichever the walker reaches first writes the full body;
    /// the other reference is dropped so the YAML isn't doubled.
    /// Container mappings (untagged, e.g. the `{build: ...}` map
    /// under circleci `jobs:`) are NOT in this set — they're
    /// allowed to be walked from any parent.
    emitted_constructs: std::collections::HashSet<GhostId>,
}

const MAX_EMIT_DEPTH: usize = 64;

fn emit_node(
    graph: &TypedGraph,
    id: GhostId,
    indent: usize,
    state: &mut EmitState,
    out: &mut String,
) {
    if indent / 2 > MAX_EMIT_DEPTH {
        return;
    }
    let Some(nd) = graph.get_node(&id) else {
        return;
    };
    match nd.type_id.as_str() {
        CST_MAPPING => emit_mapping(graph, id, indent, state, out),
        CST_SEQUENCE => emit_sequence(graph, id, indent, state, out),
        CST_SCALAR => emit_scalar(graph, id, out),
        _ => {} // unknown / lexical — ignore
    }
}

#[allow(
    clippy::too_many_lines,
    reason = "one match over CST node kinds; splitting would scatter the emit logic"
)]
fn emit_mapping(
    graph: &TypedGraph,
    id: GhostId,
    indent: usize,
    // `path` is the set of mappings currently on the recursion
    // stack from the emit root — true ancestors only. A node
    // recurring as its own descendant is a real cycle; a node
    // recurring elsewhere in the document (shared anchor, multi-
    // referenced sub-construct) is legitimate sharing and should
    // emit normally. The set is push/pop on entry/exit so it
    // tracks the active path rather than every node ever visited.
    state: &mut EmitState,
    out: &mut String,
) {
    if !state.path.insert(id) {
        // Real cycle (node is its own ancestor): bail without a
        // placeholder.
        return;
    }
    // If THIS mapping is a construct, mark it emitted so subsequent
    // references to it (from anywhere — same parent or different
    // parent) get suppressed.
    let is_construct = graph
        .get_node(&id)
        .and_then(|n| n.attrs.get(cst_attr::CONSTRUCT))
        .is_some_and(|c| !c.is_empty());
    if is_construct {
        state.emitted_constructs.insert(id);
    }
    let entries = child_ids_of_kind(graph, id, CST_HAS_CHILD, CST_MAPPING_ENTRY);
    let carriers = child_ids_of_kind(graph, id, CST_HAS_CHILD, CST_CARRIER_COMMENT);
    let user_comments = child_ids_of_kind(graph, id, CST_HAS_CHILD, CST_USER_COMMENT);

    // User `# foo` comments first — preserved verbatim from the
    // source, anchored above the mapping they belonged to. Dedupe
    // on (text, span_start) to suppress reverse-cascade duplicates
    // (a duplicate is the SAME source comment re-contributed by
    // multiple rules → identical text AND byte span). Keying on text
    // alone over-collapsed distinct comments that happen to share
    // text — notably the blank `#` lines in a license header (text
    // = "") fold into one, dropping the rest (gcb basic-config).
    let mut seen_user: std::collections::HashSet<(String, String)> =
        std::collections::HashSet::new();
    for cid in &user_comments {
        let (text, span) = graph
            .get_node(cid)
            .map(|n| {
                (
                    n.attrs.get(cst_attr::TEXT).cloned().unwrap_or_default(),
                    n.attrs
                        .get(cst_attr::SPAN_START)
                        .cloned()
                        .unwrap_or_default(),
                )
            })
            .unwrap_or_default();
        if seen_user.insert((text, span)) {
            emit_user_comment(graph, *cid, indent, out);
        }
    }
    // Carrier comments next — dedupe on (target_path, value).
    let mut seen_carrier: std::collections::HashSet<(String, String)> =
        std::collections::HashSet::new();
    for cid in &carriers {
        let (cpath, val) = graph
            .get_node(cid)
            .map(|n| {
                (
                    n.attrs
                        .get(cst_attr::TARGET_PATH)
                        .cloned()
                        .unwrap_or_default(),
                    n.attrs.get(cst_attr::VALUE).cloned().unwrap_or_default(),
                )
            })
            .unwrap_or_default();
        if seen_carrier.insert((cpath, val)) {
            emit_carrier(graph, *cid, indent, out);
        }
    }

    // A wholly empty root mapping (e.g. an empty pipeline reconstructed
    // from `{}`) must still emit visible structure — an empty string would
    // fail to reparse as a pipeline. Flow-style `{}` round-trips: the
    // parser reads it back, open_pipeline treats it as an empty pipeline.
    // Nested empty mappings are handled by their parent entry below.
    if indent == 0 && entries.is_empty() && carriers.is_empty() && user_comments.is_empty() {
        out.push_str("{}");
        state.path.remove(&id);
        return;
    }

    // Dedupe entries by key. The reverse cascade can leave more
    // than one cst:MappingEntry with the same key under one parent
    // when several rules contribute the same logical entry from
    // different paths (e.g. an implicit-containment rule and a
    // ref-carrier rule both materialising `build`). The first
    // structurally complete one wins; the rest are skipped.
    let mut seen_keys: std::collections::HashSet<String> = std::collections::HashSet::new();
    for entry_id in entries {
        let Some(entry) = graph.get_node(&entry_id) else {
            continue;
        };
        let key = entry.attrs.get(cst_attr::KEY).cloned().unwrap_or_default();
        let Some(value_id) = outgoing_target(graph, entry_id, CST_VALUE_OF) else {
            continue;
        };
        // Skip second-occurrence keys (the reverse cascade leaves
        // more than one cst:MappingEntry with the same key under
        // one parent when several rules contribute the same logical
        // entry from different paths — e.g. an implicit-containment
        // rule and a ref-carrier rule both materialising `build`).
        if !seen_keys.insert(key.clone()) {
            continue;
        }
        // Skip mapping-valued entries whose value is on the
        // active recursion path (true ancestor cycle). Sharing
        // (a value reachable from multiple keys at different
        // depths) is legitimate and stays.
        let value_is_mapping = graph
            .get_node(&value_id)
            .is_some_and(|n| n.type_id == CST_MAPPING);
        if value_is_mapping {
            // Real cycle (ancestor): skip without placeholder.
            if state.path.contains(&value_id) {
                continue;
            }
            // Construct already emitted as content elsewhere: skip
            // the alternative wrapper for it (field-rule path vs.
            // implicit-containment path produce two entries for the
            // same hub-construct sub-mapping).
            if state.emitted_constructs.contains(&value_id) {
                continue;
            }
        }
        if key.is_empty() {
            // Render the inner value at the same indent without a
            // surrounding key — the unwrapped mapping/sequence
            // shape preserves the semantic content.
            let Some(value) = graph.get_node(&value_id) else {
                continue;
            };
            match value.type_id.as_str() {
                CST_MAPPING => emit_mapping(graph, value_id, indent, state, out),
                CST_SEQUENCE => emit_sequence(graph, value_id, indent, state, out),
                _ => {}
            }
            continue;
        }
        let Some(value) = graph.get_node(&value_id) else {
            continue;
        };
        write_indent(indent, out);
        out.push_str(&yaml_key(&key));
        out.push(':');
        match value.type_id.as_str() {
            CST_SCALAR => {
                let text = value.attrs.get(cst_attr::TEXT).cloned().unwrap_or_default();
                if text.is_empty() {
                    out.push('\n');
                } else {
                    out.push(' ');
                    out.push_str(&yaml_scalar(&text));
                    out.push('\n');
                }
            }
            CST_MAPPING if name_only_image_scalar(graph, value_id).is_some() => {
                // Name-only image ref (`image: ubuntu`): emit inline as a
                // scalar, not a `{name: ubuntu}` block (see
                // name_only_image_scalar).
                let name = name_only_image_scalar(graph, value_id).unwrap_or_default();
                out.push(' ');
                out.push_str(&yaml_scalar(&name));
                out.push('\n');
            }
            CST_MAPPING => {
                // Job-block round-trip safety. When the inner
                // mapping has no native entries (only carrier
                // comments naming the construct), pipeline-cst
                // tokenises `name:\n  # @hub:...=name` as a
                // scalar value — the second forward then never
                // sees a Mapping and never tags it
                // construct=job. Force flow-style `{}` so the
                // parser commits to "this is an empty mapping".
                //
                // Narrow trigger: only when there are no
                // MappingEntry children AND the only carriers
                // are name-carriers matching the outer key. This
                // avoids regressing bitbucket / other platforms
                // where an empty `pipelines:` mapping would
                // otherwise be re-classified as a nested
                // construct.
                let has_entries =
                    child_ids_of_kind(graph, value_id, CST_HAS_CHILD, CST_MAPPING_ENTRY)
                        .iter()
                        .any(|eid| outgoing_target(graph, *eid, CST_VALUE_OF).is_some());
                let carriers_v =
                    child_ids_of_kind(graph, value_id, CST_HAS_CHILD, CST_CARRIER_COMMENT);
                let user_comments_v =
                    child_ids_of_kind(graph, value_id, CST_HAS_CHILD, CST_USER_COMMENT);
                let only_redundant_name_carrier = !has_entries && {
                    !carriers_v.is_empty()
                        && carriers_v.iter().all(|cid| {
                            graph.get_node(cid).is_some_and(|n| {
                                n.attrs
                                    .get(cst_attr::TARGET_FIELD)
                                    .map(std::string::String::as_str)
                                    == Some("name")
                                    && n.attrs.get(cst_attr::VALUE) == Some(&key)
                            })
                        })
                };
                // First-class scalar fields ride the mapping as plain
                // attributes (no MappingEntry child) and emit as entries below
                // — a mapping carrying them is NOT empty even with no entries.
                let has_first_class_attrs = graph.get_node(&value_id).is_some_and(|n| {
                    n.attrs
                        .iter()
                        .any(|(k, v)| !is_reserved_attr(k) && !v.is_empty())
                });
                if !has_entries
                    && carriers_v.is_empty()
                    && user_comments_v.is_empty()
                    && !has_first_class_attrs
                {
                    // A truly empty mapping value (e.g. buildkite `waiter: {}`,
                    // a block field whose presence is its only content) must
                    // emit flow-style `{}` — a bare `key:` reparses as null,
                    // not an empty Mapping, so the block field's identity would
                    // be lost on the next forward (the seeder normalises the
                    // `{}`-scalar back to an empty Mapping; block_attr captures
                    // it).
                    out.push_str(" {}\n");
                } else if only_redundant_name_carrier {
                    // pipeline-cst treats inline `{}` as
                    // Scalar(FlowMap), not Mapping — the seeder
                    // wouldn't tag it as construct=job and the
                    // job would be lost on the second forward.
                    // Emit a real `name: <key>` block entry
                    // instead: forward A also creates a
                    // hub:attr[name=name, value=<key>] via the
                    // implicit-containment rule (S↔a key-to-value
                    // binding), so the second forward observes
                    // the same satellite — no IR drift.
                    out.push('\n');
                    write_indent(indent + 2, out);
                    let _ = writeln!(out, "name: {}", yaml_scalar(&key));
                } else {
                    out.push('\n');
                    emit_mapping(graph, value_id, indent + 2, state, out);
                }
            }
            CST_SEQUENCE => {
                out.push('\n');
                let has_items =
                    !child_ids_of_kind(graph, value_id, CST_HAS_CHILD, "cst:SequenceItem")
                        .is_empty();
                if has_items {
                    emit_sequence(graph, value_id, indent + 2, state, out);
                } else {
                    // Empty cst:Sequence — its per-item constructs were
                    // claimed as cst:Mapping backward (rc8 kind-mismatch).
                    // Derive the items from the hub:collection instead.
                    emit_hub_collection_items(graph, value_id, indent + 2, state, out);
                }
            }
            _ => out.push('\n'),
        }
    }
    // FIRST-CLASS scalar fields (rc8 re-architecture): a construct mapping
    // carries its scalar field values as plain node attributes — lifted by
    // `lift_scalar_fields` forward, propagated back onto the cst:Mapping by
    // the first-class field rules backward (NO MappingEntry→Scalar child).
    // Emit each non-reserved attribute that isn't already an explicit entry
    // as a `key: value` line. Sorted for deterministic output.
    if is_construct {
        let construct = graph
            .get_node(&id)
            .and_then(|n| n.attrs.get(cst_attr::CONSTRUCT).cloned())
            .unwrap_or_default();
        let name_keyed = is_name_keyed_construct(&construct);
        if let Some(node) = graph.get_node(&id) {
            let mut field_keys: Vec<&String> = node
                .attrs
                .keys()
                .filter(|k| {
                    // a name-keyed construct's `name` is the parent's
                    // containment key (emitted by the parent below), not a
                    // child `name:` entry.
                    let is_parent_name_key = name_keyed && k.as_str() == "name";
                    !is_reserved_attr(k) && !seen_keys.contains(k.as_str()) && !is_parent_name_key
                })
                .collect();
            field_keys.sort();
            for k in field_keys {
                let val = node.attrs.get(k).cloned().unwrap_or_default();
                // A first-class field rule fires for every construct and
                // propagates MC.<key> — absent fields arrive as "" (rc8
                // collect_propagated_attrs' unwrap_or_default). Skip empties
                // so an absent field is NOT emitted as a spurious entry.
                if val.is_empty() {
                    continue;
                }
                write_indent(indent, out);
                out.push_str(&yaml_key(k));
                out.push_str(": ");
                out.push_str(&yaml_scalar(&val));
                out.push('\n');
            }
        }
        // NAME-KEYED CONTAINMENT (rc8 first-class, EMIT-DERIVED): nest each
        // named child construct as `<child.name>:` by following the hub
        // has_job/has_pipeline edge via corrs — there is no cst containment
        // entry node (the satellite that used to anchor it is gone). Mirrors
        // how scalar field entries are derived from construct attributes.
        if let Some(hub) = cst_to_hub(graph, id) {
            let children: Vec<(GhostId, String)> = graph
                .iter_edges()
                .into_iter()
                .filter(|(s, _, e)| *s == hub && is_name_keyed_containment_edge(&e.type_id))
                .filter_map(|(_, child_hub, _)| {
                    let cst_child = hub_to_cst(graph, child_hub)?;
                    let name = graph
                        .get_node(&child_hub)
                        .and_then(|n| n.attrs.get("name").cloned())
                        .unwrap_or_default();
                    (!name.is_empty()).then_some((cst_child, name))
                })
                .collect();
            for (cst_child, name) in children {
                if state.path.contains(&cst_child) || state.emitted_constructs.contains(&cst_child)
                {
                    continue;
                }
                write_indent(indent, out);
                out.push_str(&yaml_key(&name));
                out.push_str(":\n");
                emit_mapping(graph, cst_child, indent + 2, state, out);
            }
        }
    }
    state.path.remove(&id);
}

/// Constructs entered by NAME (their parent map-entry key IS the construct's
/// name, e.g. gitlab `build:`). For these, `name` is the CONTAINMENT key
/// (the parent derives `<name>:` from has_<kind> + hub.name) — NOT a child
/// `name:` entry. Field-entered constructs (artifact, …) keep `name` as a
/// real field.
fn is_name_keyed_construct(c: &str) -> bool {
    matches!(c, "job" | "pipeline")
}

/// A `hub:image` is DUAL-NATURE: an object (`jobContainer: {image, env, …}`,
/// reconstructed as a full cst:Mapping by the image identity rule) OR a bare
/// string (`image: ubuntu`, a scalar_node ref → a name-only hub:image). The
/// backward identity rule always rebuilds the cst as a Mapping, so a name-only
/// image would otherwise emit as a `{name: …}` block and fail to re-seed as
/// the original scalar. When the cst:Mapping's hub pendant is a name-only
/// image (a `name`, no other fields, no field edges), return that name so emit
/// can render it inline as `key: <name>`. Scoped to `image` — the only
/// construct with both a non-empty `[image.maps]` object form AND scalar_node
/// string usage; map-keyed constructs (service, …) must stay block-rendered.
fn name_only_image_scalar(graph: &TypedGraph, cst_mapping: GhostId) -> Option<String> {
    let node = graph.get_node(&cst_mapping)?;
    if node.attrs.get(cst_attr::CONSTRUCT).map(String::as_str) != Some("image") {
        return None;
    }
    let hub = cst_to_hub(graph, cst_mapping)?;
    let hubn = graph.get_node(&hub)?;
    let name = hubn.attrs.get("name").filter(|v| !v.is_empty())?.clone();
    if hubn
        .attrs
        .iter()
        .any(|(k, v)| !is_reserved_attr(k) && k != "name" && !v.is_empty())
    {
        return None;
    }
    let has_fields = graph.iter_edges().into_iter().any(|(s, _, e)| {
        s == hub
            && matches!(
                e.type_id.as_str(),
                "hub:has_attr" | "hub:has_value" | "hub:has_item"
            )
    });
    if has_fields {
        return None;
    }
    Some(name)
}

/// Hub edge kinds that are NAME-KEYED containment (parent → named child
/// construct), as opposed to field edges (has_attr/has_value/has_item) or
/// field-keyed construct edges (has_artifact, …). emit derives `<child
/// name>:` nesting from these via the hub graph (rc8 first-class: no cst
/// containment entry node).
fn is_name_keyed_containment_edge(kind: &str) -> bool {
    matches!(kind, "hub:has_job" | "hub:has_pipeline")
}

/// cst:Mapping → its hub correspondent: cst <-corrR- tgg:refines <-corrL- hub.
fn cst_to_hub(graph: &TypedGraph, cst_id: GhostId) -> Option<GhostId> {
    let refines = graph
        .iter_edges()
        .into_iter()
        .find(|(_, t, e)| *t == cst_id && e.type_id == "corrR")
        .map(|(s, _, _)| s)?;
    graph
        .iter_edges()
        .into_iter()
        .find(|(_, t, e)| *t == refines && e.type_id == "corrL")
        .map(|(s, _, _)| s)
}

/// hub node → its cst correspondent: hub -corrL-> tgg:refines -corrR-> cst.
fn hub_to_cst(graph: &TypedGraph, hub_id: GhostId) -> Option<GhostId> {
    let refines = graph
        .iter_edges()
        .into_iter()
        .find(|(s, _, e)| *s == hub_id && e.type_id == "corrL")
        .map(|(_, t, _)| t)?;
    graph
        .iter_edges()
        .into_iter()
        .find(|(s, _, e)| *s == refines && e.type_id == "corrR")
        .map(|(_, t, _)| t)
}

/// EMIT-DERIVED scalar-list-of-constructs items (rc8 first-class): when a
/// cst:Sequence reconstructs EMPTY backward (its per-item construct got
/// claimed as a cst:Mapping by the construct identity rule — kind mismatch
/// under rc8 reuse), derive the items from the corresponding hub:collection
/// instead. Each item construct (e.g. hub:step from a gitlab `script:` line)
/// is rendered by FORM: a single value-bearing satellite → a scalar `- v`;
/// otherwise a nested mapping `- {…}` via its cst correspondent.
fn emit_hub_collection_items(
    graph: &TypedGraph,
    cst_seq: GhostId,
    indent: usize,
    state: &mut EmitState,
    out: &mut String,
) {
    let Some(coll) = cst_to_hub(graph, cst_seq) else {
        return;
    };
    let items: Vec<GhostId> = graph
        .iter_edges()
        .into_iter()
        .filter(|(s, _, e)| *s == coll && e.type_id == "hub:has_item")
        .map(|(_, t, _)| t)
        .collect();
    for item in items {
        // scalar form: the item construct carries a single value-bearing
        // satellite (hub:attr with a `value`) — render its value as a scalar.
        let scalar_val = graph
            .iter_edges()
            .into_iter()
            .filter(|(s, _, e)| *s == item && e.type_id == "hub:has_attr")
            .find_map(|(_, a, _)| {
                graph
                    .get_node(&a)
                    .and_then(|n| n.attrs.get(cst_attr::VALUE).cloned())
            });
        write_indent(indent, out);
        out.push_str("- ");
        if let Some(v) = scalar_val {
            out.push_str(&yaml_scalar(&v));
            out.push('\n');
        } else {
            // structured item → render its cst mapping nested under `- `.
            out.push('\n');
            if let Some(cst_item) = hub_to_cst(graph, item) {
                emit_mapping(graph, cst_item, indent + 2, state, out);
            }
        }
    }
}

/// Attributes that are structural/provenance bookkeeping on a construct
/// `cst:Mapping` — NOT first-class IR fields — so they must never be emitted
/// as YAML entries. Everything else on a construct mapping is a lifted field.
///
/// CRUCIAL: this set is restricted to attrs the seeder actually places on a
/// `cst:Mapping`. Entry-/scalar-/carrier-structural names (`key`, `value`,
/// `text`, `index`, `entry_role`, `scalar_style`, `target_*`) live on
/// cst:MappingEntry / cst:Scalar / cst:CarrierComment, NEVER on a Mapping —
/// reserving them here wrongly dropped IR FIELDS that happen to share the
/// name (gitlab `cache.key` → the `key:` entry vanished because "key" was
/// reserved). They are deliberately NOT reserved.
fn is_reserved_attr(k: &str) -> bool {
    matches!(
        k,
        "construct"
            | "construct_name"
            | "span_start"
            | "span_end"
            | "parent_key"
            | "source_file"
            | "from_merge"
            | "merged_from_anchor"
            | "anchor"
            | "prov_byte_start"
            | "prov_byte_end"
    )
}

fn emit_sequence(
    graph: &TypedGraph,
    id: GhostId,
    indent: usize,
    state: &mut EmitState,
    out: &mut String,
) {
    if !state.path.insert(id) {
        return;
    }
    let items = child_ids_of_kind(graph, id, CST_HAS_CHILD, CST_SEQUENCE_ITEM);
    if items.is_empty() {
        write_indent(indent, out);
        out.push_str("[]\n");
        state.path.remove(&id);
        return;
    }
    for item_id in items {
        let Some(value_id) = outgoing_target(graph, item_id, CST_VALUE_OF) else {
            continue;
        };
        let Some(value) = graph.get_node(&value_id) else {
            continue;
        };
        write_indent(indent, out);
        out.push_str("- ");
        match value.type_id.as_str() {
            CST_SCALAR => {
                let text = value.attrs.get(cst_attr::TEXT).cloned().unwrap_or_default();
                out.push_str(&yaml_scalar(&text));
                out.push('\n');
            }
            CST_MAPPING => {
                // Inline the first key on the same line as `- `, the
                // rest indented one extra level. This is the standard
                // YAML block-sequence-of-mappings shape.
                out.push('\n');
                emit_mapping(graph, value_id, indent + 2, state, out);
            }
            CST_SEQUENCE => {
                out.push('\n');
                emit_sequence(graph, value_id, indent + 2, state, out);
            }
            _ => out.push('\n'),
        }
    }
    state.path.remove(&id);
}

fn emit_scalar(graph: &TypedGraph, id: GhostId, out: &mut String) {
    if let Some(nd) = graph.get_node(&id) {
        if let Some(text) = nd.attrs.get(cst_attr::TEXT) {
            out.push_str(&yaml_scalar(text));
        }
    }
}

fn emit_user_comment(graph: &TypedGraph, id: GhostId, indent: usize, out: &mut String) {
    let Some(nd) = graph.get_node(&id) else {
        return;
    };
    let text = nd.attrs.get(cst_attr::TEXT).cloned().unwrap_or_default();
    write_indent(indent, out);
    let _ = writeln!(out, "# {text}");
}

fn emit_carrier(graph: &TypedGraph, id: GhostId, indent: usize, out: &mut String) {
    let Some(nd) = graph.get_node(&id) else {
        return;
    };
    let construct = nd
        .attrs
        .get(cst_attr::TARGET_CONSTRUCT)
        .cloned()
        .unwrap_or_default();
    let path = nd
        .attrs
        .get(cst_attr::TARGET_PATH)
        .or_else(|| nd.attrs.get(cst_attr::TARGET_FIELD))
        .cloned()
        .unwrap_or_default();
    let value = nd.attrs.get(cst_attr::VALUE).cloned().unwrap_or_default();
    write_indent(indent, out);
    let _ = writeln!(out, "# @hub:{construct}.{path}={}", yaml_scalar(&value));
}

// ─── helpers ──────────────────────────────────────────────────────

fn child_ids_of_kind(
    graph: &TypedGraph,
    parent: GhostId,
    edge_kind: &str,
    node_kind: &str,
) -> Vec<GhostId> {
    let mut ids: Vec<GhostId> = graph
        .iter_edges()
        .into_iter()
        // Skip Ghost-overlay-deleted edges: a re-emit that re-shapes the CST
        // (e.g. wrapping argo `tasks:` under `dag:`) tombstones the old edge.
        .filter(|(s, _, e)| {
            *s == parent
                && e.type_id == edge_kind
                && !matches!(
                    e.status,
                    seesaw_core::graph::Status::Tombstone
                        | seesaw_core::graph::Status::TentativeTombstone
                )
        })
        .filter_map(|(_, t, _)| {
            graph
                .get_node(&t)
                .filter(|n| n.type_id == node_kind)
                .map(|_| t)
        })
        .collect();
    // Deterministic order: sort by span_start when present, else by
    // ghost-id hash. The reverse cascade doesn't preserve source order
    // (there is no source), so span_start may be absent — that's fine,
    // the BTree ordering of ghost-ids gives us a stable result.
    ids.sort_by_key(|id| span_start_or_zero(graph, *id));
    ids
}

fn span_start_or_zero(graph: &TypedGraph, id: GhostId) -> usize {
    graph
        .get_node(&id)
        .and_then(|n| n.attrs.get(cst_attr::SPAN_START))
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(0)
}

fn outgoing_target(graph: &TypedGraph, source: GhostId, edge_kind: &str) -> Option<GhostId> {
    graph
        .iter_edges()
        .into_iter()
        .find(|(s, _, e)| *s == source && e.type_id == edge_kind)
        .map(|(_, t, _)| t)
}

fn write_indent(n: usize, out: &mut String) {
    for _ in 0..n {
        out.push(' ');
    }
}

/// Render a YAML key — quote only if it contains characters that
/// would confuse the block-key tokenizer.
fn yaml_key(k: &str) -> String {
    if k.is_empty()
        || k.chars()
            .any(|c| matches!(c, ':' | '#' | '\n' | '[' | ']' | '{' | '}'))
    {
        format!("{k:?}") // Debug-quoted is JSON-style, valid YAML
    } else {
        k.to_string()
    }
}

/// Render a YAML scalar — quote only if it could be misread by the
/// parser (leading sigils, trailing colon, embedded comment marker).
fn yaml_scalar(s: &str) -> String {
    if s.is_empty() {
        return "''".to_string();
    }
    let first = s.chars().next().unwrap();
    let needs_quote = matches!(
        first,
        '-' | '?' | ':' | '&' | '*' | '!' | '|' | '>' | '%' | '@' | '`' | '{' | '[' | ',' | '#'
    )
        // A leading `{`/`[` makes YAML parse the value as a flow collection,
        // so `{{workflow.parameters.revision}}`-style templating must be quoted.
        || s.contains(": ")
        // A TRAILING colon turns a block-sequence item into a mapping key:
        // `- scp -r $PWD vm:` parses as `{"scp -r $PWD vm": null}`, dropping the
        // command (libinput). `contains(": ")` misses it (no following space).
        || s.ends_with(':')
        || s.contains(" #")
        || s.contains('\n');
    if needs_quote {
        format!("{s:?}")
    } else {
        s.to_string()
    }
}
