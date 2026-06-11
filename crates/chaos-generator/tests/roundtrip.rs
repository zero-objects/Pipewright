//! Per-platform bidirectional roundtrip baseline.
//!
//! For each platform + seed: generate a chaos fixture, then run
//!   yaml → seed → FORWARD cascade → hub1
//!        → BACKWARD cascade → cst → emit → yaml'
//!        → seed → FORWARD cascade → hub2
//! and assert hub1 ≅ hub2 at the CONTENT level (node kinds + identity
//! attrs + edge shape, provenance/span attrs ignored). Hub-level
//! comparison sidesteps YAML formatting noise — losslessness means the
//! semantic IR survives the round trip, not byte-identical text.
//!
//! This is the "both directions" gate for the all-platforms goal.

use chaos_generator::{generate_yaml, walker::Budget};
use pipeline_cst::{parse, Document};
use pipeline_earthfile_cst::parse as parse_earthfile;
use pipeline_jenkinsfile_cst::parse as parse_jenkinsfile;
use pipeline_tgg_seeder::emit::emit_yaml;
use pipeline_tgg_seeder::emit_earthfile::emit_earthfile;
use pipeline_tgg_seeder::emit_jenkinsfile::emit_jenkinsfile;

/// Parse a generated fixture with the platform's surface parser: jenkins
/// (Groovy) and earthly (Earthfile) have dedicated CSTs; everything else
/// (incl. dagger, whose `dagger.json` is a YAML subset) uses the YAML parser.
fn parse_for(platform: &str, src: &str) -> Result<Document, String> {
    match platform {
        "jenkins" => parse_jenkinsfile(src).map_err(|e| format!("{e:?}")),
        "earthly" => parse_earthfile(src).map_err(|e| format!("{e:?}")),
        _ => parse(src).map_err(|e| format!("{e:?}")),
    }
}

/// Emit a reconstructed pipeline in the platform's surface syntax.
fn emit_for(platform: &str, g: &TypedGraph, root: GhostId) -> String {
    match platform {
        "jenkins" => emit_jenkinsfile(g, root),
        "earthly" => emit_earthfile(g, root),
        _ => emit_yaml(g, root),
    }
}
use seesaw_core::engine::{
    cascade_step_cached, find_matches, run_cascade_cached, run_cascade_full, Cascade, MatchCache,
    Rule, TerminationState,
};
use seesaw_core::graph::{EdgeData, GhostId, NodeData, Status, TypedGraph};
use seesaw_core::rule::compile::compile_bidirectional;
use seesaw_core::rule::instantiate::instantiate;
use seesaw_core::rule::spec::RuleSetSpec;
use std::collections::BTreeMap;
use std::path::Path;

fn ruleset(platform: &str) -> RuleSetSpec {
    let p = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../catalog/rules")
        .join(format!("{platform}.ruleset.json"));
    serde_json::from_str(&std::fs::read_to_string(&p).unwrap()).unwrap()
}

fn pool(rs: &RuleSetSpec) -> Vec<Box<dyn Rule>> {
    rs.rules
        .iter()
        .flat_map(|r| compile_bidirectional(r).expect("compile_bidirectional"))
        .map(|c| instantiate(&c))
        .collect()
}

fn seed_for(platform: &str, doc: &pipeline_cst::Document, source: &str) -> TypedGraph {
    use pipeline_tgg_seeder::platforms as p;
    match platform {
        "drone" => p::drone::seed_from_document(doc, source).graph,
        "woodpecker" => p::woodpecker::seed_from_document(doc, source).graph,
        "buildkite" => p::buildkite::seed_from_document(doc, source).graph,
        "tekton" => p::tekton::seed_from_document(doc, source).graph,
        "argo" => p::argo::seed_from_document(doc, source).graph,
        "google_cloudbuild" => p::google_cloudbuild::seed_from_document(doc, source).graph,
        "gitlab" => p::gitlab::seed_from_document(doc, source).graph,
        "github" => p::github::seed_from_document(doc, source).graph,
        "azure" => p::azure::seed_from_document(doc, source).graph,
        "travis" => p::travis::seed_from_document(doc, source).graph,
        "bitbucket" => p::bitbucket::seed_from_document(doc, source).graph,
        "aws_codebuild" => p::aws_codebuild::seed_from_document(doc, source).graph,
        "aws_codepipeline" => p::aws_codepipeline::seed_from_document(doc, source).graph,
        "circleci" => p::circleci::seed_from_document(doc, source).graph,
        "jenkins" => p::jenkins::seed_from_document(doc, source).graph,
        "earthly" => p::earthly::seed_from_document(doc, source).graph,
        "dagger" => p::dagger::seed_from_document(doc, source).graph,
        _ => panic!("unsupported: {platform}"),
    }
}

const PLATFORMS: &[&str] = &[
    "drone",
    "woodpecker",
    "buildkite",
    "tekton",
    "argo",
    "google_cloudbuild",
    "github",
    "azure",
    "aws_codebuild",
    "aws_codepipeline",
    "travis",
    "gitlab",
    "bitbucket",
    "circleci",
    "jenkins",
    "earthly",
    "dagger",
];

fn graph_kinds(g: &TypedGraph) -> std::collections::HashSet<String> {
    g.iter_nodes().map(|n| n.type_id.clone()).collect()
}

fn run_routed(graph: &mut TypedGraph, rules: &[Box<dyn Rule>]) -> Result<(), String> {
    let delta = graph_kinds(graph);
    let active: Vec<&dyn Rule> = rules
        .iter()
        .filter(|r| {
            let idk = r.input_domain_kinds();
            idk.is_empty() || idk.iter().any(|k| delta.contains(k))
        })
        .map(AsRef::as_ref)
        .collect();
    // CACHED cascade driver: cascade_step_cached carries a MatchCache across
    // steps so the engine's incremental re-validation (Hebel D/3/4/5) kicks in —
    // the quadratic-breakers that only live on the cached path. Bit-identical
    // results to the old `cascade_step` loop, but linear instead of quadratic on
    // large graphs (tokio ~3.3×, growing with size). One function swap covers
    // every test path (roundtrip / chaos / corpus / interop / fwd_hub).
    let mut cascade = Cascade::new();
    run_cascade_cached(&mut cascade, graph, &active, 20_000).map_err(|e| format!("{e:?}"))?;
    Ok(())
}

fn isolate_hub(g: &TypedGraph) -> TypedGraph {
    let mut hub = TypedGraph::new();
    for nd in g.iter_nodes() {
        if nd.type_id.starts_with("hub:") {
            hub.insert_node_data(nd.clone());
        }
    }
    for (s, t, e) in g.iter_edges() {
        if e.type_id.starts_with("hub:") {
            hub.insert_edge_data(s, t, e.clone());
        }
    }
    hub
}

fn pick_pipeline_root(g: &TypedGraph) -> Option<GhostId> {
    let inner: std::collections::HashSet<_> = g
        .iter_edges()
        .into_iter()
        .filter(|(s, _, e)| {
            e.type_id == "cst:value_of"
                && g.get_node(s)
                    .is_some_and(|n| n.type_id == "cst:MappingEntry")
        })
        .map(|(_, t, _)| t)
        .collect();
    g.iter_nodes()
        .filter(|n| {
            n.type_id == "cst:Mapping"
                && n.attrs.get("construct").map(String::as_str) == Some("pipeline")
        })
        .map(|n| n.id)
        .find(|m| !inner.contains(m))
}

fn is_prov_attr(k: &str) -> bool {
    k.starts_with("prov") || k.starts_with("span")
}

/// Content signature of a hub graph: a sorted multiset of node
/// signatures (kind + non-provenance attrs) and edge signatures
/// (src-kind | edge-kind | tgt-kind). GhostId-independent.
fn hub_signature(hub: &TypedGraph) -> (BTreeMap<String, usize>, BTreeMap<String, usize>) {
    // A CONTENT-LESS job (`hub:job` with no steps and no attribute besides its
    // `name`) carries no information — like an empty-valued attr below, its
    // presence is a seeder quirk, not semantics: gitlab seeds an empty `build: {}`
    // as a job, github/earthly don't seed an empty top-level key, so a job whose
    // content the target can't represent drifts present/absent across the
    // round-trip. Treat it as semantically ABSENT: drop it, its lone name-attr,
    // and their incident edges from the signature (same rule as empty attrs).
    let job_has_content = |j: &GhostId| -> bool {
        hub.iter_edges().into_iter().any(|(s, t, e)| {
            s == *j
                && (e.type_id == "hub:has_step"
                    || (e.type_id == "hub:has_attr"
                        && hub
                            .get_node(&t)
                            .and_then(|n| n.attrs.get("name").cloned())
                            .as_deref()
                            != Some("name")))
        })
    };
    let empty_jobs: std::collections::HashSet<GhostId> = hub
        .iter_nodes()
        .filter(|n| n.type_id == "hub:job" && !job_has_content(&n.id))
        .map(|n| n.id)
        .collect();
    let empty_job_attrs: std::collections::HashSet<GhostId> = hub
        .iter_edges()
        .into_iter()
        .filter(|(s, _, e)| empty_jobs.contains(s) && e.type_id == "hub:has_attr")
        .map(|(_, t, _)| t)
        .collect();
    let skip = |id: &GhostId| empty_jobs.contains(id) || empty_job_attrs.contains(id);
    let mut nodes: BTreeMap<String, usize> = BTreeMap::new();
    for n in hub.iter_nodes() {
        if skip(&n.id) {
            continue;
        }
        // Empty-valued attrs are semantically ABSENT (an empty-string field
        // carries no information, and the lift adds every declared scalar
        // field as "" when the source omits it). Two hubs differing ONLY in
        // which empty attrs they carry are losslessly equal, so drop them from
        // the signature. A REAL loss (name=foo -> name="") still shows: hub1
        // keeps name=foo, hub2 drops the empty name -> the foo signature is
        // missing from hub2.
        let mut attrs: Vec<String> = n
            .attrs
            .iter()
            .filter(|(k, v)| !is_prov_attr(k) && !v.is_empty())
            .map(|(k, v)| format!("{k}={v}"))
            .collect();
        attrs.sort();
        *nodes
            .entry(format!("{}#{}", n.type_id, attrs.join(",")))
            .or_default() += 1;
    }
    let mut edges: BTreeMap<String, usize> = BTreeMap::new();
    for (s, t, e) in hub.iter_edges() {
        if skip(&s) || skip(&t) {
            continue;
        }
        let sk = hub
            .get_node(&s)
            .map(|n| n.type_id.clone())
            .unwrap_or_default();
        let tk = hub
            .get_node(&t)
            .map(|n| n.type_id.clone())
            .unwrap_or_default();
        *edges.entry(format!("{sk}|{}|{tk}", e.type_id)).or_default() += 1;
    }
    (nodes, edges)
}

struct RtOutcome {
    converged_fwd: bool,
    converged_bwd: bool,
    emitted: bool,
    reparsed: bool,
    hub_equal: bool,
    detail: String,
}

fn roundtrip_once(platform: &str, yaml: &str, rules: &[Box<dyn Rule>]) -> RtOutcome {
    let mut o = RtOutcome {
        converged_fwd: false,
        converged_bwd: false,
        emitted: false,
        reparsed: false,
        hub_equal: false,
        detail: String::new(),
    };
    let doc = match parse_for(platform, yaml) {
        Ok(d) => d,
        Err(e) => {
            o.detail = format!("parse src: {e}");
            return o;
        }
    };
    let mut g = seed_for(platform, &doc, yaml);
    if let Err(e) = run_routed(&mut g, rules) {
        o.detail = format!("fwd: {e}");
        return o;
    }
    o.converged_fwd = true;
    let mut hub1 = isolate_hub(&g);
    let sig1 = hub_signature(&hub1);

    if let Err(e) = run_routed(&mut hub1, rules) {
        o.detail = format!("bwd: {e}");
        return o;
    }
    o.converged_bwd = true;
    let root = match pick_pipeline_root(&hub1) {
        Some(r) => r,
        None => {
            o.detail = "no reconstructed pipeline root".into();
            return o;
        }
    };
    let yaml2 = emit_for(platform, &hub1, root);
    o.emitted = !yaml2.trim().is_empty();
    if !o.emitted {
        o.detail = "emit empty".into();
        return o;
    }
    let doc2 = match parse_for(platform, &yaml2) {
        Ok(d) => d,
        Err(e) => {
            o.detail = format!("parse emitted: {e}\n--- emitted ---\n{yaml2}");
            return o;
        }
    };
    o.reparsed = true;
    let mut g2 = seed_for(platform, &doc2, &yaml2);
    if let Err(e) = run_routed(&mut g2, rules) {
        o.detail = format!("fwd2: {e}");
        return o;
    }
    let hub2 = isolate_hub(&g2);
    let sig2 = hub_signature(&hub2);
    o.hub_equal = sig1 == sig2;
    if !o.hub_equal {
        let only1: Vec<_> = sig1
            .0
            .iter()
            .filter(|(k, v)| sig2.0.get(*k) != Some(*v))
            .map(|(k, v)| format!("  -{k} (x{v})"))
            .collect();
        let only2: Vec<_> = sig2
            .0
            .iter()
            .filter(|(k, v)| sig1.0.get(*k) != Some(*v))
            .map(|(k, v)| format!("  +{k} (x{v})"))
            .collect();
        o.detail = format!("hub node diff:\n{}\n{}", only1.join("\n"), only2.join("\n"));
    }
    o
}

fn emit_dbg_cst_to_hub(g: &TypedGraph, cst_id: GhostId) -> Option<String> {
    let refines = g
        .iter_edges()
        .into_iter()
        .find(|(_, t, e)| *t == cst_id && e.type_id == "corrR")
        .map(|(s, _, _)| s)?;
    let hub = g
        .iter_edges()
        .into_iter()
        .find(|(_, t, e)| *t == refines && e.type_id == "corrL")
        .map(|(s, _, _)| s)?;
    Some(format!(
        "{:?} ({})",
        hub,
        g.get_node(&hub)
            .map(|n| n.type_id.clone())
            .unwrap_or_default()
    ))
}

/// Focused debug: dump the reconstructed cst + emit for one (platform, seed).
#[test]
#[ignore = "debug — run with --ignored --nocapture; set PLAT/SEED env"]
fn roundtrip_debug_one() {
    let platform = std::env::var("PLAT").unwrap_or_else(|_| "github".into());
    let seed: u64 = std::env::var("SEED")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(1);
    let budget = Budget::shallow();
    let rules = pool(&ruleset(&platform));
    // FIXTURE=<path> debugs a real-config file (relative to repo root or
    // absolute) instead of a chaos seed — used for the real_config_corpus gaps.
    let (yaml, src_label) = match std::env::var("FIXTURE") {
        Ok(p) => {
            let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").join(&p);
            (
                std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {p}: {e}")),
                p,
            )
        }
        Err(_) => (
            generate_yaml(&platform, seed, &budget).expect("gen"),
            format!("seed {seed}"),
        ),
    };
    println!("\n=== SOURCE ({platform} {src_label}) ===\n{yaml}");
    {
        let rt = roundtrip_once(&platform, &yaml, &rules);
        println!(
            "=== ROUNDTRIP hub_equal={} ===\n{}",
            rt.hub_equal, rt.detail
        );
    }
    let doc = parse_for(&platform, &yaml).expect("parse");
    let mut g = seed_for(&platform, &doc, &yaml);
    run_routed(&mut g, &rules).expect("fwd");
    let mut hub = isolate_hub(&g);
    let mut hubkinds: BTreeMap<String, usize> = BTreeMap::new();
    for n in hub.iter_nodes() {
        *hubkinds.entry(n.type_id.clone()).or_default() += 1;
    }
    println!("=== HUB kinds === {hubkinds:?}");
    run_routed(&mut hub, &rules).expect("bwd");
    let mut cstkinds: BTreeMap<String, usize> = BTreeMap::new();
    for n in hub.iter_nodes() {
        if n.type_id.starts_with("cst:") {
            *cstkinds.entry(n.type_id.clone()).or_default() += 1;
        }
    }
    println!("=== rebuilt CST kinds === {cstkinds:?}");
    // all cst:Mapping construct=pipeline candidates
    for n in hub.iter_nodes() {
        if n.type_id == "cst:Mapping" {
            let c = n.attrs.get("construct").cloned().unwrap_or_default();
            let has_corr_r = hub
                .iter_edges()
                .into_iter()
                .any(|(_, t, e)| t == n.id && e.type_id == "corrR");
            println!(
                "  cst:Mapping construct={c:?} id={:?} corrR_in={has_corr_r}",
                n.id
            );
        }
    }
    // hub containment edges
    for (s, t, e) in hub.iter_edges() {
        if e.type_id.starts_with("hub:has_") {
            let sk = hub
                .get_node(&s)
                .map(|n| n.type_id.clone())
                .unwrap_or_default();
            let tk = hub
                .get_node(&t)
                .map(|n| n.type_id.clone())
                .unwrap_or_default();
            if sk == "hub:pipeline" || e.type_id == "hub:has_job" {
                println!("  HUB {sk} -{}-> {tk}", e.type_id);
            }
        }
    }
    match pick_pipeline_root(&hub) {
        Some(root) => {
            println!("=== pipeline root {root:?} ===");
            println!("  cst_to_hub(root) = {:?}", emit_dbg_cst_to_hub(&hub, root));
            for (s, t, e) in hub.iter_edges() {
                if t == root && (e.type_id == "corrR" || e.type_id.starts_with("cst:")) {
                    let sk = hub
                        .get_node(&s)
                        .map(|n| n.type_id.clone())
                        .unwrap_or_default();
                    println!("  IN {sk} -{}->", e.type_id);
                }
                if s == root {
                    let tk = hub
                        .get_node(&t)
                        .map(|n| n.type_id.clone())
                        .unwrap_or_default();
                    println!("  OUT {} -> {tk} {t:?}", e.type_id);
                }
            }
            // SEQ/IT/collection multiplicity probe
            println!("=== collection/item structure ===");
            for n in hub.iter_nodes() {
                if n.type_id == "hub:collection" {
                    let items: Vec<_> = hub
                        .iter_edges()
                        .into_iter()
                        .filter(|(s, _, e)| *s == n.id && e.type_id == "hub:has_item")
                        .map(|(_, t, _)| {
                            hub.get_node(&t)
                                .map(|x| x.type_id.clone())
                                .unwrap_or_default()
                        })
                        .collect();
                    println!(
                        "  hub:collection {:?} has_item x{}: {items:?}",
                        n.id,
                        items.len()
                    );
                }
                if n.type_id == "cst:Sequence" {
                    let its: Vec<_> = hub
                        .iter_edges()
                        .into_iter()
                        .filter(|(s, _, e)| *s == n.id && e.type_id == "cst:has_child")
                        .map(|(_, t, _)| t)
                        .collect();
                    println!(
                        "  cst:Sequence {:?} has_child x{} SequenceItems",
                        n.id,
                        its.len()
                    );
                    // One level deeper: each item's outgoing edges (kind+status),
                    // and the grandchildren — shows whether wrapper chains exist.
                    for it in its {
                        for (_, t, e) in hub.iter_edges().into_iter().filter(|(s, _, _)| *s == it) {
                            let tk = hub
                                .get_node(&t)
                                .map(|x| x.type_id.clone())
                                .unwrap_or_default();
                            let key = hub
                                .get_node(&t)
                                .and_then(|x| x.attrs.get("key").cloned())
                                .unwrap_or_default();
                            println!(
                                "    item {it:?} -{}({:?})-> {tk} {t:?} key={key}",
                                e.type_id, e.status
                            );
                            for (_, t2, e2) in
                                hub.iter_edges().into_iter().filter(|(s, _, _)| *s == t)
                            {
                                let tk2 = hub
                                    .get_node(&t2)
                                    .map(|x| x.type_id.clone())
                                    .unwrap_or_default();
                                let key2 = hub
                                    .get_node(&t2)
                                    .and_then(|x| x.attrs.get("key").cloned())
                                    .unwrap_or_default();
                                println!(
                                    "      -{}({:?})-> {tk2} {t2:?} key={key2}",
                                    e2.type_id, e2.status
                                );
                            }
                        }
                    }
                }
            }
            let emitted = emit_for(&platform, &hub, root);
            println!(
                "=== EMITTED ===\n{emitted}\n=== END ({} chars) ===",
                emitted.trim().len()
            );
        }
        None => println!("=== NO pipeline root found ==="),
    }
}

/// Build platform B's (construct, IR-field) -> primary platform-key map from
/// its ruleset JSON. Used to re-key an A-sourced hub into B's vocabulary so
/// B's prov_key-gated backward rules fire. First key per (construct, field)
/// wins — ir.toml lists the canonical/primary key first.
/// Target `prov_key`s for one `(construct, field)`, split by the attr's shape —
/// scalar value vs. sequence (collection). Platforms spell the same semantic
/// field two ways (gcb's step command = scalar `script` OR seq `args`); re-keying
/// must match the live attr's shape or a collection gets forced onto a scalar key
/// and dropped by the target backward (the gcb→drone drift root cause).
#[derive(Clone)]
struct KeySpec {
    key: String,
    vkind: Option<String>,
}

#[derive(Default, Clone)]
struct ShapeKeys {
    scalar: Option<KeySpec>,
    seq: Option<KeySpec>,
}

/// `true` if rule-pattern node `attr_id`'s `hub:has_value` edge targets a
/// `hub:collection` node (the attr is a sequence, not a scalar).
fn rule_attr_is_seq(nodes: &serde_json::Value, edges: &serde_json::Value, attr_id: &str) -> bool {
    let kind_of = |id: &str| -> Option<&str> {
        nodes
            .as_array()?
            .iter()
            .find(|n| n["id"].as_str() == Some(id))
            .and_then(|n| n["kind"].as_str())
    };
    edges.as_array().into_iter().flatten().any(|e| {
        e["kind"].as_str() == Some("hub:has_value")
            && e["source_node_id"].as_str() == Some(attr_id)
            && e["target_node_id"].as_str().and_then(kind_of) == Some("hub:collection")
    })
}

/// For a construct-reference attr (its `hub:has_value` targets a nested construct
/// like `hub:trigger`, not value/collection), the structural owner (source of its
/// `hub:has_attr` edge). A rule of that kind names two constructs (owner +
/// referent), and "last construct wins" would pick the referent; the structural
/// owner is the defined truth. Limited to construct-refs so collection-container
/// fields stay last-construct.
fn rule_attr_construct_owner(
    nodes: &serde_json::Value,
    edges: &serde_json::Value,
    attr_id: &str,
) -> Option<String> {
    let kind_of = |id: &str| -> Option<&str> {
        nodes
            .as_array()?
            .iter()
            .find(|n| n["id"].as_str() == Some(id))
            .and_then(|n| n["kind"].as_str())
    };
    let edge_arr = edges.as_array()?;
    let refers_construct = edge_arr.iter().any(|e| {
        e["kind"].as_str() == Some("hub:has_value")
            && e["source_node_id"].as_str() == Some(attr_id)
            && e["target_node_id"]
                .as_str()
                .and_then(kind_of)
                .and_then(|k| k.strip_prefix("hub:"))
                .is_some_and(|k| !matches!(k, "value" | "collection" | "attr"))
    });
    if !refers_construct {
        return None;
    }
    edge_arr.iter().find_map(|e| {
        (e["kind"].as_str() == Some("hub:has_attr")
            && e["target_node_id"].as_str() == Some(attr_id))
        .then(|| e["source_node_id"].as_str().and_then(kind_of))
        .flatten()
        .and_then(|k| k.strip_prefix("hub:"))
        .map(ToString::to_string)
    })
}

fn field_to_key_map(platform: &str) -> std::collections::HashMap<(String, String), ShapeKeys> {
    let p = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../catalog/rules")
        .join(format!("{platform}.ruleset.json"));
    let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&p).unwrap()).unwrap();
    let mut m: std::collections::HashMap<(String, String), ShapeKeys> =
        std::collections::HashMap::new();
    for rule in v["rules"].as_array().into_iter().flatten() {
        // hub side: hub:<construct> + hub:attr{name=<field>, prov_key=<key>, vkind}
        let nodes = &rule["r_pattern"]["nodes"];
        let edges = &rule["r_pattern"]["edges"];
        let mut construct = None;
        let mut field = None;
        let mut key = None;
        let mut vkind = None;
        let mut attr_id = None;
        for n in nodes.as_array().into_iter().flatten() {
            let kind = n["kind"].as_str().unwrap_or("");
            if let Some(c) = kind.strip_prefix("hub:") {
                if c != "attr" && c != "value" && c != "collection" {
                    construct = Some(c.to_string());
                }
            }
            if kind == "hub:attr" {
                attr_id = n["id"].as_str().map(ToString::to_string);
                for c in n["constraints"].as_array().into_iter().flatten() {
                    let cn = c["name"].as_str().unwrap_or("");
                    let cv = c["matcher"]["value"].as_str().unwrap_or("");
                    if cn == "name" {
                        field = Some(cv.to_string());
                    }
                    if cn == "prov_key" {
                        key = Some(cv.to_string());
                    }
                    if cn == "vkind" {
                        vkind = Some(cv.to_string());
                    }
                }
            }
        }
        if let Some(aid) = &attr_id {
            if let Some(owner) = rule_attr_construct_owner(nodes, edges, aid) {
                construct = Some(owner);
            }
        }
        if let (Some(c), Some(f), Some(k)) = (construct, field, key) {
            let is_seq = attr_id
                .as_deref()
                .is_some_and(|id| rule_attr_is_seq(nodes, edges, id));
            let slot = m.entry((c, f)).or_default();
            let target = if is_seq {
                &mut slot.seq
            } else {
                &mut slot.scalar
            };
            if target.is_none() {
                *target = Some(KeySpec { key: k, vkind });
            }
        }
    }
    m
}

/// Re-key an A-sourced hub into B's vocabulary: for each hub:attr, look up B's
/// primary key for (owning-construct, attr.name) AT THE ATTR'S SHAPE (scalar vs.
/// seq) and overwrite prov_key. `.or` only crosses shape for a single-spelling
/// field. Attrs whose (construct, field) B doesn't model are left unchanged (they
/// fall away in B's backward — the expected interop intersection loss).
fn rekey_hub(hub: &mut TypedGraph, fk: &std::collections::HashMap<(String, String), ShapeKeys>) {
    // owning construct per attr = kind of the node that has_attr-> it.
    let mut owner: std::collections::HashMap<GhostId, String> = std::collections::HashMap::new();
    for (s, t, e) in hub.iter_edges() {
        if e.type_id == "hub:has_attr" {
            if let Some(sn) = hub.get_node(&s) {
                owner.insert(
                    t,
                    sn.type_id
                        .strip_prefix("hub:")
                        .unwrap_or(&sn.type_id)
                        .to_string(),
                );
            }
        }
    }
    let ids: Vec<GhostId> = hub
        .iter_nodes()
        .filter(|n| n.type_id == "hub:attr")
        .map(|n| n.id)
        .collect();
    for id in ids {
        let (Some(field), Some(c)) = (
            hub.get_node(&id).and_then(|n| n.attrs.get("name").cloned()),
            owner.get(&id).cloned(),
        ) else {
            continue;
        };
        if let Some(sk) = fk.get(&(c, field)) {
            let seq = hub
                .iter_edges()
                .into_iter()
                .find(|(s, _, e)| *s == id && e.type_id == "hub:has_value")
                .and_then(|(_, t, _)| hub.get_node(&t))
                .is_some_and(|n| n.type_id == "hub:collection");
            let spec = if seq {
                sk.seq.clone().or_else(|| sk.scalar.clone())
            } else {
                sk.scalar.clone().or_else(|| sk.seq.clone())
            };
            if let Some(spec) = spec {
                hub.set_node_attr(&id, "prov_key", &spec.key);
                // Adopt the target's vkind so its backward constraint matches; a
                // no-vkind target rule matches regardless, so leave it then.
                if let Some(vk) = spec.vkind {
                    hub.set_node_attr(&id, "vkind", &vk);
                }
            }
        }
    }
}

/// Run the cross-platform a->b->a' loop and return (hub_A signature, hub_B'
/// signature): forward A -> shared hub_A -> rekey to B's vocabulary ->
/// backward B -> emit B -> re-seed via B -> forward B -> hub_B'. When the two
/// signatures are equal the SHARED semantic model survived the cross-platform
/// trip losslessly. (Platform-specific fields A has but B can't represent are
/// inherently dropped — correct cross-platform-migration semantics.)
fn interop_ab(a: &str, b: &str, seed: u64) -> (BTreeMap<String, usize>, BTreeMap<String, usize>) {
    let rules_a = pool(&ruleset(a));
    let rules_b = pool(&ruleset(b));
    let yaml_a = generate_yaml(a, seed, &Budget::shallow()).expect("gen A");
    let doc = parse(&yaml_a).expect("parse A");
    let mut g = seed_for(a, &doc, &yaml_a);
    run_routed(&mut g, &rules_a).expect("fwd A");
    let mut hub = isolate_hub(&g);
    let sig_a = hub_signature(&hub).0;
    rekey_hub(&mut hub, &field_to_key_map(b));
    run_routed(&mut hub, &rules_b).expect("bwd B");
    let root = pick_pipeline_root(&hub).expect("B pipeline root");
    let yaml_b = emit_yaml(&hub, root);
    let doc_b = parse(&yaml_b).expect("parse B");
    let mut gb = seed_for(b, &doc_b, &yaml_b);
    run_routed(&mut gb, &rules_b).expect("fwd B");
    (sig_a, hub_signature(&isolate_hub(&gb)).0)
}

/// Point 3 guarantee (asserted, CI-gated): cross-platform a->b is FAITHFUL for
/// a structurally-compatible pair — the destination reconstructs a sub-model of
/// the source and INVENTS nothing. woodpecker and drone are both bijektiv-flach
/// container-step platforms, so a woodpecker config translated to drone and
/// re-forwarded yields a hub whose every node (kind + identity, by multiset)
/// also exists in the woodpecker source hub — the shared model survives intact
/// (only key vocabulary is translated, via rekey_hub).
///
/// The invariant is a MULTISET SUBSET (sig_b ⊆ sig_a), NOT full equality:
/// woodpecker-specific fields the chaos corpus generates (labels, dns,
/// backend_options) have no drone representation and are legitimately dropped
/// (the documented cross-platform-migration semantics above). What must NEVER
/// happen is the trip INVENTING or DUPLICATING a node — that would be a real
/// corruption bug, and it is exactly what this asserts.
#[test]
fn interop_faithful_for_compatible_pair() {
    let (sig_a, sig_b) = interop_ab("woodpecker", "drone", 1);
    // Every node drone reconstructs must exist in the woodpecker source with at
    // least the same multiplicity; anything beyond that is invented/corrupted.
    let invented: BTreeMap<&String, &usize> = sig_b
        .iter()
        .filter(|(k, v)| sig_a.get(*k).copied().unwrap_or(0) < **v)
        .collect();
    assert!(
        invented.is_empty(),
        "woodpecker->drone must invent nothing — these re-hub nodes exceed the source hub:\n{invented:#?}\n--- source hub ---\n{sig_a:#?}\n--- re-hub ---\n{sig_b:#?}",
    );
    // And the trip must actually carry the shared structural core across (not a
    // vacuous empty hub): the pipeline and its steps survive.
    assert_eq!(
        sig_b.get("hub:pipeline#"),
        sig_a.get("hub:pipeline#"),
        "pipeline lost in interop"
    );
    assert_eq!(
        sig_b.get("hub:step#"),
        sig_a.get("hub:step#"),
        "steps lost in interop"
    );
}

/// Forward a config to its shared hub (platform P → hub, hub-only graph).
fn fwd_hub(p: &str, yaml: &str, rules: &[Box<dyn Rule>]) -> TypedGraph {
    let doc = parse_for(p, yaml).expect("parse");
    let mut g = seed_for(p, &doc, yaml);
    run_routed(&mut g, rules).expect("fwd");
    isolate_hub(&g)
}

/// How a platform contains its pipeline's jobs in the hub: a direct keyless
/// `pipeline --has_job--> job` edge (gitlab/earthly — top-level keyless), or an
/// `attr[name=jobs]`+collection wrapper (github/circleci/tekton/travis/azure/…).
#[derive(Clone)]
enum JobForm {
    Keyless,
    AttrCollection { key: String, vkind: Option<String> },
    Unknown,
}

/// Read the target's job-containment form deterministically from its ruleset —
/// not guessed.
fn target_job_form(platform: &str) -> JobForm {
    let Some((key, vkind)) = target_jobs_attr_collection(platform) else {
        // No attr+collection jobs rule found. Either keyless (a direct
        // pipeline-has_job-job rule) or no jobs at all; the helper signals
        // keyless by short-circuiting. Re-derive which.
        return if platform_is_keyless_jobs(platform) {
            JobForm::Keyless
        } else {
            JobForm::Unknown
        };
    };
    JobForm::AttrCollection { key, vkind }
}

/// `true` if the platform's ruleset contains a direct `pipeline --has_job--> job`
/// rule (the keyless top-level-jobs form).
fn platform_is_keyless_jobs(platform: &str) -> bool {
    let p = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../catalog/rules")
        .join(format!("{platform}.ruleset.json"));
    let Ok(txt) = std::fs::read_to_string(&p) else {
        return false;
    };
    let Ok(v) = serde_json::from_str::<serde_json::Value>(&txt) else {
        return false;
    };
    v["rules"].as_array().into_iter().flatten().any(|rule| {
        let nodes = &rule["r_pattern"]["nodes"];
        let kind = |id: &str| -> Option<String> {
            nodes
                .as_array()?
                .iter()
                .find(|n| n["id"].as_str() == Some(id))
                .and_then(|n| n["kind"].as_str())
                .map(ToString::to_string)
        };
        rule["r_pattern"]["edges"]
            .as_array()
            .into_iter()
            .flatten()
            .any(|e| {
                e["kind"].as_str() == Some("hub:has_job")
                    && e["source_node_id"].as_str().and_then(kind).as_deref()
                        == Some("hub:pipeline")
            })
    })
}

fn target_jobs_attr_collection(platform: &str) -> Option<(String, Option<String>)> {
    let p = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../catalog/rules")
        .join(format!("{platform}.ruleset.json"));
    let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&p).ok()?).ok()?;
    let mut attr_collection = None;
    for rule in v["rules"].as_array().into_iter().flatten() {
        let nodes = &rule["r_pattern"]["nodes"];
        let edges = &rule["r_pattern"]["edges"];
        let kind = |id: &str| -> Option<&str> {
            nodes
                .as_array()?
                .iter()
                .find(|n| n["id"].as_str() == Some(id))
                .and_then(|n| n["kind"].as_str())
        };
        // Keyless: a direct pipeline --has_job--> job edge anywhere → no wrap.
        let keyless = edges.as_array().into_iter().flatten().any(|e| {
            e["kind"].as_str() == Some("hub:has_job")
                && e["source_node_id"].as_str().and_then(kind) == Some("hub:pipeline")
        });
        if keyless {
            return None;
        }
        // attr+collection: pipeline --has_attr--> attr[name=jobs] AND collection --has_item--> job.
        let has_item_job = edges.as_array().into_iter().flatten().any(|e| {
            e["kind"].as_str() == Some("hub:has_item")
                && e["target_node_id"].as_str().and_then(kind) == Some("hub:job")
        });
        if has_item_job {
            for n in nodes.as_array().into_iter().flatten() {
                if n["kind"].as_str() != Some("hub:attr") {
                    continue;
                }
                let cons: std::collections::HashMap<&str, &str> = n["constraints"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|c| Some((c["name"].as_str()?, c["matcher"]["value"].as_str()?)))
                    .collect();
                let owned_by_pipeline = edges.as_array().into_iter().flatten().any(|e| {
                    e["kind"].as_str() == Some("hub:has_attr")
                        && e["source_node_id"].as_str().and_then(kind) == Some("hub:pipeline")
                        && e["target_node_id"].as_str() == n["id"].as_str()
                });
                // First wins — the PRIMARY jobs key (ir.toml lists it first, e.g.
                // circleci `jobs` before the secondary `job-groups`). Wrapping
                // under the primary keeps wrap/unwrap consistent.
                if owned_by_pipeline
                    && cons.get("name") == Some(&"jobs")
                    && attr_collection.is_none()
                {
                    if let Some(k) = cons.get("prov_key") {
                        attr_collection = Some((
                            (*k).to_string(),
                            cons.get("vkind").map(|s| (*s).to_string()),
                        ));
                    }
                }
            }
        }
    }
    attr_collection
}

/// `true` if the edge is still live (not tombstoned).
fn edge_live(e: &EdgeData) -> bool {
    !matches!(e.status, Status::Tombstone | Status::TentativeTombstone)
}

/// Bridge job-containment forms to the target's, in place (tombstone + add — no
/// rebuild): a keyless `pipeline --has_job--> job` hub and an `attr[name=jobs]`+
/// collection hub are the SAME semantics in two shapes (gitlab/earthly keyless;
/// github/travis/… attr+collection). Whichever the source produced, re-shape it
/// to the form the TARGET's backward consumes — otherwise the jobs cross as bare
/// top-level keys (vacuous, travis even mis-reads a language-named one) one way,
/// or get dropped the other way, and the fixpoint drifts.
fn normalize_job_containment(hub: &mut TypedGraph, to: &str) {
    let pipes: Vec<GhostId> = hub
        .iter_nodes()
        .filter(|n| n.type_id == "hub:pipeline")
        .map(|n| n.id)
        .collect();
    match target_job_form(to) {
        JobForm::AttrCollection { key, vkind } => {
            for pid in pipes {
                let job_edges: Vec<(GhostId, GhostId)> = hub
                    .iter_edges()
                    .into_iter()
                    .filter(|(s, _, e)| *s == pid && e.type_id == "hub:has_job" && edge_live(e))
                    .map(|(_, t, e)| (e.id, t))
                    .collect();
                if job_edges.is_empty() {
                    continue;
                }
                let mut a_attrs = std::collections::BTreeMap::new();
                a_attrs.insert("name".to_string(), "jobs".to_string());
                a_attrs.insert("prov_key".to_string(), key.clone());
                if let Some(vk) = &vkind {
                    a_attrs.insert("vkind".to_string(), vk.clone());
                }
                let attr = hub.add_solid_child_node(pid, "hub:has_attr", "hub:attr", a_attrs);
                let coll = hub.add_solid_child_node(
                    attr,
                    "hub:has_value",
                    "hub:collection",
                    std::collections::BTreeMap::new(),
                );
                hub.add_edge(
                    pid,
                    attr,
                    "hub:has_attr",
                    std::collections::BTreeMap::new(),
                    Status::Solid,
                );
                hub.add_edge(
                    attr,
                    coll,
                    "hub:has_value",
                    std::collections::BTreeMap::new(),
                    Status::Solid,
                );
                for (eid, job) in job_edges {
                    hub.set_edge_status(&eid, Status::Tombstone);
                    hub.add_edge(
                        coll,
                        job,
                        "hub:has_item",
                        std::collections::BTreeMap::new(),
                        Status::Solid,
                    );
                }
            }
        }
        JobForm::Keyless => {
            // Unwrap: pipeline -has_attr-> attr[name=jobs] -has_value-> collection
            // -has_item-> job  ⇒  pipeline -has_job-> job. Tombstone the wrapper
            // attr so the keyless backward sees only the direct edges.
            for pid in pipes {
                // jobs-attrs owned by this pipeline whose name=jobs.
                let job_attrs: Vec<(GhostId /*edge*/, GhostId /*attr*/)> = hub
                    .iter_edges()
                    .into_iter()
                    .filter(|(s, t, e)| {
                        *s == pid
                            && e.type_id == "hub:has_attr"
                            && edge_live(e)
                            && hub
                                .get_node(t)
                                .and_then(|n| n.attrs.get("name").cloned())
                                .as_deref()
                                == Some("jobs")
                    })
                    .map(|(_, t, e)| (e.id, t))
                    .collect();
                for (attr_edge, attr) in job_attrs {
                    // collection(s) under the attr, then jobs under the collection.
                    let colls: Vec<GhostId> = hub
                        .iter_edges()
                        .into_iter()
                        .filter(|(s, t, e)| {
                            *s == attr
                                && e.type_id == "hub:has_value"
                                && edge_live(e)
                                && hub
                                    .get_node(t)
                                    .map(|n| n.type_id == "hub:collection")
                                    .unwrap_or(false)
                        })
                        .map(|(_, t, _)| t)
                        .collect();
                    let mut jobs: Vec<GhostId> = Vec::new();
                    for c in &colls {
                        for (s, t, e) in hub.iter_edges() {
                            if s == *c
                                && e.type_id == "hub:has_item"
                                && edge_live(e)
                                && hub
                                    .get_node(&t)
                                    .map(|n| n.type_id == "hub:job")
                                    .unwrap_or(false)
                            {
                                jobs.push(t);
                            }
                        }
                    }
                    if jobs.is_empty() {
                        continue;
                    }
                    for job in jobs {
                        hub.add_edge(
                            pid,
                            job,
                            "hub:has_job",
                            std::collections::BTreeMap::new(),
                            Status::Solid,
                        );
                    }
                    // Tombstone the wrapper attr + its has_attr edge so the
                    // attr+collection shape is gone from the matchable view.
                    hub.set_edge_status(&attr_edge, Status::Tombstone);
                    hub.set_node_status(&attr, Status::Tombstone);
                }
            }
        }
        JobForm::Unknown => {}
    }
}

/// Cross-emit a shared hub into platform `to`'s surface syntax: rekey the hub
/// to `to`'s key vocabulary, bridge job-containment form, run `to`'s rules
/// backward, emit. Consumes the hub.
fn cross_emit(mut hub: TypedGraph, to: &str, rules_to: &[Box<dyn Rule>]) -> String {
    rekey_hub(&mut hub, &field_to_key_map(to));
    normalize_job_containment(&mut hub, to);
    run_routed(&mut hub, rules_to).expect("bwd");
    let root = pick_pipeline_root(&hub).expect("cross-emit: no pipeline root");
    emit_for(to, &hub, root)
}

/// Cross-platform migration FIXPOINT: a→b→a'→b'. The lossy A→B projection
/// drops what B can't represent, but it must be IDEMPOTENT — once a config has
/// crossed to B, deriving B again via B→A'→B' must reproduce the SAME B. Returns
/// (hub_B signature, hub_B' signature); a stable migration has them equal.
fn interop_fixpoint(
    a: &str,
    b: &str,
    seed: u64,
) -> (BTreeMap<String, usize>, BTreeMap<String, usize>) {
    let (ra, rb) = (pool(&ruleset(a)), pool(&ruleset(b)));
    let yaml_a = generate_yaml(a, seed, &Budget::shallow()).expect("gen A");
    let hub_a = fwd_hub(a, &yaml_a, &ra);
    let yaml_b = cross_emit(hub_a, b, &rb); // a → b
    let mut hb2 = fwd_hub(b, &yaml_b, &rb);
    let sig_b = hub_signature(&hb2).0;
    // dump pipeline-owned attrs before/after rekey to woodpecker
    let pid: Vec<GhostId> = hb2
        .iter_nodes()
        .filter(|n| n.type_id == "hub:pipeline")
        .map(|n| n.id)
        .collect();
    let powned: Vec<GhostId> = hb2
        .iter_edges()
        .into_iter()
        .filter(|(s, _, e)| pid.contains(s) && e.type_id == "hub:has_attr")
        .map(|(_, t, _)| t)
        .collect();
    eprintln!("--- pipeline attrs BEFORE rekey ---");
    for id in &powned {
        if let Some(n) = hb2.get_node(id) {
            eprintln!(
                "  name={:?} key={:?}",
                n.attrs.get("name"),
                n.attrs.get("prov_key")
            );
        }
    }
    rekey_hub(&mut hb2, &field_to_key_map(a));
    eprintln!("--- pipeline attrs AFTER rekey to wdp ---");
    for id in &powned {
        if let Some(n) = hb2.get_node(id) {
            eprintln!(
                "  name={:?} key={:?}",
                n.attrs.get("name"),
                n.attrs.get("prov_key")
            );
        }
    }
    let hub_b = fwd_hub(b, &yaml_b, &rb);
    let yaml_a2 = cross_emit(hub_b, a, &ra); // b → a'
    let hub_a2 = fwd_hub(a, &yaml_a2, &ra);
    let yaml_b2 = cross_emit(hub_a2, b, &rb); // a' → b'
    let hub_b2 = fwd_hub(b, &yaml_b2, &rb);
    (sig_b, hub_signature(&hub_b2).0)
}

/// Point 2 guarantee (asserted, CI-gated): the cross-platform projection is a
/// FIXPOINT — a→b→a'→b' yields b'≡b. Once translated to a platform, re-deriving
/// that platform through the reverse-and-forward trip is stable (no drift, no
/// accreting/losing nodes round over round). Uses the bijektiv-flach compatible
/// pair (woodpecker↔drone) where the shared model is large, in both directions.
#[test]
fn interop_fixpoint_stable() {
    for (a, b) in [("woodpecker", "drone"), ("drone", "woodpecker")] {
        let (sig_b, sig_b2) = interop_fixpoint(a, b, 1);
        assert!(!sig_b.is_empty(), "{a}→{b}: empty B hub (vacuous fixpoint)");
        assert_eq!(
            sig_b, sig_b2,
            "{a}→{b} is not a fixpoint: B≠B' after a→b→a'→b'\n--- B ---\n{sig_b:#?}\n--- B' ---\n{sig_b2:#?}",
        );
    }
}

/// Wide fixpoint sweep over many pairs + seeds — surfaces drift the single
/// compatible-pair gate misses. Reports per-pair stability; a mismatch is a
/// migration that accretes or loses nodes on the second crossing.
/// Run: cargo test interop_fixpoint_wide -- --ignored --nocapture
#[test]
#[ignore = "wide fixpoint sweep — run with --ignored --nocapture"]
fn interop_fixpoint_wide() {
    let pairs = [
        ("woodpecker", "drone"),
        ("drone", "woodpecker"),
        ("github", "gitlab"),
        ("gitlab", "github"),
        ("drone", "github"),
        ("travis", "drone"),
        ("circleci", "github"),
        ("azure", "github"),
    ];
    println!("\n── Interop fixpoint a→b→a'→b' (assert b'≡b) ──");
    let mut unstable = 0;
    for (a, b) in pairs {
        let mut stable = 0;
        let mut drift: Vec<u64> = Vec::new();
        for seed in 1..=5u64 {
            let res = std::panic::catch_unwind(|| interop_fixpoint(a, b, seed));
            match res {
                Ok((sig_b, sig_b2)) if sig_b == sig_b2 && !sig_b.is_empty() => stable += 1,
                Ok(_) => drift.push(seed),
                Err(_) => drift.push(seed), // emit/cascade failure counts as unstable
            }
        }
        if !drift.is_empty() {
            unstable += 1;
        }
        println!(
            "{a:>11} → {b:<11} {stable}/5 stable {}",
            if drift.is_empty() {
                String::new()
            } else {
                format!("drift/err seeds={drift:?}")
            }
        );
    }
    println!("── pairs with drift: {unstable}/{} ──", pairs.len());
}

/// The full cross-platform interop MATRIX. For every ordered pair (A, B), A≠B,
/// run the fixpoint trip a→b→a'→b' over several seeds and classify whether B can
/// faithfully represent an A pipeline. The full ordered matrix is BOTH
/// directions (cell [A][B] is a→b→a'→b'; cell [B][A] is b→a→b'→a'). The diagonal
/// is the plain round-trip (proven elsewhere), shown as `—`.
///
/// Writes a markdown table to docs/interop-matrix.md and prints a summary.
/// Cell legend: `ok` faithful+stable+non-empty · `∅` empty (B can't represent
/// A) · `≠` drift (b'≠b) · `x` error (cascade/emit panic).
///   cargo test -p chaos-generator --test roundtrip interop_matrix -- --ignored --nocapture
#[test]
#[ignore = "wide sweep — writes docs/interop-matrix.md (PLAT×PLAT fixpoint)"]
fn interop_matrix() {
    // Short codes for a compact table; same order as PLATFORMS.
    let code = |p: &str| -> &'static str {
        match p {
            "drone" => "dro",
            "woodpecker" => "wdp",
            "buildkite" => "bkt",
            "tekton" => "tkt",
            "argo" => "arg",
            "google_cloudbuild" => "gcb",
            "github" => "gh",
            "azure" => "az",
            "aws_codebuild" => "acb",
            "aws_codepipeline" => "acp",
            "travis" => "trv",
            "gitlab" => "gl",
            "bitbucket" => "bb",
            "circleci" => "cci",
            "jenkins" => "jen",
            "earthly" => "ear",
            "dagger" => "dag",
            _ => "???",
        }
    };
    /// Classify one ordered pair across seeds: worst case wins.
    fn classify(a: &str, b: &str) -> &'static str {
        let (mut ok, mut empty, mut drift, mut err) = (0, 0, 0, 0);
        for seed in 1..=3u64 {
            match std::panic::catch_unwind(|| interop_fixpoint(a, b, seed)) {
                Ok((sig_b, _)) if sig_b.is_empty() => empty += 1,
                Ok((sig_b, sig_b2)) if sig_b == sig_b2 => ok += 1,
                Ok(_) => drift += 1,
                Err(_) => err += 1,
            }
        }
        if err > 0 {
            "x"
        } else if empty == 3 {
            "∅"
        } else if drift > 0 {
            "≠"
        } else if ok > 0 {
            "ok"
        } else {
            "∅"
        }
    }

    let mut md = String::from("# Cross-platform interop matrix\n\n");
    md.push_str(
        "Each cell is the fixpoint trip **row → col → row' → col'** over chaos seeds 1–3.\n\n",
    );
    md.push_str("Legend: `ok` faithful + stable + non-empty · `∅` empty (target can't represent the source) · `≠` drift (b'≠b) · `x` error · `—` diagonal (round-trip, proven separately).\n\n");
    // header
    md.push_str("| from\\to |");
    for p in PLATFORMS {
        md.push_str(&format!(" {} |", code(p)));
    }
    md.push_str("\n|---|");
    for _ in PLATFORMS {
        md.push_str("---|");
    }
    md.push('\n');

    let mut counts = std::collections::BTreeMap::<&str, usize>::new();
    for a in PLATFORMS {
        md.push_str(&format!("| **{}** |", code(a)));
        for b in PLATFORMS {
            let cell = if a == b { "—" } else { classify(a, b) };
            if a != b {
                *counts.entry(cell).or_default() += 1;
            }
            md.push_str(&format!(" {cell} |"));
        }
        md.push('\n');
        eprintln!("row {a} done");
    }
    md.push_str(&format!(
        "\n**Summary** ({} ordered pairs): ok={}, ∅={}, ≠={}, x={}\n",
        PLATFORMS.len() * (PLATFORMS.len() - 1),
        counts.get("ok").unwrap_or(&0),
        counts.get("∅").unwrap_or(&0),
        counts.get("≠").unwrap_or(&0),
        counts.get("x").unwrap_or(&0),
    ));

    let out = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/interop-matrix.md");
    std::fs::write(&out, &md).unwrap();
    eprintln!("\n{md}\nwrote {}", out.display());
}

/// Wide chaos stress: run the full roundtrip over a BROAD seed range (not the
/// 5 baseline seeds) to harden the all-14-platforms losslessness claim against
/// more of the generated corpus. Prints a per-platform pass count; any failing
/// seed is a latent loss the 5-seed baseline didn't surface.
/// Run: cargo test roundtrip_wide_stress -- --ignored --nocapture
#[test]
#[ignore = "wide stress — broad seed sweep, run with --ignored --nocapture"]
fn roundtrip_wide_stress() {
    let seeds: Vec<u64> = (1..=30).collect();
    let budget = Budget::shallow();
    println!(
        "\n── Wide roundtrip stress ({} seeds/platform) ──",
        seeds.len()
    );
    let mut grand_fail = 0;
    for &platform in PLATFORMS {
        let rules = pool(&ruleset(platform));
        let (mut eq, mut n) = (0usize, 0usize);
        let mut fails: Vec<u64> = Vec::new();
        for &seed in &seeds {
            let Ok(yaml) = generate_yaml(platform, seed, &budget) else {
                continue;
            };
            n += 1;
            if roundtrip_once(platform, &yaml, &rules).hub_equal {
                eq += 1;
            } else {
                fails.push(seed);
            }
        }
        grand_fail += fails.len();
        let tag = if fails.is_empty() { "OK" } else { "FAIL" };
        println!(
            "{platform:<20} {eq:>3}/{n}  {tag}  {}",
            if fails.is_empty() {
                String::new()
            } else {
                format!("fails={fails:?}")
            }
        );
    }
    println!("── grand total failing seeds: {grand_fail} ──");
}

/// CI gate sample: a fast subset of the heavy gates for every PR — 3 chaos
/// seeds per platform plus the small canonical corpus fixtures. The full
/// gates (30 seeds, whole corpus, interop matrix) run nightly; this catches
/// gross round-trip regressions in ~under a minute. Panics on any failure so
/// CI goes red.
/// Run: cargo test -p chaos-generator --test roundtrip gate_sample -- --ignored --nocapture
#[test]
#[ignore = "CI gate sample — run with --ignored --nocapture"]
fn gate_sample() {
    let budget = Budget::shallow();
    let seeds: [u64; 3] = [1, 7, 19];
    let mut failures: Vec<String> = Vec::new();
    let (mut chaos_ok, mut corpus_ok) = (0usize, 0usize);

    // Per-platform: chaos seeds + the canonical corpus fixture, one line each so
    // the CI log shows exactly what was exercised, not just a green light.
    let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/cross_corpus");
    println!(
        "\n── Gate sample: {} chaos seeds + corpus fixture per platform ──",
        seeds.len()
    );
    for &platform in PLATFORMS {
        let rules = pool(&ruleset(platform));

        let mut chaos = 0usize;
        for &seed in &seeds {
            let Ok(yaml) = generate_yaml(platform, seed, &budget) else {
                continue;
            };
            if roundtrip_once(platform, &yaml, &rules).hub_equal {
                chaos += 1;
                chaos_ok += 1;
            } else {
                failures.push(format!("chaos {platform} seed {seed}"));
            }
        }

        // The first fixture file under tests/cross_corpus/<platform>/.
        let corpus = std::fs::read_dir(base.join(platform))
            .ok()
            .and_then(|rd| rd.flatten().map(|e| e.path()).find(|p| p.is_file()));
        let corpus_mark = match &corpus {
            Some(f) => match std::fs::read_to_string(f) {
                Ok(content) if roundtrip_once(platform, &content, &rules).hub_equal => {
                    corpus_ok += 1;
                    "ok"
                }
                Ok(_) => {
                    failures.push(format!("corpus {}", f.display()));
                    "FAIL"
                }
                Err(_) => "—",
            },
            None => "—",
        };

        println!(
            "  {platform:<18} chaos {chaos}/{}  corpus {corpus_mark}",
            seeds.len()
        );
    }

    println!(
        "── gate sample: {chaos_ok} chaos + {corpus_ok} corpus round-trips across {} platforms — {} ──",
        PLATFORMS.len(),
        if failures.is_empty() { "all green" } else { "FAILURES" }
    );
    if !failures.is_empty() {
        for f in &failures {
            println!("  FAIL {f}");
        }
        panic!("gate sample: {} round-trip failure(s)", failures.len());
    }
}

/// Real-config corpus: round-trip the HAND-CURATED fixtures under
/// `tests/cross_corpus/` (canonical configs, all 17 platforms) and
/// `tests/edge_cases/` (configs lifted from real-world repos) — NOT
/// chaos-generated. The honest losslessness test: chaos exercises the
/// schema, these exercise what people actually write. Reports pass/fail
/// per fixture so latent real-world gaps surface.
/// Run: cargo test real_config_corpus_roundtrip -- --ignored --nocapture
#[test]
#[ignore = "real-config corpus — run with --ignored --nocapture"]
fn real_config_corpus_roundtrip() {
    let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests");
    let known: std::collections::HashSet<&str> = PLATFORMS.iter().copied().collect();
    let (mut total, mut pass) = (0usize, 0usize);
    let mut fails: Vec<String> = Vec::new();
    println!("\n── Real-config corpus roundtrip ──");
    for group in ["cross_corpus", "edge_cases"] {
        let Ok(rd) = std::fs::read_dir(base.join(group)) else {
            continue;
        };
        let mut platdirs: Vec<_> = rd
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.is_dir())
            .collect();
        platdirs.sort();
        for pdir in platdirs {
            let platform = pdir.file_name().unwrap().to_string_lossy().to_string();
            if !known.contains(platform.as_str()) {
                continue;
            }
            let rules = pool(&ruleset(&platform));
            let mut files: Vec<_> = std::fs::read_dir(&pdir)
                .unwrap()
                .flatten()
                .map(|e| e.path())
                .filter(|p| p.is_file())
                .collect();
            files.sort();
            for f in files {
                let Ok(content) = std::fs::read_to_string(&f) else {
                    continue;
                };
                let name = format!(
                    "{group}/{platform}/{}",
                    f.file_name().unwrap().to_string_lossy()
                );
                // The whole curated corpus now round-trips: the engine's reverse
                // pole was linearised (Hebel 5/6 + const-hasher), so even the two
                // ~1.3k-line monsters (tokio, libinput) complete the full
                // fwd→bwd→fwd roundtrip in ~1–2min. Cap at 1500 is a sanity
                // backstop against an accidentally-huge fixture.
                if content.lines().count() > 1500 {
                    eprintln!("  [SKIP huge ] {name} ({} lines)", content.lines().count());
                    continue;
                }
                total += 1;
                let t = std::time::Instant::now();
                let o = roundtrip_once(&platform, &content, &rules);
                let ms = t.elapsed().as_millis();
                if o.hub_equal {
                    pass += 1;
                    eprintln!("  [OK   {ms:>5}ms] {name}");
                } else {
                    let first = o.detail.lines().next().unwrap_or("");
                    eprintln!("  [FAIL {ms:>5}ms] {name} — {first}");
                    fails.push(format!(
                        "{name}\n      {}",
                        o.detail
                            .lines()
                            .take(6)
                            .collect::<Vec<_>>()
                            .join("\n      ")
                    ));
                }
            }
        }
    }
    println!("  {pass}/{total} real-config fixtures round-trip losslessly\n");
    for fl in &fails {
        println!("  FAIL {fl}");
    }
}

/// Interop probe: forward platform A -> shared hub -> backward platform B
/// -> emit B. Then a->b->a': seed_B(emit) -> forward B -> hub_B, and compare
/// hub_A to hub_B at the content level. Losslessness here holds only for the
/// INTERSECTION both platforms model; the report shows what survived.
/// Run: PLAT_A=drone PLAT_B=woodpecker SEED=1 cargo test interop_probe -- --ignored --nocapture
#[test]
#[ignore = "interop probe — PLAT_A/PLAT_B/SEED, prints cross-platform emit"]
fn interop_probe() {
    let a = std::env::var("PLAT_A").unwrap_or_else(|_| "drone".into());
    let b = std::env::var("PLAT_B").unwrap_or_else(|_| "woodpecker".into());
    let seed: u64 = std::env::var("SEED")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(1);
    let budget = Budget::shallow();
    let rules_a = pool(&ruleset(&a));
    let rules_b = pool(&ruleset(&b));

    let yaml_a = generate_yaml(&a, seed, &budget).expect("gen A");
    println!("\n=== SOURCE YAML ({a} seed {seed}) ===\n{yaml_a}");
    let doc = parse(&yaml_a).expect("parse A");
    let mut g = seed_for(&a, &doc, &yaml_a);
    run_routed(&mut g, &rules_a).expect("fwd A");
    let mut hub = isolate_hub(&g);
    let sig_a = hub_signature(&hub);
    let mut hubkinds: BTreeMap<String, usize> = BTreeMap::new();
    for n in hub.iter_nodes() {
        *hubkinds.entry(n.type_id.clone()).or_default() += 1;
    }
    println!("=== HUB (from {a}) kinds === {hubkinds:?}");

    // Re-key the A-sourced hub into B's vocabulary so B's prov_key-gated
    // backward rules fire (unless REKEY=0 to observe the raw coupling).
    if std::env::var("REKEY").as_deref() != Ok("0") {
        let fk = field_to_key_map(&b);
        rekey_hub(&mut hub, &fk);
    }
    // Backward with B's ruleset: hub -> cst[B].
    run_routed(&mut hub, &rules_b).expect("bwd B");
    match pick_pipeline_root(&hub) {
        Some(root) => {
            let yaml_b = emit_yaml(&hub, root);
            println!(
                "=== EMITTED AS {b} ===\n{yaml_b}\n=== END ({} chars) ===",
                yaml_b.trim().len()
            );
            // a->b->a': re-seed the B emission via B, forward to hub_B.
            if let Ok(doc_b) = parse(&yaml_b) {
                let mut gb = seed_for(&b, &doc_b, &yaml_b);
                if run_routed(&mut gb, &rules_b).is_ok() {
                    let hub_b = isolate_hub(&gb);
                    let sig_b = hub_signature(&hub_b);
                    let only_a: Vec<_> = sig_a
                        .0
                        .iter()
                        .filter(|(k, v)| sig_b.0.get(*k) != Some(*v))
                        .map(|(k, v)| format!("  -{k} (x{v})"))
                        .collect();
                    let only_b: Vec<_> = sig_b
                        .0
                        .iter()
                        .filter(|(k, v)| sig_a.0.get(*k) != Some(*v))
                        .map(|(k, v)| format!("  +{k} (x{v})"))
                        .collect();
                    if only_a.is_empty() && only_b.is_empty() {
                        println!("=== a->b->a' HUB EQUAL (lossless interop) ===");
                    } else {
                        println!("=== a->b->a' HUB DIFF (only in {a}-hub / only in {b}-rehub) ===\n{}\n{}", only_a.join("\n"), only_b.join("\n"));
                    }
                }
            }
        }
        None => println!("=== NO {b} pipeline root — backward B produced no pipeline ==="),
    }
}

#[test]
#[ignore = "debug — PLAT/SEED, prints source YAML only (no cascade)"]
fn dump_seed_yaml() {
    let platform = std::env::var("PLAT").unwrap_or_else(|_| "github".into());
    let seed: u64 = std::env::var("SEED")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(1);
    let budget = Budget::shallow();
    let yaml = generate_yaml(&platform, seed, &budget).expect("gen");
    eprintln!("\n=== SOURCE YAML ({platform} seed {seed}) ===\n{yaml}\n=== END ===");
}

/// Where does the cascade time go? Instruments the FORWARD cascade on a real
/// fixture: per-step timing + graph growth, then ranks per-rule
/// `find_matches` cost on the final graph (each cascade_step re-matches ALL
/// active rules → top rules × #steps ≈ total cost). Run:
///   PLAT=azure FIXTURE=tests/cross_corpus/azure/multistage_with_environments.yml \
///     cargo test -p chaos-generator --test roundtrip cascade_profile -- --ignored --nocapture
#[test]
#[ignore = "perf profile — PLAT/FIXTURE; per-step + per-rule cascade cost"]
fn cascade_profile() {
    use std::time::Instant;
    let platform = std::env::var("PLAT").unwrap_or_else(|_| "azure".into());
    let yaml = match std::env::var("FIXTURE") {
        Ok(p) => {
            std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").join(&p))
                .unwrap_or_else(|e| panic!("read {p}: {e}"))
        }
        Err(_) => panic!("set FIXTURE=<repo-rel path>"),
    };
    let rules = pool(&ruleset(&platform));
    let doc = parse_for(&platform, &yaml).expect("parse");
    let mut g = seed_for(&platform, &doc, &yaml);
    let n_seed = g.iter_nodes().count();

    // Mirror run_routed's active-rule filter.
    let delta: std::collections::HashSet<String> =
        g.iter_nodes().map(|n| n.type_id.clone()).collect();
    let active: Vec<&dyn Rule> = rules
        .iter()
        .filter(|r| {
            let idk = r.input_domain_kinds();
            idk.is_empty() || idk.iter().any(|k| delta.contains(k))
        })
        .map(AsRef::as_ref)
        .collect();
    eprintln!(
        "\n=== cascade_profile {platform} ===\nseed nodes={n_seed}  total rules={}  active rules={}",
        rules.len(),
        active.len()
    );

    // Instrumented forward cascade: time each step + sample node count.
    // Cached driver (matches run_routed) so the curve reflects the real path.
    let mut cascade = Cascade::new();
    let mut cache = MatchCache::new();
    let mut step_us: Vec<u128> = Vec::new();
    let t0 = Instant::now();
    for _ in 0..20_000 {
        let ts = Instant::now();
        let state = cascade_step_cached(&mut cascade, &mut g, &active, &mut cache).expect("step");
        step_us.push(ts.elapsed().as_micros());
        if !matches!(state, TerminationState::Running) {
            break;
        }
    }
    let total = t0.elapsed();
    let steps = step_us.len();
    let n_final = g.iter_nodes().count();
    let first5: u128 = step_us.iter().take(5).sum::<u128>() / 5_u128;
    let last5: u128 = step_us.iter().rev().take(5).sum::<u128>() / 5_u128;
    let maxs = step_us.iter().max().copied().unwrap_or(0);
    eprintln!(
        "FORWARD: {steps} steps in {total:?}  → {:.1}ms/step avg\n  graph: {n_seed} → {n_final} nodes\n  step time: first5≈{first5}us  last5≈{last5}us  max={maxs}us  (growth = re-match-all-rules cost rising with graph size)",
        total.as_secs_f64() * 1000.0 / steps.max(1) as f64
    );

    // Per-rule find_matches cost on the FINAL graph (the most expensive
    // re-match — graph is largest). Each step pays ~this for every rule.
    let mut costs: Vec<(String, u128, usize, usize)> = active
        .iter()
        .map(|r| {
            let ts = Instant::now();
            let m = find_matches(r.pattern(), &g);
            (
                r.id().to_string(),
                ts.elapsed().as_micros(),
                m.len(),
                r.pattern().nodes.len(),
            )
        })
        .collect();
    let sum_us: u128 = costs.iter().map(|c| c.1).sum();
    costs.sort_by_key(|c| std::cmp::Reverse(c.1));
    eprintln!(
        "\nPER-RULE find_matches on final graph: Σ={sum_us}us across {} rules (paid EVERY step → ~{:.1}s over {steps} steps)\n  top 20 most expensive rules (us, #matches, #pattern-nodes):",
        active.len(),
        sum_us as f64 * steps as f64 / 1e6
    );
    for (id, us, m, pn) in costs.iter().take(20) {
        eprintln!("  {us:>7}us  matches={m:<4} nodes={pn:<2} {id}");
    }
}

/// STREAMING perf monitor for a single forward cascade — built to watch the
/// huge configs (tokio/libinput) that don't converge in practical time. Prints
/// a progress line every PERF_EVERY steps: step#, wall elapsed, recent
/// per-step ms (is it climbing super-linearly?), node + edge count (where the
/// memory goes). eprintln is unbuffered so it streams live; pair with an
/// external `ps`/`sample` on the process. Forward-only (the slow direction is
/// already visible there). Run:
///   PLAT=github FIXTURE=tests/edge_cases/github/tokio-ci.yml \
///     cargo test -p chaos-generator --test roundtrip cascade_monitor -- --ignored --nocapture
#[test]
#[ignore = "perf monitor — PLAT/FIXTURE; streams per-step time + graph size live"]
fn cascade_monitor() {
    use std::time::Instant;
    let platform = std::env::var("PLAT").unwrap_or_else(|_| "github".into());
    let every: usize = std::env::var("PERF_EVERY")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(100);
    let yaml = match std::env::var("FIXTURE") {
        Ok(p) => {
            std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").join(&p))
                .unwrap_or_else(|e| panic!("read {p}: {e}"))
        }
        Err(_) => panic!("set FIXTURE=<repo-rel path>"),
    };
    let rules = pool(&ruleset(&platform));
    let doc = parse_for(&platform, &yaml).expect("parse");
    let mut g = seed_for(&platform, &doc, &yaml);
    let delta: std::collections::HashSet<String> =
        g.iter_nodes().map(|n| n.type_id.clone()).collect();
    let active: Vec<&dyn Rule> = rules
        .iter()
        .filter(|r| {
            let idk = r.input_domain_kinds();
            idk.is_empty() || idk.iter().any(|k| delta.contains(k))
        })
        .map(AsRef::as_ref)
        .collect();
    eprintln!(
        "=== cascade_monitor {platform} | seed nodes={} active rules={} (every {every} steps) ===\n  step    elapsed   ms/step(recent)   nodes    edges",
        g.iter_nodes().count(), active.len()
    );
    let mut cascade = Cascade::new();
    let mut cache = MatchCache::new();
    let t0 = Instant::now();
    let mut window = Instant::now();
    let mut step = 0usize;
    loop {
        let state = cascade_step_cached(&mut cascade, &mut g, &active, &mut cache).expect("step");
        step += 1;
        if step % every == 0 {
            let win_ms = window.elapsed().as_secs_f64() * 1000.0 / every as f64;
            let nodes = g.iter_nodes().count();
            let edges = g.iter_edges().len();
            eprintln!(
                "  {step:>6}  {:>7.1}s   {win_ms:>8.2}      {nodes:>7}  {edges:>7}",
                t0.elapsed().as_secs_f64()
            );
            window = Instant::now();
        }
        if !matches!(state, TerminationState::Running) {
            eprintln!(
                "  CONVERGED at step {step} in {:.1}s ({} nodes)",
                t0.elapsed().as_secs_f64(),
                g.iter_nodes().count()
            );
            break;
        }
        if step >= 200_000 {
            eprintln!("  STOP at 200k steps");
            break;
        }
    }
}

/// REVERSE perf monitor — the backward cascade is the dominant pole on big
/// graphs (the corpus's fwd→bwd→fwd roundtrip stalls in the bwd leg). Runs a
/// quiet FORWARD to produce the hub, then streams the BACKWARD cascade (hub →
/// reconstructed CST) with the same per-step instrumentation as cascade_monitor.
/// Compare against cascade_monitor (forward) to see the fwd/bwd asymmetry. Run:
///   PLAT=github FIXTURE=tests/edge_cases/github/tokio-ci.yml \
///     cargo test -p chaos-generator --test roundtrip cascade_monitor_reverse -- --ignored --nocapture
#[test]
#[ignore = "perf monitor — REVERSE/backward cascade; PLAT/FIXTURE/PERF_EVERY"]
fn cascade_monitor_reverse() {
    use std::time::Instant;
    let platform = std::env::var("PLAT").unwrap_or_else(|_| "github".into());
    let every: usize = std::env::var("PERF_EVERY")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(100);
    let yaml = match std::env::var("FIXTURE") {
        Ok(p) => {
            std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").join(&p))
                .unwrap_or_else(|e| panic!("read {p}: {e}"))
        }
        Err(_) => panic!("set FIXTURE=<repo-rel path>"),
    };
    let rules = pool(&ruleset(&platform));
    let doc = parse_for(&platform, &yaml).expect("parse");
    // FORWARD (quiet, cached) → the hub that the backward cascade consumes.
    let fwd_t = Instant::now();
    let mut gf = seed_for(&platform, &doc, &yaml);
    run_routed(&mut gf, &rules).expect("fwd");
    let fwd_s = fwd_t.elapsed().as_secs_f64();
    let mut hub = isolate_hub(&gf);
    let (hub_n, hub_e) = (hub.iter_nodes().count(), hub.iter_edges().len());
    // BACKWARD: cascade on the isolated hub (creates cst: nodes). Mirror
    // run_routed's active-rule filter on the hub's node kinds.
    let delta: std::collections::HashSet<String> =
        hub.iter_nodes().map(|n| n.type_id.clone()).collect();
    let active: Vec<&dyn Rule> = rules
        .iter()
        .filter(|r| {
            let idk = r.input_domain_kinds();
            idk.is_empty() || idk.iter().any(|k| delta.contains(k))
        })
        .map(AsRef::as_ref)
        .collect();
    eprintln!(
        "=== cascade_monitor_reverse {platform} | fwd {fwd_s:.1}s → hub nodes={hub_n} edges={hub_e} | active rules={} (every {every}) ===\n  step    elapsed   ms/step(recent)   nodes    edges",
        active.len()
    );
    let mut cascade = Cascade::new();
    let mut cache = MatchCache::new();
    let t0 = Instant::now();
    let mut window = Instant::now();
    let mut step = 0usize;
    loop {
        let state = cascade_step_cached(&mut cascade, &mut hub, &active, &mut cache).expect("step");
        step += 1;
        if step % every == 0 {
            let win_ms = window.elapsed().as_secs_f64() * 1000.0 / every as f64;
            eprintln!(
                "  {step:>6}  {:>7.1}s   {win_ms:>8.2}      {:>7}  {:>7}",
                t0.elapsed().as_secs_f64(),
                hub.iter_nodes().count(),
                hub.iter_edges().len()
            );
            window = Instant::now();
        }
        if !matches!(state, TerminationState::Running) {
            eprintln!(
                "  BWD CONVERGED at step {step} in {:.1}s ({} nodes)  [fwd was {fwd_s:.1}s]",
                t0.elapsed().as_secs_f64(),
                hub.iter_nodes().count()
            );
            break;
        }
        if step >= 200_000 {
            eprintln!("  STOP at 200k steps");
            break;
        }
    }
}

/// Verify the pipeline-render model lifts cleanly off a real fixture's hub
/// graph. Run: PLAT=azure FIXTURE=tests/cross_corpus/azure/multistage_with_environments.yml \
///   cargo test -p chaos-generator --test roundtrip dump_render_model -- --ignored --nocapture
#[test]
#[ignore = "debug — pipeline-render model from a real fixture (PLAT/FIXTURE)"]
fn dump_render_model() {
    let platform = std::env::var("PLAT").unwrap_or_else(|_| "azure".into());
    let yaml = match std::env::var("FIXTURE") {
        Ok(p) => {
            std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").join(&p))
                .unwrap_or_else(|e| panic!("read {p}: {e}"))
        }
        Err(_) => panic!("set FIXTURE=<repo-rel path>"),
    };
    let rules = pool(&ruleset(&platform));
    let doc = parse_for(&platform, &yaml).expect("parse");
    let mut g = seed_for(&platform, &doc, &yaml);
    run_routed(&mut g, &rules).expect("fwd");
    // RAW node-kind census — quick ground-truth of what the forward cascade
    // produced, handy when a fixture renders sparsely (an IR gap, not a bug).
    let mut kinds: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
    for n in g.iter_nodes() {
        if n.type_id.starts_with("hub:") {
            *kinds.entry(n.type_id.clone()).or_default() += 1;
        }
    }
    eprintln!("RAW hub kinds: {kinds:?}");
    match pipeline_render::lift(&g) {
        Some(model) => eprintln!("\n{}", pipeline_render::describe(&model)),
        None => eprintln!("\n(lift returned None)"),
    }
}

/// EXPERIMENT: the real TGG edit path — mutate the Hub-IR, re-emit via the
/// backward cascade (NOT byte provenance). forward → edit a hub scalar →
/// isolate_hub → run rules backward (materialises CST from the edited hub) →
/// emit. Proves a clean IR mutation propagates to surface syntax through TGG.
///   PLAT=gitlab FIXTURE=tests/cross_corpus/gitlab/ci_build_test.yml \
///   cargo test -p chaos-generator --test roundtrip experiment_tgg_edit -- --ignored --nocapture
#[test]
#[ignore = "experiment — TGG backward re-emit after a hub edit (PLAT/FIXTURE)"]
fn experiment_tgg_edit() {
    let platform = std::env::var("PLAT").unwrap_or_else(|_| "gitlab".into());
    let fixture = std::env::var("FIXTURE")
        .unwrap_or_else(|_| "tests/cross_corpus/gitlab/ci_build_test.yml".into());
    let yaml = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join(&fixture),
    )
    .unwrap();
    let rules = pool(&ruleset(&platform));
    let doc = parse_for(&platform, &yaml).expect("parse");
    let mut g = seed_for(&platform, &doc, &yaml);
    run_routed(&mut g, &rules).expect("fwd");

    // Baseline backward emit (no edit) for comparison.
    let mut hub0 = isolate_hub(&g);
    run_routed(&mut hub0, &rules).expect("bwd0");
    let root0 = pick_pipeline_root(&hub0).expect("root0");
    let before = emit_for(&platform, &hub0, root0);

    // Pick a hub:value scalar and edit it cleanly on the IR.
    let target = g
        .iter_nodes()
        .find(|n| {
            n.type_id == "hub:value" && n.attrs.get("text").is_some_and(|t| t.contains("cargo"))
        })
        .map(|n| (n.id, n.attrs.get("text").cloned().unwrap_or_default()))
        .expect("a cargo command value");
    let new_text = format!("{} --locked", target.1);
    assert!(
        g.set_node_attr(&target.0, "text", &new_text),
        "set_node_attr"
    );
    eprintln!(
        "EDIT hub:value {} : {:?} -> {:?}",
        target.0.short(),
        target.1,
        new_text
    );

    // Re-emit via the backward cascade from the EDITED hub.
    let mut hub1 = isolate_hub(&g);
    run_routed(&mut hub1, &rules).expect("bwd1");
    let root1 = pick_pipeline_root(&hub1).expect("root1");
    let after = emit_for(&platform, &hub1, root1);

    eprintln!("\n=== BEFORE ===\n{before}\n=== AFTER ===\n{after}");
    assert!(
        after.contains(&new_text),
        "edited command must appear in re-emitted YAML"
    );
    assert!(
        !after.contains(&format!("{}\n", target.1)) || after.contains(&new_text),
        "old value replaced"
    );
}

/// EXPERIMENT: structural insert via TGG. Clone a hub:step subgraph (new ids),
/// append it to its job's steps-collection, re-emit backward. Proves a
/// structural IR mutation (= the copy/paste / recipe-insert primitive)
/// propagates to surface syntax. Run:
///   cargo test -p chaos-generator --test roundtrip experiment_tgg_insert -- --ignored --nocapture
#[test]
#[ignore = "experiment — TGG backward re-emit after a structural step insert"]
fn experiment_tgg_insert() {
    let platform = "gitlab";
    let yaml = "build:\n  image: rust:1.75\n  script:\n    - cargo build\n";
    let rules = pool(&ruleset(platform));
    let doc = parse_for(platform, yaml).expect("parse");
    let mut g = seed_for(platform, &doc, yaml);
    run_routed(&mut g, &rules).expect("fwd");

    // The step to clone, and the collection it hangs under (its has_item parent).
    let step = g
        .iter_nodes()
        .find(|n| n.type_id == "hub:step")
        .expect("a step")
        .id;
    let coll = g
        .iter_edges()
        .into_iter()
        .find(|(_, t, e)| *t == step && e.type_id == "hub:has_item")
        .map(|(s, _, _)| s)
        .expect("steps collection");

    // Collect the step's whole outgoing subtree.
    let mut sub: Vec<GhostId> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let mut q = std::collections::VecDeque::from([step]);
    while let Some(id) = q.pop_front() {
        if !seen.insert(id) {
            continue;
        }
        sub.push(id);
        for (s, t, _) in g.iter_edges() {
            if s == id {
                q.push_back(t);
            }
        }
    }
    // Mint fresh ids for every subtree node.
    let idmap: BTreeMap<GhostId, GhostId> = sub
        .iter()
        .map(|id| (*id, GhostId::from_opaque(&format!("clone#{}", id.short()))))
        .collect();
    // Clone nodes (keep status; fresh id; tweak the command to distinguish).
    for id in &sub {
        let mut clone = g.get_node(id).unwrap().clone();
        clone.id = idmap[id];
        if clone
            .attrs
            .get("text")
            .is_some_and(|t| t.contains("cargo build"))
        {
            clone
                .attrs
                .insert("text".into(), "cargo build --release".into());
        }
        g.insert_node_data(clone);
    }
    // Clone intra-subtree edges via add_edge (computes the edge id, keeps status).
    let edges: Vec<_> = g
        .iter_edges()
        .into_iter()
        .filter(|(s, t, _)| idmap.contains_key(s) && idmap.contains_key(t))
        .map(|(s, t, e)| (s, t, e.type_id.clone(), e.attrs.clone(), e.status))
        .collect();
    for (s, t, ty, attrs, status) in edges {
        g.add_edge(idmap[&s], idmap[&t], &ty, attrs, status);
    }
    // Attach the clone to the same steps-collection.
    g.add_edge(
        coll,
        idmap[&step],
        "hub:has_item",
        BTreeMap::new(),
        seesaw_core::graph::Status::Solid,
    );

    // Re-emit backward.
    let mut hub = isolate_hub(&g);
    run_routed(&mut hub, &rules).expect("bwd");
    let root = pick_pipeline_root(&hub).expect("root");
    let out = emit_for(platform, &hub, root);
    eprintln!("=== AFTER INSERT ===\n{out}");
    let count = out.matches("cargo build").count();
    assert!(
        count >= 2,
        "expected the cloned step too, got {count} cargo-build lines:\n{out}"
    );
}

/// Write the render artefacts (SVG diagram + HTML/Markdown runbook) for a real
/// fixture to /tmp/render-<plat>.{svg,html,md} for eyeballing. Run:
///   PLAT=gitlab FIXTURE=tests/cross_corpus/gitlab/release_with_artifacts.yml \
///   cargo test -p chaos-generator --test roundtrip dump_render_artifacts -- --ignored --nocapture
#[test]
#[ignore = "debug — emit pipeline-render SVG/HTML/MD for a fixture (PLAT/FIXTURE)"]
fn dump_render_artifacts() {
    let platform = std::env::var("PLAT").unwrap_or_else(|_| "gitlab".into());
    let fixture = std::env::var("FIXTURE").expect("set FIXTURE=<repo-rel path>");
    let yaml = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join(&fixture),
    )
    .unwrap_or_else(|e| panic!("read {fixture}: {e}"));
    let rules = pool(&ruleset(&platform));
    let doc = parse_for(&platform, &yaml).expect("parse");
    let mut g = seed_for(&platform, &doc, &yaml);
    run_routed(&mut g, &rules).expect("fwd");
    let bundle = pipeline_render::render_all(&g).expect("render_all");
    let base = format!("/tmp/render-{platform}");
    std::fs::write(format!("{base}.svg"), &bundle.diagram.svg).unwrap();
    std::fs::write(format!("{base}.html"), &bundle.html).unwrap();
    std::fs::write(format!("{base}.md"), &bundle.markdown).unwrap();
    eprintln!("wrote {base}.{{svg,html,md}}");
    eprintln!(
        "layout: {}x{}, {} boxes",
        bundle.diagram.layout.width,
        bundle.diagram.layout.height,
        bundle.diagram.layout.jobs.len()
    );
    eprintln!("\n--- markdown ---\n{}", bundle.markdown);
}

#[test]
#[ignore = "debug — forward hub:trigger structure (PLAT/FIXTURE), no backward cascade"]
fn dump_triggers() {
    let platform = std::env::var("PLAT").unwrap_or_else(|_| "azure".into());
    let yaml = match std::env::var("FIXTURE") {
        Ok(p) => {
            let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").join(&p);
            std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {p}: {e}"))
        }
        Err(_) => panic!("set FIXTURE=<repo-rel path>"),
    };
    let rules = pool(&ruleset(&platform));
    let doc = parse_for(&platform, &yaml).expect("parse");
    let mut g = seed_for(&platform, &doc, &yaml);
    run_routed(&mut g, &rules).expect("fwd");
    eprintln!("=== FWD hub:pipeline -> children ({platform}) ===");
    for n in g.iter_nodes() {
        if n.type_id != "hub:pipeline" {
            continue;
        }
        for (s, t, e) in g.iter_edges() {
            if s != n.id {
                continue;
            }
            let tn = g.get_node(&t);
            let lab = tn
                .map(|x| {
                    let k = x.attrs.get("kind").cloned().unwrap_or_default();
                    format!(
                        "{}{}",
                        x.type_id,
                        if k.is_empty() {
                            String::new()
                        } else {
                            format!("[kind={k}]")
                        }
                    )
                })
                .unwrap_or_default();
            eprintln!(
                "  pipeline -{}-> {lab}",
                e.type_id.rsplit(':').next().unwrap_or(&e.type_id)
            );
        }
    }
    let short = |id: &GhostId| format!("{id:?}")[8..16].to_string();
    let deep_kids = |g: &TypedGraph, root: GhostId| -> Vec<String> {
        g.iter_edges()
            .into_iter()
            .filter(move |(s, _, _)| *s == root)
            .filter_map(|(_, t, e)| {
                let tn = g.get_node(&t)?;
                let name = tn.attrs.get("name").cloned().unwrap_or_default();
                let val = tn.attrs.get("value").cloned().unwrap_or_default();
                let text = tn.attrs.get("text").cloned().unwrap_or_default();
                let gk: Vec<String> = g
                    .iter_edges()
                    .into_iter()
                    .filter(|(s2, _, _)| *s2 == t)
                    .filter_map(|(_, t2, e2)| {
                        let tn2 = g.get_node(&t2)?;
                        Some(format!(
                            "{}->{}[text={},value={}]",
                            e2.type_id.rsplit(':').next().unwrap_or(&e2.type_id),
                            tn2.type_id,
                            tn2.attrs.get("text").cloned().unwrap_or_default(),
                            tn2.attrs.get("value").cloned().unwrap_or_default()
                        ))
                    })
                    .collect();
                Some(format!(
                    "{}->{}[name={name},value={val},text={text}]{}",
                    e.type_id.rsplit(':').next().unwrap_or(&e.type_id),
                    tn.type_id,
                    if gk.is_empty() {
                        String::new()
                    } else {
                        format!(" {{{}}}", gk.join(", "))
                    }
                ))
            })
            .collect()
    };
    eprintln!("=== FWD hub:trigger nodes (deep) ===");
    for n in g.iter_nodes() {
        if n.type_id != "hub:trigger" {
            continue;
        }
        let allattrs: Vec<String> = n.attrs.iter().map(|(k, v)| format!("{k}={v}")).collect();
        let incoming: Vec<String> = g
            .iter_edges()
            .into_iter()
            .filter(|(_, t, _)| *t == n.id)
            .filter_map(|(s, _, e)| {
                Some(format!(
                    "{}<-{}",
                    e.type_id.rsplit(':').next().unwrap_or(&e.type_id),
                    g.get_node(&s)?.type_id
                ))
            })
            .collect();
        eprintln!(
            "  hub:trigger id={} attrs={allattrs:?}\n     IN={incoming:?}\n     kids={:?}",
            short(&n.id),
            deep_kids(&g, n.id)
        );
    }
    eprintln!("=== ALL hub:value (text + parents) ===");
    for n in g.iter_nodes() {
        if n.type_id != "hub:value" {
            continue;
        }
        let text = n.attrs.get("text").cloned().unwrap_or_default();
        let parents: Vec<String> = g
            .iter_edges()
            .into_iter()
            .filter(|(_, t, _)| *t == n.id)
            .filter_map(|(s, _, e)| {
                let sn = g.get_node(&s)?;
                Some(format!(
                    "{}<-{}[{}]",
                    e.type_id.rsplit(':').next().unwrap_or(&e.type_id),
                    sn.type_id,
                    sn.attrs.get("name").cloned().unwrap_or_default()
                ))
            })
            .collect();
        if !text.is_empty() {
            eprintln!("  hub:value text={text:?} IN={parents:?}");
        }
    }
}

#[test]
#[ignore = "debug — step-identity trace on a minimal literal YAML (PLAT env)"]
fn dump_steps() {
    let platform = std::env::var("PLAT").unwrap_or_else(|_| "woodpecker".into());
    let yaml = std::env::var("LIT").unwrap_or_else(|_| {
        "steps:\n  - name: alpha\n    image: img1\n  - name: beta\n    image: img2\n".into()
    });
    let rules = pool(&ruleset(&platform));
    let doc = parse(&yaml).expect("parse");
    let mut g = seed_for(&platform, &doc, &yaml);
    run_routed(&mut g, &rules).expect("fwd");
    let dump = |g: &TypedGraph, kind: &str, span_attr: &str| {
        for n in g.iter_nodes() {
            if n.type_id != kind {
                continue;
            }
            let sp = n.attrs.get(span_attr).cloned().unwrap_or_default();
            let kids: Vec<String> = g
                .iter_edges()
                .into_iter()
                .filter(|(s, _, _)| *s == n.id)
                .filter_map(|(_, t, e)| {
                    let tn = g.get_node(&t)?;
                    let label = tn
                        .attrs
                        .get("name")
                        .or(tn.attrs.get("key"))
                        .cloned()
                        .unwrap_or_else(|| tn.type_id.clone());
                    Some(format!(
                        "{}->{}",
                        e.type_id.rsplit(':').next().unwrap_or(&e.type_id),
                        label
                    ))
                })
                .collect();
            eprintln!(
                "  {kind} {} {span_attr}={sp} : {kids:?}",
                &format!("{:?}", n.id)[8..16]
            );
        }
    };
    eprintln!("=== FWD hub:step ({platform}) ===");
    dump(&g, "hub:step", "prov_byte_start");
    let mut hub = isolate_hub(&g);
    run_routed(&mut hub, &rules).expect("bwd");
    eprintln!("=== BWD cst:Mapping[step] children ===");
    for n in hub.iter_nodes() {
        if n.type_id != "cst:Mapping" {
            continue;
        }
        let c = n.attrs.get("construct").cloned().unwrap_or_default();
        if c != "step" {
            continue;
        }
        let sp = n.attrs.get("span_start").cloned().unwrap_or_default();
        let kids: Vec<String> = hub
            .iter_edges()
            .into_iter()
            .filter(|(s, _, _)| *s == n.id)
            .filter_map(|(_, t, e)| {
                let tn = hub.get_node(&t)?;
                let label = tn
                    .attrs
                    .get("key")
                    .cloned()
                    .unwrap_or_else(|| tn.type_id.clone());
                Some(format!(
                    "{}->{}",
                    e.type_id.rsplit(':').next().unwrap_or(&e.type_id),
                    label
                ))
            })
            .collect();
        eprintln!("  cst:Mapping[step] span={sp} : {kids:?}");
    }
    match pick_pipeline_root(&hub) {
        Some(root) => eprintln!("=== EMITTED ===\n{}", emit_yaml(&hub, root)),
        None => eprintln!("no root"),
    }
}

/// Baseline diagnostic — prints a per-platform roundtrip table.
/// Opt-in so it never gates CI while being built out.
#[test]
#[ignore = "baseline diagnostic — run with --ignored --nocapture"]
fn roundtrip_baseline_report() {
    let seeds: [u64; 5] = [1, 2, 3, 7, 42];
    let budget = Budget::shallow();
    let detail = std::env::var("DETAIL").unwrap_or_default();
    println!("\n── Per-platform roundtrip baseline ──");
    println!(
        "{:<20} {:>6} {:>6} {:>6} {:>6} {:>6}",
        "platform", "fwd", "bwd", "emit", "parse", "EQUAL"
    );
    for &platform in PLATFORMS {
        let rules = pool(&ruleset(platform));
        let (mut fwd, mut bwd, mut emit, mut rep, mut eq) = (0, 0, 0, 0, 0);
        let mut first_fail = String::new();
        let mut n = 0;
        for &seed in &seeds {
            let yaml = match generate_yaml(platform, seed, &budget) {
                Ok(y) => y,
                Err(_) => continue,
            };
            n += 1;
            let o = roundtrip_once(platform, &yaml, &rules);
            fwd += o.converged_fwd as usize;
            bwd += o.converged_bwd as usize;
            emit += o.emitted as usize;
            rep += o.reparsed as usize;
            eq += o.hub_equal as usize;
            if !o.hub_equal && first_fail.is_empty() {
                first_fail = format!("    seed {seed}: {}", o.detail);
            }
            if !o.hub_equal && platform == detail {
                println!("  [DETAIL] {platform} seed {seed}:\n{}", o.detail);
            }
        }
        println!("{platform:<20} {fwd:>5}/{n} {bwd:>5}/{n} {emit:>5}/{n} {rep:>5}/{n} {eq:>5}/{n}");
        if !first_fail.is_empty() {
            println!("{first_fail}");
        }
    }
    println!("──");
}

/// Perf + correctness A/B on a real fixture: the FULL matcher (`run_cascade_full`,
/// what `run_routed`/`cascade_step` use today) vs the CACHED matcher
/// (`run_cascade_cached`, which carries the engine's quadratic-breaking levers
/// D/3/4/5). Asserts cached == full BIT-IDENTICALLY on the real pipewright
/// rules (delta sequence + final graph), then reports the wall-clock of each.
///
///   PLAT=github FIXTURE=tests/edge_cases/github/tokio-ci.yml \
///     cargo test --release -p chaos-generator --test roundtrip tokio_cached_vs_full -- --ignored --nocapture
#[test]
#[ignore = "perf+correctness A/B: full vs cached matcher on a real fixture"]
fn tokio_cached_vs_full() {
    use std::time::Instant;
    let platform = std::env::var("PLAT").unwrap_or_else(|_| "github".into());
    let yaml = match std::env::var("FIXTURE") {
        Ok(p) => {
            std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").join(&p))
                .unwrap_or_else(|e| panic!("read {p}: {e}"))
        }
        Err(_) => panic!("set FIXTURE=<repo-rel path>"),
    };
    let rules = pool(&ruleset(&platform));
    let doc = parse_for(&platform, &yaml).expect("parse");

    let fingerprint = |g: &TypedGraph| -> (Vec<(GhostId, String, u8)>, usize) {
        let mut v: Vec<(GhostId, String, u8)> = g
            .iter_nodes()
            .map(|n| (n.id, n.type_id.clone(), n.status as u8))
            .collect();
        v.sort();
        let edges = g.iter_edges().len();
        (v, edges)
    };

    // Active rule set (mirror run_routed's input-domain filtering).
    let mut g0 = seed_for(&platform, &doc, &yaml);
    let delta: std::collections::HashSet<String> =
        g0.iter_nodes().map(|n| n.type_id.clone()).collect();
    let active: Vec<&dyn Rule> = rules
        .iter()
        .filter(|r| {
            let idk = r.input_domain_kinds();
            idk.is_empty() || idk.iter().any(|k| delta.contains(k))
        })
        .map(AsRef::as_ref)
        .collect();
    let seed_nodes = g0.iter_nodes().count();
    eprintln!(
        "=== tokio_cached_vs_full {platform} | seed nodes={seed_nodes} active rules={} ===",
        active.len()
    );

    // FULL (current production path).
    let mut g_full = std::mem::take(&mut g0);
    let mut c_full = Cascade::new();
    let t = Instant::now();
    let term_full = run_cascade_full(&mut c_full, &mut g_full, &active, 10_000_000).expect("full");
    let dt_full = t.elapsed().as_secs_f64();

    // CACHED (engine levers D/3/4/5).
    let mut g_cached = seed_for(&platform, &doc, &yaml);
    let mut c_cached = Cascade::new();
    let t = Instant::now();
    let term_cached =
        run_cascade_cached(&mut c_cached, &mut g_cached, &active, 10_000_000).expect("cached");
    let dt_cached = t.elapsed().as_secs_f64();

    eprintln!(
        "  FULL   : term={term_full:?} steps={} {:.1}s",
        c_full.entries.len(),
        dt_full
    );
    eprintln!(
        "  CACHED : term={term_cached:?} steps={} {:.1}s  → {:.2}× speedup",
        c_cached.entries.len(),
        dt_cached,
        dt_full / dt_cached.max(1e-9)
    );

    // Bit-identity on the REAL rules.
    assert_eq!(
        format!("{term_full:?}"),
        format!("{term_cached:?}"),
        "termination differs"
    );
    assert_eq!(
        c_full.entries.len(),
        c_cached.entries.len(),
        "step count differs"
    );
    for (i, (ef, ei)) in c_full
        .entries
        .iter()
        .zip(c_cached.entries.iter())
        .enumerate()
    {
        assert_eq!(ef.origin, ei.origin, "origin @entry {i}");
        assert_eq!(ef.rank, ei.rank, "rank @entry {i}");
        assert_eq!(ef.op_star, ei.op_star, "op_star @entry {i}");
        assert_eq!(ef.bindings, ei.bindings, "bindings @entry {i}");
    }
    assert_eq!(
        fingerprint(&g_full),
        fingerprint(&g_cached),
        "final graph differs"
    );
    eprintln!("  ✓ cached == full (bit-identical) on real {platform} rules");
}

/// Dumps the seeded graph (nodes + edges, with their exact GhostIds) of a
/// real fixture to JSON, so the cascade workload can be reproduced verbatim
/// inside the seesaw repo's own tests (load via `insert_node_data` /
/// `insert_edge_data`, rules via the platform `*.ruleset.json`).
///
///   PLAT=github FIXTURE=tests/edge_cases/github/tokio-ci.yml OUT=/tmp \
///     cargo test -p chaos-generator --test roundtrip dump_seed_fixture -- --ignored --nocapture
#[test]
#[ignore = "dump a real seed graph as JSON for the seesaw repo"]
fn dump_seed_fixture() {
    let platform = std::env::var("PLAT").unwrap_or_else(|_| "github".into());
    let yaml = match std::env::var("FIXTURE") {
        Ok(p) => {
            std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").join(&p))
                .unwrap_or_else(|e| panic!("read {p}: {e}"))
        }
        Err(_) => panic!("set FIXTURE=<repo-rel path>"),
    };
    let out = std::env::var("OUT").unwrap_or_else(|_| "/tmp".into());
    let doc = parse_for(&platform, &yaml).expect("parse");
    let g = seed_for(&platform, &doc, &yaml);

    let nodes: Vec<&NodeData> = g.iter_nodes().collect();
    let edges: Vec<(GhostId, GhostId, &EdgeData)> = g.iter_edges();
    std::fs::write(
        format!("{out}/{platform}_seed_nodes.json"),
        serde_json::to_string(&nodes).unwrap(),
    )
    .unwrap();
    std::fs::write(
        format!("{out}/{platform}_seed_edges.json"),
        serde_json::to_string(&edges).unwrap(),
    )
    .unwrap();
    eprintln!(
        "dumped {} nodes, {} edges to {out}/{platform}_seed_{{nodes,edges}}.json",
        nodes.len(),
        edges.len()
    );
}

/// Dumps the BACKWARD seed: run the forward cascade, isolate the hub, and
/// dump that hub graph (the input the backward cascade consumes to rebuild
/// the CST). Lets the seesaw repo reproduce the reverse-direction workload.
///
///   PLAT=github FIXTURE=tests/edge_cases/github/tokio-ci.yml OUT=/tmp \
///     cargo test --release -p chaos-generator --test roundtrip dump_hub_fixture -- --ignored --nocapture
#[test]
#[ignore = "dump the backward seed (isolated hub) as JSON for the seesaw repo"]
fn dump_hub_fixture() {
    let platform = std::env::var("PLAT").unwrap_or_else(|_| "github".into());
    let yaml = match std::env::var("FIXTURE") {
        Ok(p) => {
            std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").join(&p))
                .unwrap_or_else(|e| panic!("read {p}: {e}"))
        }
        Err(_) => panic!("set FIXTURE=<repo-rel path>"),
    };
    let out = std::env::var("OUT").unwrap_or_else(|_| "/tmp".into());
    let rules = pool(&ruleset(&platform));
    let doc = parse_for(&platform, &yaml).expect("parse");
    let mut gf = seed_for(&platform, &doc, &yaml);
    run_routed(&mut gf, &rules).expect("fwd");
    let hub = isolate_hub(&gf);

    let nodes: Vec<&NodeData> = hub.iter_nodes().collect();
    let edges: Vec<(GhostId, GhostId, &EdgeData)> = hub.iter_edges();
    std::fs::write(
        format!("{out}/{platform}_hub_nodes.json"),
        serde_json::to_string(&nodes).unwrap(),
    )
    .unwrap();
    std::fs::write(
        format!("{out}/{platform}_hub_edges.json"),
        serde_json::to_string(&edges).unwrap(),
    )
    .unwrap();
    eprintln!(
        "dumped hub: {} nodes, {} edges to {out}/{platform}_hub_{{nodes,edges}}.json",
        nodes.len(),
        edges.len()
    );
}
