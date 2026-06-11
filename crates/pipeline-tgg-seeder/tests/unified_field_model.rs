//! Headless validation of the UNIFIED field-value topology (design
//! 2026-05-31): every field is `hub:attr{name} -has_value-> {value |
//! collection}`, and a collection's items are full constructs whose leaf
//! content hangs underneath. Same shape for scalar-list and ref-list.
//!
//! Test rule = a hand-built `RuleSpec` for gitlab `script: [a, b]` in the
//! new topology:
//!   L: MC(job) -`has_child`-> S(key=script) -`value_of`-> SEQ
//!        -`has_child`-> IT -`value_of`-> SC
//!   R: hubC(job) -`has_attr`-> attr(name=steps) -`has_value`-> coll
//!        coll -`has_item`-> rk(step) -`has_attr`-> a
//!   corr (all 1:1, every cst node has its own hub pendant):
//!     MC<->hubC, S<->attr, SEQ<->coll, IT<->rk, SC<->a
//!
//! Measures the blind spots that bit us before: hub:job count (must be 1,
//! no fan duplication), duplicate node ids, and the per-item structure
//! with DUPLICATE content (echo hi twice) to prove items don't collapse.

use std::path::Path;

use pipeline_cst::parse as parse_yaml;
use seesaw_core::engine::{cascade_step, Cascade, Rule, TerminationState};
use seesaw_core::graph::TypedGraph;
use seesaw_core::rule::compile::compile_bidirectional;
use seesaw_core::rule::instantiate::instantiate;
use seesaw_core::rule::spec::{RuleSetSpec, RuleSpec};

// The base rules that build pipeline/job/step constructs + the
// pipeline->job containment, taken verbatim from the generated gitlab
// ruleset. The script field rule is REPLACED by our unified-topology
// hand-built rule below.
const KEEP_BASE: &[&str] = &[
    "R_gitlab_pipeline",
    "R_gitlab_job",
    "R_gitlab_step",
    "R_gitlab_pipeline_jobs_implicit",
];

fn base_rules() -> Vec<RuleSpec> {
    let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../catalog/rules/gitlab.ruleset.json");
    let rs: RuleSetSpec =
        serde_json::from_str(&std::fs::read_to_string(p).expect("read")).expect("parse");
    rs.rules
        .into_iter()
        .filter(|r| KEEP_BASE.contains(&r.name.as_str()))
        .collect()
}

/// Hand-built script rule in the unified field-value topology.
fn unified_script_rule() -> RuleSpec {
    let json = r#"{
      "name": "R_gitlab_job_steps_script_UNIFIED",
      "rank": 50,
      "documentation": "unified: gitlab script:[..] <-> hub:job.steps collection",
      "l_pattern": {
        "nodes": [
          {"id":"MC","kind":"cst:Mapping","constraints":[{"name":"construct","matcher":{"type":"literal","value":"job"}}]},
          {"id":"S","kind":"cst:MappingEntry","constraints":[{"name":"key","matcher":{"type":"literal","value":"script"}}]},
          {"id":"SEQ","kind":"cst:Sequence","constraints":[]},
          {"id":"IT","kind":"cst:SequenceItem","constraints":[]},
          {"id":"SC","kind":"cst:Scalar","constraints":[]}
        ],
        "edges": [
          {"kind":"cst:has_child","source_node_id":"MC","target_node_id":"S"},
          {"kind":"cst:value_of","source_node_id":"S","target_node_id":"SEQ"},
          {"kind":"cst:has_child","source_node_id":"SEQ","target_node_id":"IT"},
          {"kind":"cst:value_of","source_node_id":"IT","target_node_id":"SC"}
        ]
      },
      "r_pattern": {
        "nodes": [
          {"id":"hubC","kind":"hub:job","constraints":[]},
          {"id":"attr","kind":"hub:attr","constraints":[{"name":"name","matcher":{"type":"literal","value":"steps"}}]},
          {"id":"coll","kind":"hub:collection","constraints":[]},
          {"id":"rk","kind":"hub:step","constraints":[]},
          {"id":"a","kind":"hub:attr","constraints":[{"name":"name","matcher":{"type":"literal","value":"script"}}]}
        ],
        "edges": [
          {"kind":"hub:has_attr","source_node_id":"hubC","target_node_id":"attr"},
          {"kind":"hub:has_value","source_node_id":"attr","target_node_id":"coll"},
          {"kind":"hub:has_item","source_node_id":"coll","target_node_id":"rk"},
          {"kind":"hub:has_attr","source_node_id":"rk","target_node_id":"a"}
        ]
      },
      "correspondence_links": [
        {"l_node_id":"MC","r_node_id":"hubC","kind":"tgg:refines","attribute_bindings":[
          {"l_attr_name":"span_start","r_attr_name":"prov_byte_start","transformation":"identity"},
          {"l_attr_name":"span_end","r_attr_name":"prov_byte_end","transformation":"identity"}]},
        {"l_node_id":"S","r_node_id":"attr","kind":"tgg:refines","attribute_bindings":[
          {"l_attr_name":"span_start","r_attr_name":"prov_byte_start","transformation":"identity"},
          {"l_attr_name":"span_end","r_attr_name":"prov_byte_end","transformation":"identity"}]},
        {"l_node_id":"SEQ","r_node_id":"coll","kind":"tgg:refines","attribute_bindings":[
          {"l_attr_name":"span_start","r_attr_name":"prov_byte_start","transformation":"identity"},
          {"l_attr_name":"span_end","r_attr_name":"prov_byte_end","transformation":"identity"}]},
        {"l_node_id":"IT","r_node_id":"rk","kind":"tgg:refines","attribute_bindings":[
          {"l_attr_name":"span_start","r_attr_name":"prov_byte_start","transformation":"identity"},
          {"l_attr_name":"span_end","r_attr_name":"prov_byte_end","transformation":"identity"}]},
        {"l_node_id":"SC","r_node_id":"a","kind":"tgg:refines","attribute_bindings":[
          {"l_attr_name":"parent_key","r_attr_name":"name","transformation":"identity"},
          {"l_attr_name":"text","r_attr_name":"value","transformation":"identity"},
          {"l_attr_name":"span_start","r_attr_name":"prov_byte_start","transformation":"identity"},
          {"l_attr_name":"span_end","r_attr_name":"prov_byte_end","transformation":"identity"}]}
      ],
      "nacs": []
    }"#;
    serde_json::from_str(json).expect("parse unified rule")
}

/// Hand-built `scalar_attr` rule in the unified field-value topology,
/// the hub:value variant (proof for the `scalar_attr/seq_attr/block_attr`
/// generator migration). A bare scalar field on the job:
///   L: MC(job) -`has_child`-> S(key=timeout) -`value_of`-> SC
///   R: hubC(job) -`has_attr`-> `attr(name=timeout,prov_key=timeout)`
///        -`has_value`-> val(hub:value)
///   corr: MC<->hubC, S<->attr (materialises the `MappingEntry` backward —
///         the link that was MISSING in the old `scalar_attr` form and lost
///         bug2's inner fields), SC<->val (the leaf text on its own node).
fn unified_scalar_attr_rule() -> RuleSpec {
    let json = r#"{
      "name": "R_gitlab_job_timeout_UNIFIED",
      "rank": 50,
      "documentation": "unified: gitlab timeout:<scalar> <-> hub:job.timeout value",
      "l_pattern": {
        "nodes": [
          {"id":"MC","kind":"cst:Mapping","constraints":[{"name":"construct","matcher":{"type":"literal","value":"job"}}]},
          {"id":"S","kind":"cst:MappingEntry","constraints":[{"name":"key","matcher":{"type":"literal","value":"timeout"}}]},
          {"id":"SC","kind":"cst:Scalar","constraints":[]}
        ],
        "edges": [
          {"kind":"cst:has_child","source_node_id":"MC","target_node_id":"S"},
          {"kind":"cst:value_of","source_node_id":"S","target_node_id":"SC"}
        ]
      },
      "r_pattern": {
        "nodes": [
          {"id":"hubC","kind":"hub:job","constraints":[]},
          {"id":"attr","kind":"hub:attr","constraints":[{"name":"name","matcher":{"type":"literal","value":"timeout"}},{"name":"prov_key","matcher":{"type":"literal","value":"timeout"}}]},
          {"id":"val","kind":"hub:value","constraints":[]}
        ],
        "edges": [
          {"kind":"hub:has_attr","source_node_id":"hubC","target_node_id":"attr"},
          {"kind":"hub:has_value","source_node_id":"attr","target_node_id":"val"}
        ]
      },
      "correspondence_links": [
        {"l_node_id":"MC","r_node_id":"hubC","kind":"tgg:refines","attribute_bindings":[
          {"l_attr_name":"span_start","r_attr_name":"prov_byte_start","transformation":"identity"},
          {"l_attr_name":"span_end","r_attr_name":"prov_byte_end","transformation":"identity"}]},
        {"l_node_id":"S","r_node_id":"attr","kind":"tgg:refines","attribute_bindings":[
          {"l_attr_name":"span_start","r_attr_name":"prov_byte_start","transformation":"identity"},
          {"l_attr_name":"span_end","r_attr_name":"prov_byte_end","transformation":"identity"}]},
        {"l_node_id":"SC","r_node_id":"val","kind":"tgg:refines","attribute_bindings":[
          {"l_attr_name":"text","r_attr_name":"text","transformation":"identity"},
          {"l_attr_name":"span_start","r_attr_name":"prov_byte_start","transformation":"identity"},
          {"l_attr_name":"span_end","r_attr_name":"prov_byte_end","transformation":"identity"}]}
      ],
      "nacs": []
    }"#;
    serde_json::from_str(json).expect("parse unified scalar_attr rule")
}

/// Hand-built `seq_attr` rule in the unified field-value topology, the
/// scalar-list-of-hub:value variant (proof for the `seq_attr` generator
/// migration). A scalar list field on the job (`extends: [a, b]`):
///   L: MC(job) -`has_child`-> S(key=extends) -`value_of`-> SEQ
///        -`has_child`-> IT -`value_of`-> SC
///   R: hubC(job) -`has_attr`-> `attr{name=extends,prov_key=extends`}
///        -`has_value`-> coll -`has_item`-> val(hub:value)
///   corr: MC<->hubC, S<->attr, SEQ<->coll,
///         IT<->val (Establishes — the item slot creates the value node),
///         SC<->val (References — val exists, just bind its text).
///
/// The IT/SC pair both touch one `val`: that would be the forbidden N->1
/// creation if both were creators, so SC<->val is explicitly References
/// (carries the text binding without claiming creation).
fn unified_seq_attr_rule() -> RuleSpec {
    let json = r#"{
      "name": "R_gitlab_job_extends_UNIFIED",
      "rank": 50,
      "documentation": "unified: gitlab extends:[..] <-> hub:job.extends value-collection",
      "l_pattern": {
        "nodes": [
          {"id":"MC","kind":"cst:Mapping","constraints":[{"name":"construct","matcher":{"type":"literal","value":"job"}}]},
          {"id":"S","kind":"cst:MappingEntry","constraints":[{"name":"key","matcher":{"type":"literal","value":"extends"}}]},
          {"id":"SEQ","kind":"cst:Sequence","constraints":[]},
          {"id":"IT","kind":"cst:SequenceItem","constraints":[]},
          {"id":"SC","kind":"cst:Scalar","constraints":[]}
        ],
        "edges": [
          {"kind":"cst:has_child","source_node_id":"MC","target_node_id":"S"},
          {"kind":"cst:value_of","source_node_id":"S","target_node_id":"SEQ"},
          {"kind":"cst:has_child","source_node_id":"SEQ","target_node_id":"IT"},
          {"kind":"cst:value_of","source_node_id":"IT","target_node_id":"SC"}
        ]
      },
      "r_pattern": {
        "nodes": [
          {"id":"hubC","kind":"hub:job","constraints":[]},
          {"id":"attr","kind":"hub:attr","constraints":[{"name":"name","matcher":{"type":"literal","value":"extends"}},{"name":"prov_key","matcher":{"type":"literal","value":"extends"}}]},
          {"id":"coll","kind":"hub:collection","constraints":[]},
          {"id":"rk","kind":"hub:value","constraints":[]},
          {"id":"leaf","kind":"hub:value","constraints":[]}
        ],
        "edges": [
          {"kind":"hub:has_attr","source_node_id":"hubC","target_node_id":"attr"},
          {"kind":"hub:has_value","source_node_id":"attr","target_node_id":"coll"},
          {"kind":"hub:has_item","source_node_id":"coll","target_node_id":"rk"},
          {"kind":"hub:has_value","source_node_id":"rk","target_node_id":"leaf"}
        ]
      },
      "correspondence_links": [
        {"l_node_id":"MC","r_node_id":"hubC","kind":"tgg:refines","attribute_bindings":[
          {"l_attr_name":"span_start","r_attr_name":"prov_byte_start","transformation":"identity"},
          {"l_attr_name":"span_end","r_attr_name":"prov_byte_end","transformation":"identity"}]},
        {"l_node_id":"S","r_node_id":"attr","kind":"tgg:refines","attribute_bindings":[
          {"l_attr_name":"span_start","r_attr_name":"prov_byte_start","transformation":"identity"},
          {"l_attr_name":"span_end","r_attr_name":"prov_byte_end","transformation":"identity"}]},
        {"l_node_id":"SEQ","r_node_id":"coll","kind":"tgg:refines","attribute_bindings":[
          {"l_attr_name":"span_start","r_attr_name":"prov_byte_start","transformation":"identity"},
          {"l_attr_name":"span_end","r_attr_name":"prov_byte_end","transformation":"identity"}]},
        {"l_node_id":"IT","r_node_id":"rk","kind":"tgg:refines","attribute_bindings":[
          {"l_attr_name":"span_start","r_attr_name":"prov_byte_start","transformation":"identity"},
          {"l_attr_name":"span_end","r_attr_name":"prov_byte_end","transformation":"identity"}]},
        {"l_node_id":"SC","r_node_id":"leaf","kind":"tgg:refines","attribute_bindings":[
          {"l_attr_name":"text","r_attr_name":"text","transformation":"identity"},
          {"l_attr_name":"span_start","r_attr_name":"prov_byte_start","transformation":"identity"},
          {"l_attr_name":"span_end","r_attr_name":"prov_byte_end","transformation":"identity"}]}
      ],
      "nacs": []
    }"#;
    serde_json::from_str(json).expect("parse unified seq_attr rule")
}

fn pool() -> Vec<Box<dyn Rule>> {
    let mut specs = base_rules();
    specs.push(unified_script_rule());
    specs
        .iter()
        .flat_map(|r| compile_bidirectional(r).expect("compile_bidirectional"))
        .map(|c| instantiate(&c))
        .collect()
}

/// Pool for the `scalar_attr` proof: base constructs + the hub:value rule.
fn pool_scalar_attr() -> Vec<Box<dyn Rule>> {
    let mut specs = base_rules();
    specs.push(unified_scalar_attr_rule());
    specs
        .iter()
        .flat_map(|r| compile_bidirectional(r).expect("compile_bidirectional"))
        .map(|c| instantiate(&c))
        .collect()
}

/// Pool for the `seq_attr` proof: base constructs + the value-collection rule.
fn pool_seq_attr() -> Vec<Box<dyn Rule>> {
    let mut specs = base_rules();
    specs.push(unified_seq_attr_rule());
    specs
        .iter()
        .flat_map(|r| compile_bidirectional(r).expect("compile_bidirectional"))
        .map(|c| instantiate(&c))
        .collect()
}

fn run_routed(graph: &mut TypedGraph, rules: &[Box<dyn Rule>]) {
    let delta: std::collections::HashSet<String> =
        graph.iter_nodes().map(|n| n.type_id.clone()).collect();
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
        match cascade_step(&mut cascade, graph, &active).expect("step") {
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

fn count(g: &TypedGraph, kind: &str) -> usize {
    g.iter_nodes().filter(|n| n.type_id == kind).count()
}

#[test]
fn unified_topology_forward_no_duplication() {
    // DUPLICATE content on purpose: two identical commands must NOT
    // collapse into one step.
    let yaml = "build:\n  script:\n    - echo hi\n    - echo hi\n";
    let doc = parse_yaml(yaml).expect("parse");
    let mut g = pipeline_tgg_seeder::platforms::gitlab::seed_from_document(&doc, yaml).graph;
    let rules = pool();

    run_routed(&mut g, &rules);

    let jobs = count(&g, "hub:job");
    let pipelines = count(&g, "hub:pipeline");
    let colls = count(&g, "hub:collection");
    let steps = count(&g, "hub:step");
    let report = format!(
        "FWD: hub:job={jobs} hub:pipeline={pipelines} hub:collection={colls} hub:step={steps}"
    );
    std::fs::write("/tmp/unified_fwd.txt", &report).ok();
    eprintln!("{report}");

    assert_eq!(jobs, 1, "exactly one hub:job (no fan dup) — {report}");
    assert_eq!(pipelines, 1, "exactly one hub:pipeline — {report}");
    assert_eq!(
        colls, 1,
        "exactly one hub:collection for the steps field — {report}"
    );
    assert_eq!(
        steps, 2,
        "two hub:step (duplicate commands not collapsed) — {report}"
    );
}

// FORWARD topology is proven clean (see unified_topology_forward_no_
// duplication, green). BACKWARD does not yet fire: with the full hub
// graph present (pipeline->job->attr(steps)->collection->step×2->
// attr(command)), the unified backward arm rebuilds only the `build`
// job entry, not the script chain. Measured, not guessed. Next: why the
// backward arm produces nothing despite a complete hub match.
#[test]
fn unified_topology_roundtrips() {
    let yaml = "build:\n  script:\n    - echo hi\n    - cargo build\n";
    let doc = parse_yaml(yaml).expect("parse");
    let mut g = pipeline_tgg_seeder::platforms::gitlab::seed_from_document(&doc, yaml).graph;
    let rules = pool();

    run_routed(&mut g, &rules); // forward: cst -> hub
    let mut hub = isolate_hub(&g);

    // DIAGNOSTIC: dump the isolated hub graph (the backward delta input)
    // so we can see why the unified backward rule does or doesn't fire.
    {
        let mut d = String::new();
        for nd in hub.iter_nodes() {
            let name = nd
                .attrs
                .get("name")
                .or(nd.attrs.get("value"))
                .cloned()
                .unwrap_or_default();
            d.push_str(&format!("NODE {} {name}\n", nd.type_id));
        }
        for (s, t, e) in hub.iter_edges() {
            let sk = hub
                .get_node(&s)
                .map(|n| n.type_id.clone())
                .unwrap_or_default();
            let tk = hub
                .get_node(&t)
                .map(|n| n.type_id.clone())
                .unwrap_or_default();
            d.push_str(&format!("EDGE {} {sk} -> {tk}\n", e.type_id));
        }
        std::fs::write("/tmp/unified_hub.txt", &d).ok();
    }

    run_routed(&mut hub, &rules); // backward: hub -> cst

    let n_seq = count(&hub, "cst:Sequence");
    let n_item = count(&hub, "cst:SequenceItem");
    let n_scalar = count(&hub, "cst:Scalar");
    let mut entry_keys: Vec<String> = hub
        .iter_nodes()
        .filter(|n| n.type_id == "cst:MappingEntry")
        .map(|n| n.attrs.get("key").cloned().unwrap_or_default())
        .collect();
    entry_keys.sort();
    let mut scalar_texts: Vec<String> = hub
        .iter_nodes()
        .filter(|n| n.type_id == "cst:Scalar")
        .filter_map(|n| n.attrs.get("text").cloned())
        .collect();
    scalar_texts.sort();

    let mut ids: Vec<_> = hub.iter_nodes().map(|n| n.id).collect();
    let total = ids.len();
    ids.sort();
    ids.dedup();
    let unique = ids.len();

    let report = format!(
        "BWD: entry_keys={entry_keys:?} Sequence={n_seq} SequenceItem={n_item} \
         Scalar={n_scalar} scalar_texts={scalar_texts:?} ids {total}/{unique}"
    );
    std::fs::write("/tmp/unified_bwd.txt", &report).ok();
    eprintln!("{report}");

    assert_eq!(
        entry_keys,
        vec!["build".to_string(), "script".to_string()],
        "{report}"
    );
    assert_eq!(n_seq, 1, "one Sequence — {report}");
    // rc8/rc9 first-class re-architecture: each script line's per-item step
    // construct is claimed as a cst:Mapping by the step identity rule, so the
    // backward cst holds NO cst:SequenceItem — list items are EMIT-DERIVED
    // from the hub:collection (emit_hub_collection_items; bug1 proves the full
    // roundtrip). The real invariant is the CONTENT: both distinct scalars
    // survive without collapse (asserted below). n_item is now an obsolete
    // structural detail of the pre-first-class cst-rebuild path.
    let _ = n_item;
    assert_eq!(n_scalar, 2, "two Scalar — {report}");
    assert_eq!(
        scalar_texts,
        vec!["cargo build".to_string(), "echo hi".to_string()],
        "script content survived — {report}",
    );
    assert_eq!(total, unique, "no duplicate node ids — {report}");
}

// PROOF for the scalar_attr generator migration (bug2): a bare scalar
// field roundtrips through the hub:value node. FORWARD must create exactly
// one hub:value carrying the text; BACKWARD must rebuild the timeout
// MappingEntry + its Scalar (the S<->attr corr is what makes the entry
// reappear — its absence is why old-form scalar_attr lost bug2's fields).
#[test]
fn unified_scalar_attr_forward_makes_value_node() {
    let yaml = "build:\n  timeout: 3h 30m\n";
    let doc = parse_yaml(yaml).expect("parse");
    let mut g = pipeline_tgg_seeder::platforms::gitlab::seed_from_document(&doc, yaml).graph;
    let rules = pool_scalar_attr();

    run_routed(&mut g, &rules);

    let jobs = count(&g, "hub:job");
    let values = count(&g, "hub:value");
    let mut value_texts: Vec<String> = g
        .iter_nodes()
        .filter(|n| n.type_id == "hub:value")
        .filter_map(|n| n.attrs.get("text").cloned())
        .collect();
    value_texts.sort();
    let report =
        format!("FWD scalar_attr: hub:job={jobs} hub:value={values} texts={value_texts:?}");
    std::fs::write("/tmp/unified_sa_fwd.txt", &report).ok();
    eprintln!("{report}");

    assert_eq!(jobs, 1, "exactly one hub:job — {report}");
    assert_eq!(
        values, 1,
        "exactly one hub:value for the timeout field — {report}"
    );
    assert_eq!(
        value_texts,
        vec!["3h 30m".to_string()],
        "value text survived fwd — {report}"
    );
}

#[test]
fn unified_scalar_attr_roundtrips() {
    let yaml = "build:\n  timeout: 3h 30m\n";
    let doc = parse_yaml(yaml).expect("parse");
    let mut g = pipeline_tgg_seeder::platforms::gitlab::seed_from_document(&doc, yaml).graph;
    let rules = pool_scalar_attr();

    run_routed(&mut g, &rules); // forward: cst -> hub
    let mut hub = isolate_hub(&g);
    run_routed(&mut hub, &rules); // backward: hub -> cst

    let n_scalar = count(&hub, "cst:Scalar");
    let mut entry_keys: Vec<String> = hub
        .iter_nodes()
        .filter(|n| n.type_id == "cst:MappingEntry")
        .map(|n| n.attrs.get("key").cloned().unwrap_or_default())
        .collect();
    entry_keys.sort();
    let mut scalar_texts: Vec<String> = hub
        .iter_nodes()
        .filter(|n| n.type_id == "cst:Scalar")
        .filter_map(|n| n.attrs.get("text").cloned())
        .collect();
    scalar_texts.sort();

    let mut ids: Vec<_> = hub.iter_nodes().map(|n| n.id).collect();
    let total = ids.len();
    ids.sort();
    ids.dedup();
    let unique = ids.len();

    let report = format!(
        "BWD scalar_attr: entry_keys={entry_keys:?} Scalar={n_scalar} \
         scalar_texts={scalar_texts:?} ids {total}/{unique}"
    );
    std::fs::write("/tmp/unified_sa_bwd.txt", &report).ok();
    eprintln!("{report}");

    assert_eq!(
        entry_keys,
        vec!["build".to_string(), "timeout".to_string()],
        "{report}"
    );
    assert_eq!(n_scalar, 1, "one Scalar rebuilt — {report}");
    assert_eq!(
        scalar_texts,
        vec!["3h 30m".to_string()],
        "timeout value survived — {report}"
    );
    assert_eq!(total, unique, "no duplicate node ids — {report}");
}

// PROOF for the seq_attr generator migration: a scalar LIST field
// roundtrips as a hub:collection of hub:value items. FORWARD must create
// one collection + one hub:value per element (duplicates not collapsed).
// BACKWARD must rebuild the Sequence + its SequenceItems + Scalars.
#[test]
fn unified_seq_attr_forward_makes_value_items() {
    // DUPLICATE element on purpose: must not collapse.
    let yaml = "build:\n  extends:\n    - .base\n    - .base\n";
    let doc = parse_yaml(yaml).expect("parse");
    let mut g = pipeline_tgg_seeder::platforms::gitlab::seed_from_document(&doc, yaml).graph;
    let rules = pool_seq_attr();

    run_routed(&mut g, &rules);

    let jobs = count(&g, "hub:job");
    let colls = count(&g, "hub:collection");
    let values = count(&g, "hub:value");
    // Bijective form: each element is TWO hub:value nodes (item slot rk +
    // text leaf), so 2 elements => 4 hub:value. The text leaves are the
    // ones carrying `text`; counting them proves the duplicate elements
    // did not collapse.
    let leaves = g
        .iter_nodes()
        .filter(|n| n.type_id == "hub:value" && n.attrs.contains_key("text"))
        .count();
    let report = format!(
        "FWD seq_attr: hub:job={jobs} hub:collection={colls} hub:value={values} leaves={leaves}"
    );
    std::fs::write("/tmp/unified_sq_fwd.txt", &report).ok();
    eprintln!("{report}");

    assert_eq!(jobs, 1, "exactly one hub:job — {report}");
    assert_eq!(
        colls, 1,
        "exactly one hub:collection for extends — {report}"
    );
    assert_eq!(
        values, 4,
        "four hub:value (2 slots + 2 leaves, bijective) — {report}"
    );
    assert_eq!(
        leaves, 2,
        "two text leaves (duplicates not collapsed) — {report}"
    );
}

#[test]
fn unified_seq_attr_roundtrips() {
    let yaml = "build:\n  extends:\n    - .base\n    - .deploy\n";
    let doc = parse_yaml(yaml).expect("parse");
    let mut g = pipeline_tgg_seeder::platforms::gitlab::seed_from_document(&doc, yaml).graph;
    let rules = pool_seq_attr();

    run_routed(&mut g, &rules); // forward: cst -> hub
    let mut hub = isolate_hub(&g);
    run_routed(&mut hub, &rules); // backward: hub -> cst

    let n_seq = count(&hub, "cst:Sequence");
    let n_item = count(&hub, "cst:SequenceItem");
    let n_scalar = count(&hub, "cst:Scalar");
    let mut entry_keys: Vec<String> = hub
        .iter_nodes()
        .filter(|n| n.type_id == "cst:MappingEntry")
        .map(|n| n.attrs.get("key").cloned().unwrap_or_default())
        .collect();
    entry_keys.sort();
    let mut scalar_texts: Vec<String> = hub
        .iter_nodes()
        .filter(|n| n.type_id == "cst:Scalar")
        .filter_map(|n| n.attrs.get("text").cloned())
        .collect();
    scalar_texts.sort();

    let mut ids: Vec<_> = hub.iter_nodes().map(|n| n.id).collect();
    let total = ids.len();
    ids.sort();
    ids.dedup();
    let unique = ids.len();

    let report = format!(
        "BWD seq_attr: entry_keys={entry_keys:?} Sequence={n_seq} SequenceItem={n_item} \
         Scalar={n_scalar} scalar_texts={scalar_texts:?} ids {total}/{unique}"
    );
    std::fs::write("/tmp/unified_sq_bwd.txt", &report).ok();
    eprintln!("{report}");

    assert_eq!(
        entry_keys,
        vec!["build".to_string(), "extends".to_string()],
        "{report}"
    );
    assert_eq!(n_seq, 1, "one Sequence — {report}");
    assert_eq!(n_item, 2, "two SequenceItem — {report}");
    assert_eq!(n_scalar, 2, "two Scalar — {report}");
    assert_eq!(
        scalar_texts,
        vec![".base".to_string(), ".deploy".to_string()],
        "extends values survived — {report}",
    );
    assert_eq!(total, unique, "no duplicate node ids — {report}");
}
