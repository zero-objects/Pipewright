//! Bidirectional roundtrip via seesaw rc7 — ONE rule pool, direction
//! routed by the delta's kinds (NOT a name-suffix filter, NOT two
//! orchestrated passes).
//!
//! Per the engine author + the `Rule::input_domain_kinds` doc: the
//! declarative `RuleSpec` is the direction-neutral object;
//! `compile_bidirectional` derives both directed rules; you register
//! BOTH; then each delta drives a direction-bundled cascade by
//! activating only the rules whose `input_domain_kinds` intersect the
//! delta's kinds. rc7 ships the compile half + the `input_domain_kinds`
//! metadata, but not the routing helper (it lives in seesaw-jni and is
//! being lifted into core) — so the consumer does the ~10-line filter.
//!
//! For a batch yaml->IR->yaml roundtrip there are two delta submissions:
//! seed the whole CST (delta kinds = cst:*) → forward rules fire →
//! hub side appears; then take the hub side as a fresh delta (kinds =
//! hub:*) → backward rules fire → cst side is rebuilt.
//!
//! Verification anchor for the `projection_rule` rewrite.

use std::path::Path;

use pipeline_cst::parse as parse_yaml;
use pipeline_tgg_seeder::emit::emit_yaml;
use seesaw_core::engine::{cascade_step, Cascade, Rule, TerminationState};
use seesaw_core::graph::{GhostId, TypedGraph};
use seesaw_core::rule::compile::compile_bidirectional;
use seesaw_core::rule::instantiate::instantiate;
use seesaw_core::rule::spec::RuleSetSpec;

fn ruleset(platform: &str) -> RuleSetSpec {
    let p = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../catalog/rules")
        .join(format!("{platform}.ruleset.json"));
    serde_json::from_str(&std::fs::read_to_string(p).expect("read ruleset")).expect("parse ruleset")
}

/// Instantiate BOTH directed rules of every spec into one pool.
fn pool(rs: &RuleSetSpec) -> Vec<Box<dyn Rule>> {
    rs.rules
        .iter()
        .flat_map(|r| compile_bidirectional(r).expect("compile_bidirectional"))
        .map(|c| instantiate(&c))
        .collect()
}

/// The kinds currently present in the graph = the "delta" kinds for a
/// batch submission of the whole seeded side.
fn graph_kinds(g: &TypedGraph) -> std::collections::HashSet<String> {
    g.iter_nodes().map(|n| n.type_id.clone()).collect()
}

/// Drive the cascade with only the rules whose `input_domain_kinds`
/// intersect the delta's kinds (or are empty). This is the consumer
/// side of the direction-bundling seesaw documents on `Rule`.
fn run_routed(graph: &mut TypedGraph, rules: &[Box<dyn Rule>]) {
    let delta = graph_kinds(graph);
    let active: Vec<&dyn Rule> = rules
        .iter()
        .filter(|r| {
            let idk = r.input_domain_kinds();
            idk.is_empty() || idk.iter().any(|k| delta.contains(k))
        })
        .map(AsRef::as_ref)
        .collect();
    let mut cascade = Cascade::new();
    for _ in 0..10_000 {
        match cascade_step(&mut cascade, graph, &active).expect("cascade step") {
            TerminationState::Running => continue,
            _ => return,
        }
    }
    panic!("cascade did not converge");
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

fn count_kind(g: &TypedGraph, kind: &str) -> usize {
    g.iter_nodes().filter(|n| n.type_id == kind).count()
}

fn has_mapping_construct(g: &TypedGraph, construct: &str) -> bool {
    g.iter_nodes().any(|n| {
        n.type_id == "cst:Mapping"
            && n.attrs.get("construct").map(String::as_str) == Some(construct)
    })
}

/// Pick the top-level pipeline cst:Mapping (construct=pipeline, has
/// outgoing `has_child`, not itself a mapping-entry value).
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
        // rc8 first-class: the pipeline root no longer needs a cst:has_child
        // entry — name-keyed containment (jobs) is emit-derived from the hub
        // has_job edge, not a cst entry. Just pick the non-inner pipeline.
        .find(|m| !inner.contains(m))
}

/// Full YAML -> hub -> YAML roundtrip. Two delta submissions, direction
/// routed by `input_domain_kinds` each time.
fn roundtrip_gitlab(yaml: &str) -> Result<String, String> {
    let doc = parse_yaml(yaml).map_err(|e| format!("parse: {e:?}"))?;
    let mut g = pipeline_tgg_seeder::platforms::gitlab::seed_from_document(&doc, yaml).graph;
    let rules = pool(&ruleset("gitlab"));
    run_routed(&mut g, &rules); // cst:* delta → forward rules → hub side
    let mut hub = isolate_hub(&g);
    run_routed(&mut hub, &rules); // hub:* delta → backward rules → cst side
    let root =
        pick_pipeline_root(&hub).ok_or_else(|| "no reconstructed pipeline root".to_string())?;
    Ok(emit_yaml(&hub, root))
}

/// gitlab keyless top-level job: `build:` IS the job. Forward must
/// produce a hub:job; backward must reconstruct pipeline + job mappings.
#[test]
fn gitlab_keyless_job_roundtrips_natively() {
    let yaml = "build:\n  script:\n    - echo hi\n";
    let doc = parse_yaml(yaml).expect("parse input");
    let mut g = pipeline_tgg_seeder::platforms::gitlab::seed_from_document(&doc, yaml).graph;
    let rules = pool(&ruleset("gitlab"));

    run_routed(&mut g, &rules);
    assert!(
        count_kind(&g, "hub:job") >= 1,
        "forward must create at least one hub:job, got {}",
        count_kind(&g, "hub:job"),
    );

    let mut hub = isolate_hub(&g);
    assert_eq!(
        count_kind(&hub, "cst:Mapping"),
        0,
        "isolated hub graph must contain no cst nodes before the backward delta",
    );

    run_routed(&mut hub, &rules);
    assert!(
        count_kind(&hub, "cst:Mapping") > 0,
        "backward delta must reconstruct cst:Mapping nodes from the hub graph",
    );
    assert!(
        has_mapping_construct(&hub, "pipeline"),
        "backward delta must reconstruct the pipeline cst:Mapping",
    );
    assert!(
        has_mapping_construct(&hub, "job"),
        "backward delta must reconstruct the job cst:Mapping",
    );
}

// ── The four carrier-elimination bug classes, retested natively ──────
//
// PROGRESS MARKERS for the projection_rule rewrite. Un-ignore each as
// the rewrite lands its projection class.

#[test]
fn bug1_script_content_survives() {
    let yaml = "build:\n  script:\n    - echo hi\n    - cargo build\n";
    let emitted = roundtrip_gitlab(yaml).expect("roundtrip");
    std::fs::write("/tmp/bug1.yml", &emitted).ok();
    assert!(
        emitted.contains("echo hi"),
        "script line 1 lost:\n{emitted}"
    );
    assert!(
        emitted.contains("cargo build"),
        "script line 2 lost:\n{emitted}"
    );
}

// MEASURED (diag, full pool): FORWARD is correct — hub:artifact created,
// hub:attr{name,value} = {name: build-${VAR}, paths: target/release/bin}.
// BACKWARD loses the artifact's inner fields: only `artifacts:`+`build:`
// entries come back, scalar text only "build". Root cause: artifact.name
// / artifact.paths are scalar_attr/seq_attr shapes NOT yet migrated to
// the unified attr->value form, so they don't rematerialize backward
// under the field-entered artifact construct. Fix = migrate scalar_attr/
// seq_attr (next plan shape group).
//
// UPDATE: scalar_attr/seq_attr ARE now unified (hub:value), proven in
// unified_field_model.rs both directions. But bug2 still RED — measured:
// the emit yields only `build:`, i.e. the `artifacts:` CONSTRUCT itself is
// not rebuilt backward (not merely its scalar leaves). So the gap is the
// field-entered artifact path (job.artifacts ref field -> hub:artifact via
// mapping_node/seq_mapping_nodes), not the scalar shapes. Needs its own
// diagnosis.
#[test]
fn bug2_artifact_variable_survives() {
    let yaml = "build:\n  artifacts:\n    name: \"build-${CI_COMMIT_SHORT_SHA}\"\n    paths:\n      - target/release/bin\n";
    let emitted = roundtrip_gitlab(yaml).expect("roundtrip");
    std::fs::write("/tmp/bug2.yml", &emitted).ok();
    assert!(
        emitted.contains("${CI_COMMIT_SHORT_SHA}"),
        "artifact name variable lost:\n{emitted}",
    );
    assert!(
        !emitted.contains("name: artifacts"),
        "field key 'artifacts' leaked into the name:\n{emitted}",
    );
}

#[test]
fn bug3_stepless_job_survives() {
    let yaml = "deploy:\n  image: alpine\n";
    let emitted = roundtrip_gitlab(yaml).expect("roundtrip");
    std::fs::write("/tmp/bug3.yml", &emitted).ok();
    assert!(
        emitted.contains("deploy"),
        "step-less job 'deploy' lost:\n{emitted}"
    );
}

// FIXED (deep root, Sandra's cause #3 over-reduction): a `union` ref field's
// SCALAR arm is now `scalar_attr` (first-class hub:<parent>.<field>), not a
// hub-construct (scalar_node). `image: gamma` → pipeline.image="gamma", emit
// `image: gamma`. The scalar arm no longer shares a hub:image with the mapping
// arm, so the backward arm-conflict (the cst tangle) is gone and the value
// survives. The mapping arm stays a construct.
#[test]
fn bug4_pipeline_field_stays_at_root() {
    let yaml = "image: gamma\nbuild:\n  script:\n    - echo hi\n";
    let emitted = roundtrip_gitlab(yaml).expect("roundtrip");
    std::fs::write("/tmp/bug4.yml", &emitted).ok();
    let at_root = emitted
        .lines()
        .any(|l| l == "image: gamma" || l.starts_with("image:"));
    assert!(
        at_root,
        "pipeline-level image did not survive at root:\n{emitted}"
    );
    assert!(emitted.contains("build"), "job 'build' lost:\n{emitted}");
}

// ── THROWAWAY Phase-1 diagnostic probes (delete after bug2/bug4 fixed) ──
// Dump the hub graph after forward AND the cst graph after backward, so we
// can see at WHICH boundary the construct is lost — measure, don't guess.

fn dump_graph(g: &TypedGraph, label: &str) -> String {
    let mut d = format!("=== {label} ===\n");
    let mut nodes: Vec<_> = g.iter_nodes().collect();
    nodes.sort_by(|a, b| a.type_id.cmp(&b.type_id));
    for n in &nodes {
        let mut bits = vec![];
        for k in ["construct", "key", "name", "prov_key", "text"] {
            if let Some(v) = n.attrs.get(k) {
                bits.push(format!("{k}={v}"));
            }
        }
        d.push_str(&format!("NODE {} [{}]\n", n.type_id, bits.join(" ")));
    }
    for (s, t, e) in g.iter_edges() {
        let sk = g
            .get_node(&s)
            .map(|n| n.type_id.clone())
            .unwrap_or_default();
        let tk = g
            .get_node(&t)
            .map(|n| n.type_id.clone())
            .unwrap_or_default();
        d.push_str(&format!("EDGE {} {sk} -> {tk}\n", e.type_id));
    }
    d
}

fn probe(yaml: &str, tag: &str) {
    let doc = parse_yaml(yaml).expect("parse");
    let mut g = pipeline_tgg_seeder::platforms::gitlab::seed_from_document(&doc, yaml).graph;
    let rules = pool(&ruleset("gitlab"));
    run_routed(&mut g, &rules);
    let fwd = dump_graph(&g, "HUB after FORWARD (hub nodes only shown)");
    let hub_only = isolate_hub(&g);
    let fwd_hub = dump_graph(&hub_only, "ISOLATED HUB (input to backward)");
    let mut hub = isolate_hub(&g);
    run_routed(&mut hub, &rules);
    let bwd = dump_graph(&hub, "AFTER BACKWARD (cst rebuilt)");
    let emitted = roundtrip_gitlab(yaml).unwrap_or_else(|e| format!("<emit failed: {e}>"));
    let out = format!("{fwd}\n{fwd_hub}\n{bwd}\n=== EMIT ===\n{emitted}\n");
    std::fs::write(format!("/tmp/probe_{tag}.txt"), &out).ok();
    eprintln!("{out}");
}

#[test]
fn probe_bug2() {
    probe(
        "build:\n  artifacts:\n    name: \"build-${CI_COMMIT_SHORT_SHA}\"\n    paths:\n      - target/release/bin\n",
        "bug2",
    );
}

#[test]
fn probe_bug4() {
    probe("image: gamma\nbuild:\n  script:\n    - echo hi\n", "bug4");
}

// A scalar field is captured STRUCTURALLY after seed+forward, on real data:
// the value rides hub:artifact -has_attr-> hub:attr{name=expire_in}
// -has_value-> hub:value{text=…}. (Supersedes the rc8 first-class probe: the
// unified hub:value scalar_attr is key-gated and bijective; the bare-MC
// first-class form oscillated whenever sibling keys mapped to one IR field.)
#[test]
fn probe_firstclass_forward() {
    let yaml = "build:\n  artifacts:\n    expire_in: 1 week\n";
    let doc = parse_yaml(yaml).expect("parse");
    let mut g = pipeline_tgg_seeder::platforms::gitlab::seed_from_document(&doc, yaml).graph;
    let rules = pool(&ruleset("gitlab"));
    run_routed(&mut g, &rules);
    let art = g
        .iter_nodes()
        .find(|n| n.type_id == "hub:artifact")
        .map(|n| n.id);
    // Walk hub:artifact -has_attr-> attr{name=expire_in} -has_value-> value.
    let value_text = art.and_then(|aid| {
        let attr = g.iter_edges().into_iter().find_map(|(s, t, e)| {
            (s == aid && e.type_id == "hub:has_attr")
                .then_some(t)
                .filter(|t| {
                    g.get_node(t)
                        .and_then(|n| n.attrs.get("name"))
                        .map(String::as_str)
                        == Some("expire_in")
                })
        })?;
        let val = g
            .iter_edges()
            .into_iter()
            .find_map(|(s, t, e)| (s == attr && e.type_id == "hub:has_value").then_some(t))?;
        g.get_node(&val).and_then(|n| n.attrs.get("text")).cloned()
    });
    let report = format!(
        "expire_in value text={value_text:?} (art_exists={})",
        art.is_some()
    );
    eprintln!("{report}");
    assert_eq!(
        value_text.as_deref(),
        Some("1 week"),
        "expire_in must be captured structurally on hub:artifact — {report}"
    );
}

// THROWAWAY: is the keyless-job containment info PRESERVED in the isolated
// hub (backward input)? Dumps the name satellite's value + has_job edge.
// Answers: IR information loss, or pure backward rule-competition?
#[test]
fn probe_containment_info_in_hub() {
    let yaml = "build:\n  script:\n    - echo hi\n";
    let doc = parse_yaml(yaml).expect("parse");
    let mut g = pipeline_tgg_seeder::platforms::gitlab::seed_from_document(&doc, yaml).graph;
    let rules = pool(&ruleset("gitlab"));
    run_routed(&mut g, &rules);
    let hub = isolate_hub(&g);
    let mut out = String::new();
    for n in hub.iter_nodes() {
        if n.type_id == "hub:attr" {
            out.push_str(&format!(
                "hub:attr name={:?} value={:?} prov_key={:?}\n",
                n.attrs.get("name"),
                n.attrs.get("value"),
                n.attrs.get("prov_key"),
            ));
        }
        if n.type_id == "hub:job" {
            out.push_str(&format!("hub:job attrs={:?}\n", n.attrs));
        }
    }
    let has_job = hub
        .iter_edges()
        .iter()
        .any(|(_, _, e)| e.type_id == "hub:has_job");
    out.push_str(&format!("has_job edge present: {has_job}\n"));
    std::fs::write("/tmp/containment_info.txt", &out).ok();
    eprintln!("{out}");
}

// THROWAWAY: backward cst shape for bug1's script chain — is hub:step
// rebuilt as a (wrong) cst:Mapping[step] instead of SequenceItem/Scalar?
#[test]
fn probe_bug1_backward_shape() {
    let yaml = "build:\n  script:\n    - echo hi\n    - cargo build\n";
    let doc = parse_yaml(yaml).expect("parse");
    let mut g = pipeline_tgg_seeder::platforms::gitlab::seed_from_document(&doc, yaml).graph;
    let rules = pool(&ruleset("gitlab"));
    run_routed(&mut g, &rules);
    // FORWARD graph (before isolate): how many has_item / hub:step?
    {
        let fi = g
            .iter_edges()
            .iter()
            .filter(|(_, _, e)| e.type_id == "hub:has_item")
            .count();
        let fs = g.iter_nodes().filter(|n| n.type_id == "hub:step").count();
        let fc = g
            .iter_nodes()
            .filter(|n| n.type_id == "hub:collection")
            .count();
        let mut d = format!("FORWARD g: hub:has_item={fi} hub:step={fs} hub:collection={fc}\n");
        // seeded cst:SequenceItem anchors — are they unique?
        for n in g.iter_nodes() {
            if n.type_id == "cst:SequenceItem" {
                d.push_str(&format!(
                    "cst:SequenceItem {} {:?}\n",
                    n.id.short(),
                    n.attrs
                ));
            }
        }
        // each hub:step's cst anchor: step <-corrR- refines <-corrL- cst
        for n in g.iter_nodes() {
            if n.type_id == "hub:step" {
                let refines = g
                    .iter_edges()
                    .into_iter()
                    .find(|(_, t, e)| *t == n.id && e.type_id == "corrR")
                    .map(|(s, _, _)| s);
                let anchor = refines.and_then(|r| {
                    g.iter_edges()
                        .into_iter()
                        .find(|(_, t, e)| *t == r && e.type_id == "corrL")
                        .map(|(s, _, _)| {
                            g.get_node(&s)
                                .map(|x| format!("{} {}", x.type_id, s.short()))
                                .unwrap_or_default()
                        })
                });
                d.push_str(&format!("hub:step {} anchor: {anchor:?}\n", n.id.short()));
            }
        }
        for n in g.iter_nodes() {
            if n.type_id == "hub:step" {
                let inc: Vec<String> = g
                    .iter_edges()
                    .iter()
                    .filter(|(_, t, _)| *t == n.id)
                    .map(|(s, _, e)| {
                        format!(
                            "{}<--{}",
                            g.get_node(s).map(|x| x.type_id.clone()).unwrap_or_default(),
                            e.type_id
                        )
                    })
                    .collect();
                d.push_str(&format!("hub:step {} incoming: {inc:?}\n", n.id.short()));
            }
            if n.type_id == "hub:collection" {
                let out: Vec<String> = g
                    .iter_edges()
                    .iter()
                    .filter(|(s, _, _)| *s == n.id)
                    .map(|(_, t, e)| {
                        format!(
                            "{}-->{}",
                            e.type_id,
                            g.get_node(t).map(|x| x.type_id.clone()).unwrap_or_default()
                        )
                    })
                    .collect();
                d.push_str(&format!(
                    "hub:collection {} outgoing: {out:?}\n",
                    n.id.short()
                ));
            }
        }
        eprintln!("{d}");
        std::fs::write("/tmp/bug1_fwd.txt", &d).ok();
    }
    let mut hub = isolate_hub(&g);
    let mut out = String::from("=== ISOLATED HUB step/attr ===\n");
    for n in hub.iter_nodes() {
        if matches!(
            n.type_id.as_str(),
            "hub:step" | "hub:attr" | "hub:collection"
        ) {
            out.push_str(&format!("{} {:?}\n", n.type_id, n.attrs));
        }
    }
    let n_has_item = hub
        .iter_edges()
        .iter()
        .filter(|(_, _, e)| e.type_id == "hub:has_item")
        .count();
    out.push_str(&format!(
        "hub:has_item edges in isolated hub: {n_has_item}\n"
    ));
    run_routed(&mut hub, &rules);
    out.push_str("=== AFTER BACKWARD cst nodes ===\n");
    for n in hub.iter_nodes() {
        if n.type_id.starts_with("cst:") {
            let k = n
                .attrs
                .get("construct")
                .or(n.attrs.get("key"))
                .or(n.attrs.get("text"))
                .cloned()
                .unwrap_or_default();
            out.push_str(&format!("{} [{k}]\n", n.type_id));
        }
    }
    std::fs::write("/tmp/bug1_bwd.txt", &out).ok();
    eprintln!("{out}");
}

// THROWAWAY (cascade tracer): per forward application, which has_item ops
// are EMITTED — pins (A) produce()-gap [op never emitted] vs (B) backtrack
// [op emitted but final graph lacks the edge].
#[test]
fn probe_cascade_trace_hasitem() {
    use seesaw_core::ops::Op;
    let yaml = "build:\n  script:\n    - echo hi\n    - cargo build\n";
    let doc = parse_yaml(yaml).expect("parse");
    let mut g = pipeline_tgg_seeder::platforms::gitlab::seed_from_document(&doc, yaml).graph;
    let rules = pool(&ruleset("gitlab"));
    let delta = graph_kinds(&g);
    let active: Vec<&dyn Rule> = rules
        .iter()
        .filter(|r| {
            let idk = r.input_domain_kinds();
            idk.is_empty() || idk.iter().any(|k| delta.contains(k))
        })
        .map(AsRef::as_ref)
        .collect();
    let mut cascade = Cascade::new();
    let mut log = String::new();
    let mut emitted = 0usize;
    for _ in 0..10_000 {
        let before = cascade.entries.len();
        match cascade_step(&mut cascade, &mut g, &active).expect("step") {
            TerminationState::Running => {
                if cascade.entries.len() > before {
                    let e = cascade.entries.last().unwrap();
                    let ops: Vec<String> = e
                        .op_star
                        .iter()
                        .map(|op| match op {
                            Op::AddNode { type_id, .. } => format!("+N {type_id}"),
                            Op::AddEdge {
                                type_id,
                                source,
                                target,
                                ..
                            } => {
                                if type_id == "hub:has_item" {
                                    emitted += 1;
                                }
                                format!("+E {type_id} {}->{}", source.short(), target.short())
                            }
                            other => format!("{other:?}"),
                        })
                        .collect();
                    let oid = format!("{:?}", e.origin);
                    if oid.contains("script")
                        || ops
                            .iter()
                            .any(|o| o.contains("has_item") || o.contains("hub:step"))
                    {
                        log.push_str(&format!("{oid}\n    {}\n", ops.join("\n    ")));
                    }
                }
            }
            other => {
                log.push_str(&format!("TERMINATED: {other:?}\n"));
                break;
            }
        }
    }
    let final_hi = g
        .iter_edges()
        .iter()
        .filter(|(_, _, e)| e.type_id == "hub:has_item")
        .count();
    log.push_str(&format!(
        "TOTAL has_item ops EMITTED: {emitted}\nFINAL has_item edges: {final_hi}\n"
    ));
    std::fs::write("/tmp/cascade_trace.txt", &log).ok();
    eprintln!("{log}");
}

// THROWAWAY: which BACKWARD rules actually FIRE (apply ops), step by step.
#[test]
fn probe_backward_fired() {
    let yaml = "build:\n  script:\n    - echo hi\n";
    let doc = parse_yaml(yaml).expect("parse");
    let mut g = pipeline_tgg_seeder::platforms::gitlab::seed_from_document(&doc, yaml).graph;
    let rules = pool(&ruleset("gitlab"));
    run_routed(&mut g, &rules); // forward
    let mut hub = isolate_hub(&g);
    let delta = graph_kinds(&hub);
    let active: Vec<&dyn Rule> = rules
        .iter()
        .filter(|r| {
            let idk = r.input_domain_kinds();
            idk.is_empty() || idk.iter().any(|k| delta.contains(k))
        })
        .map(AsRef::as_ref)
        .collect();
    let mut cascade = Cascade::new();
    let mut fired: Vec<String> = Vec::new();
    for _ in 0..10_000 {
        let before = cascade.entries.len();
        match cascade_step(&mut cascade, &mut hub, &active).expect("step") {
            TerminationState::Running => {
                if cascade.entries.len() > before {
                    if let Some(e) = cascade.entries.last() {
                        fired.push(format!("{:?}", e.origin));
                    }
                }
            }
            other => {
                fired.push(format!("TERMINATED: {other:?}"));
                break;
            }
        }
    }
    let out = format!(
        "backward fired ({} steps):\n{}\n",
        fired.len(),
        fired.join("\n")
    );
    std::fs::write("/tmp/backward_fired.txt", &out).ok();
    eprintln!("{out}");
}

#[test]
fn probe_backward_routing() {
    let yaml = "build:\n  script:\n    - echo hi\n";
    let doc = parse_yaml(yaml).expect("parse");
    let mut g = pipeline_tgg_seeder::platforms::gitlab::seed_from_document(&doc, yaml).graph;
    let rules = pool(&ruleset("gitlab"));
    run_routed(&mut g, &rules); // forward
    let hub = isolate_hub(&g);
    let delta = graph_kinds(&hub);
    let mut out = format!("hub delta kinds: {:?}\n\n", {
        let mut v: Vec<_> = delta.iter().cloned().collect();
        v.sort();
        v
    });
    let (mut act, mut inact) = (0usize, 0usize);
    for r in &rules {
        let id = r.id();
        if !id.ends_with('\u{2190}') {
            continue;
        } // backward rules only
        let idk = r.input_domain_kinds();
        let active = idk.is_empty() || idk.iter().any(|k| delta.contains(k));
        if active {
            act += 1;
        } else {
            inact += 1;
        }
        out.push_str(&format!("{id} active={active} idk={idk:?}\n"));
    }
    out.push_str(&format!(
        "\nBACKWARD rules: {act} active, {inact} inactive\n"
    ));
    std::fs::write("/tmp/backward_routing.txt", &out).ok();
    eprintln!("{out}");
}

// THROWAWAY (rc8 diagnostic): compile EVERY rule of EVERY platform ruleset
// and collect the rc8 created-node-invariant violations
// (CreatedNodeWithoutCorrespondence) — the exhaustive "anchorless node"
// worklist. Does NOT .expect, so it reports all sites instead of panicking
// on the first.
#[test]
fn probe_anchorless_nodes() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../catalog/rules");
    let mut platforms: Vec<String> = std::fs::read_dir(&dir)
        .expect("read rules dir")
        .filter_map(std::result::Result::ok)
        .filter_map(|e| e.file_name().into_string().ok())
        .filter_map(|n| n.strip_suffix(".ruleset.json").map(String::from))
        .collect();
    platforms.sort();

    let mut report = String::new();
    let mut total_fail = 0usize;
    for plat in &platforms {
        let rs = ruleset(plat);
        let mut fails: Vec<String> = Vec::new();
        for r in &rs.rules {
            if let Err(e) = compile_bidirectional(r) {
                fails.push(format!("    {}  ::  {:?}", r.name, e));
            }
        }
        report.push_str(&format!(
            "{plat}: {} rules, {} FAIL\n",
            rs.rules.len(),
            fails.len()
        ));
        for f in &fails {
            report.push_str(f);
            report.push('\n');
        }
        total_fail += fails.len();
    }
    let header = format!(
        "=== rc8 created-node invariant: {total_fail} violations across {} platforms ===\n",
        platforms.len()
    );
    let out = format!("{header}{report}");
    std::fs::write("/tmp/anchorless.txt", &out).ok();
    eprintln!("{out}");
}
