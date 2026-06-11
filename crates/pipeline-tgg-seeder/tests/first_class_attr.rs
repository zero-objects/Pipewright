//! PROTOTYPE (rc8 re-architecture): prove the FIRST-CLASS attribute idiom.
//!
//! Sandra's call: satellites (hub:attr{name,value} nodes on `has_attr` edges)
//! are 3rd-class — an rc6 workaround for "the engine cannot bind onto a
//! shared-anchor node". rc8 (corr-rooted identity + identity decoupled from
//! propagated attrs) lifts that limit: a scalar field can live as a REAL
//! attribute ON the construct node, propagated through the correspondence —
//! exactly the fase2019 idiom (Class.name <-> DocFile.content via the
//! c<->d corr's `attribute_bindings`, no satellite).
//!
//! This headless proof uses TWO rules, no satellites at all:
//!   construct rule: MC(cst:Mapping[construct=job]) <-> hubC(hub:job)
//!       role=Establishes (creates the construct, anchored by spans).
//!   field rule:     MC(cst:Mapping[construct=job]) <-> hubC(hub:job)
//!       role=Establishes, `attribute_bindings`=[timeout->timeout]
//!       — the construct rule already established MC<->hubC, so rc8's
//!       reuse-recognition (instantiate.rs ~110-175) detects the anchor is
//!       already translated, REUSES the partner, and propagates the bound
//!       attribute via `Op::SetAttr`. NOT References: a pure References corr
//!       is only a match constraint and does NOT propagate. Result: the
//!       scalar field rides first-class on hub:job. No hub:attr node, no
//!       `has_attr` edge, no name satellite.
//!
//! The scalar value rides on MC as its own cst attribute (the seeder would
//! lift `timeout: 1h` onto the construct mapping). Measures: does the attr
//! survive forward (onto hub:job) AND backward (back onto cst:Mapping),
//! with ZERO satellite nodes and the rc8 created-node invariant intact.

use std::collections::BTreeMap;

use seesaw_core::engine::{cascade_step, Cascade, Rule, TerminationState};
use seesaw_core::graph::TypedGraph;
use seesaw_core::rule::compile::compile_bidirectional;
use seesaw_core::rule::instantiate::instantiate;
use seesaw_core::rule::spec::RuleSpec;

fn construct_rule() -> RuleSpec {
    let json = r#"{
      "name": "R_job_construct",
      "rank": 60,
      "documentation": "construct identity: cst:Mapping[job] <-> hub:job",
      "l_pattern": {
        "nodes": [
          {"id":"MC","kind":"cst:Mapping","constraints":[{"name":"construct","matcher":{"type":"literal","value":"job"}}]}
        ],
        "edges": []
      },
      "r_pattern": {
        "nodes": [
          {"id":"hubC","kind":"hub:job","constraints":[]}
        ],
        "edges": []
      },
      "correspondence_links": [
        {"l_node_id":"MC","r_node_id":"hubC","kind":"tgg:refines","role":"Establishes","attribute_bindings":[
          {"l_attr_name":"span_start","r_attr_name":"prov_byte_start","transformation":"identity"},
          {"l_attr_name":"span_end","r_attr_name":"prov_byte_end","transformation":"identity"}]}
      ],
      "nacs": []
    }"#;
    serde_json::from_str(json).expect("parse construct rule")
}

fn field_rule() -> RuleSpec {
    let json = r#"{
      "name": "R_job_timeout_FIRSTCLASS",
      "rank": 50,
      "documentation": "first-class scalar field: hub:job.timeout, no satellite",
      "l_pattern": {
        "nodes": [
          {"id":"MC","kind":"cst:Mapping","constraints":[{"name":"construct","matcher":{"type":"literal","value":"job"}}]}
        ],
        "edges": []
      },
      "r_pattern": {
        "nodes": [
          {"id":"hubC","kind":"hub:job","constraints":[]}
        ],
        "edges": []
      },
      "correspondence_links": [
        {"l_node_id":"MC","r_node_id":"hubC","kind":"tgg:refines","role":"Establishes","attribute_bindings":[
          {"l_attr_name":"timeout","r_attr_name":"timeout","transformation":"identity"},
          {"l_attr_name":"span_start","r_attr_name":"prov_byte_start","transformation":"identity"},
          {"l_attr_name":"span_end","r_attr_name":"prov_byte_end","transformation":"identity"}]}
      ],
      "nacs": []
    }"#;
    serde_json::from_str(json).expect("parse field rule")
}

fn pool() -> Vec<Box<dyn Rule>> {
    [construct_rule(), field_rule()]
        .iter()
        .flat_map(|r| compile_bidirectional(r).expect("compile_bidirectional"))
        .map(|c| instantiate(&c))
        .collect()
}

fn graph_kinds(g: &TypedGraph) -> std::collections::HashSet<String> {
    g.iter_nodes().map(|n| n.type_id.clone()).collect()
}

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

fn attrs(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
        .collect()
}

#[test]
fn first_class_scalar_attr_roundtrips() {
    // Seed: a job construct mapping carrying `timeout` as its OWN cst
    // attribute (the lifted form — no MappingEntry/Scalar child, no
    // satellite). Spans present so the corr anchor is deterministic.
    let mut g = TypedGraph::new();
    g.add_baseline_node(
        "cst:Mapping",
        "mc_job",
        attrs(&[
            ("construct", "job"),
            ("timeout", "1h"),
            ("span_start", "0"),
            ("span_end", "9"),
        ]),
    );
    let rules = pool();

    // FORWARD: cst -> hub. The construct rule creates hub:job; the
    // first-class field rule propagates timeout onto it.
    run_routed(&mut g, &rules);

    let fwd_job = g
        .iter_nodes()
        .find(|n| n.type_id == "hub:job")
        .expect("forward must create exactly one hub:job");
    let fwd_timeout = fwd_job.attrs.get("timeout").cloned().unwrap_or_default();
    let n_satellites = g.iter_nodes().filter(|n| n.type_id == "hub:attr").count();
    let n_has_attr = g
        .iter_edges()
        .iter()
        .filter(|(_, _, e)| e.type_id == "hub:has_attr")
        .count();
    let report_fwd = format!(
        "FWD: hub:job.timeout={fwd_timeout:?} hub:attr_satellites={n_satellites} has_attr_edges={n_has_attr}"
    );
    std::fs::write("/tmp/fca_fwd.txt", &report_fwd).ok();
    eprintln!("{report_fwd}");

    assert_eq!(
        fwd_timeout, "1h",
        "timeout must ride first-class on hub:job — {report_fwd}"
    );
    assert_eq!(
        n_satellites, 0,
        "NO hub:attr satellite nodes — {report_fwd}"
    );
    assert_eq!(n_has_attr, 0, "NO has_attr edges — {report_fwd}");

    // BACKWARD: hub -> cst. Construct rule rebuilds cst:Mapping[job];
    // field rule propagates timeout back onto it.
    let mut hub = isolate_hub(&g);
    run_routed(&mut hub, &rules);

    let bwd_map = hub
        .iter_nodes()
        .find(|n| {
            n.type_id == "cst:Mapping"
                && n.attrs.get("construct").map(String::as_str) == Some("job")
        })
        .expect("backward must rebuild cst:Mapping[construct=job]");
    let bwd_timeout = bwd_map.attrs.get("timeout").cloned().unwrap_or_default();
    let report_bwd = format!("BWD: cst:Mapping[job].timeout={bwd_timeout:?}");
    std::fs::write("/tmp/fca_bwd.txt", &report_bwd).ok();
    eprintln!("{report_bwd}");

    assert_eq!(
        bwd_timeout, "1h",
        "timeout must survive backward onto cst:Mapping — {report_bwd}"
    );
}

// rc8 created-node invariant: both rules must compile clean (no
// uncorresponded created node). The construct rule's hubC is corr-anchored;
// the field rule creates nothing (pure propagation onto the reused partner).
#[test]
fn first_class_rules_satisfy_created_node_invariant() {
    for r in [construct_rule(), field_rule()] {
        let name = r.name.clone();
        compile_bidirectional(&r)
            .unwrap_or_else(|e| panic!("{name} must compile clean under rc8: {e:?}"));
    }
}

// ROLLOUT Stage 3: prove emit renders a construct's first-class scalar
// attribute as a YAML entry — on a backward-STYLE cst (the construct mapping
// carries `timeout` as an attribute, with NO MappingEntry→Scalar child, as
// the backward first-class field rule would leave it).
#[test]
fn emit_renders_first_class_scalar_field() {
    use pipeline_tgg_seeder::{add_child, emit::emit_yaml};
    let mut g = TypedGraph::new();
    let p = g.add_baseline_node(
        "cst:Mapping",
        "p",
        attrs(&[
            ("construct", "pipeline"),
            ("span_start", "0"),
            ("span_end", "30"),
        ]),
    );
    let e = add_child(
        &mut g,
        p,
        "cst:has_child",
        "cst:MappingEntry",
        attrs(&[("key", "build"), ("span_start", "0"), ("span_end", "30")]),
    );
    let _j = add_child(
        &mut g,
        e,
        "cst:value_of",
        "cst:Mapping",
        attrs(&[
            ("construct", "job"),
            ("timeout", "1h"),
            ("span_start", "7"),
            ("span_end", "30"),
        ]),
    );
    let out = emit_yaml(&g, p);
    eprintln!("{out}");
    assert!(
        out.contains("build:"),
        "job containment entry must emit:\n{out}"
    );
    assert!(
        out.contains("timeout: 1h"),
        "first-class scalar field must emit as a YAML entry:\n{out}"
    );
}

// ROLLOUT Stage 1: prove the seeder lift pass on REAL seeded gitlab data —
// a scalar job field (`timeout: 1h`) is lifted onto the construct mapping
// as a first-class cst attribute, ready for a first-class field rule. No
// satellite involved.
#[test]
fn lift_pass_sets_scalar_attr_on_construct_from_real_seed() {
    use pipeline_cst::parse as parse_yaml;
    let yaml = "build:\n  timeout: 1h\n";
    let doc = parse_yaml(yaml).expect("parse");
    let mut g = pipeline_tgg_seeder::platforms::gitlab::seed_from_document(&doc, yaml).graph;

    pipeline_tgg_seeder::lift_scalar_fields(&mut g);

    let job = g
        .iter_nodes()
        .find(|n| {
            n.type_id == "cst:Mapping"
                && n.attrs.get("construct").map(String::as_str) == Some("job")
        })
        .expect("seeded gitlab `build:` must tag a cst:Mapping[construct=job]");
    assert_eq!(
        job.attrs.get("timeout").map(String::as_str),
        Some("1h"),
        "lift must set the scalar field `timeout` first-class on the job construct mapping; attrs={:?}",
        job.attrs,
    );
}
