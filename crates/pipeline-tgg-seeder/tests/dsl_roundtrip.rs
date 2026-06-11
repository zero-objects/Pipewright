//! DSL roundtrip for the non-YAML platforms (Point 2): jenkins (Groovy)
//! and earthly (Earthfile). The chaos generator only emits YAML, so these
//! can't ride the chaos roundtrip — instead a hand-written DSL fixture goes
//!   DSL -> parse -> seed -> FORWARD -> hub1
//!       -> BACKWARD -> cst -> emit_<dsl> -> DSL'
//!       -> parse -> seed -> FORWARD -> hub2
//! and we assert hub1 ≅ hub2 at the content level. This exercises the
//! dedicated `emit_jenkinsfile` / `emit_earthfile` emitters (which existed but
//! had no end-to-end test) and proves the non-YAML platforms round-trip in
//! their OWN surface syntax, not just forward into the hub.

use pipeline_earthfile_cst::parse as parse_earthfile;
use pipeline_jenkinsfile_cst::parse as parse_jenkinsfile;
use pipeline_tgg_seeder::emit_earthfile::emit_earthfile;
use pipeline_tgg_seeder::emit_jenkinsfile::emit_jenkinsfile;
use pipeline_tgg_seeder::platforms;
use seesaw_core::engine::{cascade_step, Cascade, Rule, TerminationState};
use seesaw_core::graph::{GhostId, TypedGraph};
use seesaw_core::rule::compile::compile_bidirectional;
use seesaw_core::rule::instantiate::instantiate;
use seesaw_core::rule::spec::RuleSetSpec;
use std::collections::BTreeMap;
use std::path::Path;

fn pool(platform: &str) -> Vec<Box<dyn Rule>> {
    let p = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../catalog/rules")
        .join(format!("{platform}.ruleset.json"));
    let rs: RuleSetSpec = serde_json::from_str(&std::fs::read_to_string(&p).unwrap()).unwrap();
    rs.rules
        .iter()
        .flat_map(|r| compile_bidirectional(r).expect("compile_bidirectional"))
        .map(|c| instantiate(&c))
        .collect()
}

fn run_routed(graph: &mut TypedGraph, rules: &[Box<dyn Rule>]) -> Result<(), String> {
    let delta: std::collections::HashSet<String> =
        graph.iter_nodes().map(|n| n.type_id.clone()).collect();
    let active: Vec<&dyn Rule> = rules
        .iter()
        .filter(|r| {
            let k = r.input_domain_kinds();
            k.is_empty() || k.iter().any(|x| delta.contains(x))
        })
        .map(AsRef::as_ref)
        .collect();
    let mut c = Cascade::new();
    for _ in 0..20_000 {
        match cascade_step(&mut c, graph, &active).map_err(|e| format!("{e:?}"))? {
            TerminationState::Running => continue,
            _ => return Ok(()),
        }
    }
    Err("cascade did not converge".into())
}

fn isolate_hub(g: &TypedGraph) -> TypedGraph {
    let mut hub = TypedGraph::new();
    for n in g.iter_nodes() {
        if n.type_id.starts_with("hub:") {
            hub.insert_node_data(n.clone());
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

fn is_prov(k: &str) -> bool {
    k.starts_with("prov") || k.starts_with("span")
}

fn hub_nodes(hub: &TypedGraph) -> BTreeMap<String, usize> {
    let mut nodes: BTreeMap<String, usize> = BTreeMap::new();
    for n in hub.iter_nodes() {
        let mut attrs: Vec<String> = n
            .attrs
            .iter()
            .filter(|(k, v)| !is_prov(k) && !v.is_empty())
            .map(|(k, v)| format!("{k}={v}"))
            .collect();
        attrs.sort();
        *nodes
            .entry(format!("{}#{}", n.type_id, attrs.join(",")))
            .or_default() += 1;
    }
    nodes
}

enum Dsl {
    Jenkins,
    Earthly,
}

fn dsl_roundtrip(platform: &str, dsl: Dsl, src: &str) {
    let rules = pool(platform);
    let parse = |s: &str| match dsl {
        Dsl::Jenkins => parse_jenkinsfile(s).expect("parse jenkinsfile"),
        Dsl::Earthly => parse_earthfile(s).expect("parse earthfile"),
    };
    let emit = |g: &TypedGraph, root: GhostId| match dsl {
        Dsl::Jenkins => emit_jenkinsfile(g, root),
        Dsl::Earthly => emit_earthfile(g, root),
    };
    let seed = |doc: &pipeline_cst::Document, s: &str| match platform {
        "jenkins" => platforms::jenkins::seed_from_document(doc, s).graph,
        "earthly" => platforms::earthly::seed_from_document(doc, s).graph,
        _ => unreachable!(),
    };

    let doc = parse(src);
    let mut g = seed(&doc, src);
    run_routed(&mut g, &rules).expect("fwd");
    let mut hub = isolate_hub(&g);
    let sig1 = hub_nodes(&hub);

    run_routed(&mut hub, &rules).expect("bwd");
    let root = pick_pipeline_root(&hub).expect("reconstructed pipeline root");
    let dsl_out = emit(&hub, root);
    assert!(!dsl_out.trim().is_empty(), "emit produced empty DSL");

    let doc2 = parse(&dsl_out);
    let mut g2 = seed(&doc2, &dsl_out);
    run_routed(&mut g2, &rules).expect("fwd2");
    let sig2 = hub_nodes(&isolate_hub(&g2));

    assert_eq!(
        sig1, sig2,
        "{platform} DSL roundtrip lost content.\n--- emitted {platform} ---\n{dsl_out}\n--- hub1 ---\n{sig1:#?}\n--- hub2 ---\n{sig2:#?}"
    );
}

#[test]
fn jenkins_dsl_roundtrips() {
    dsl_roundtrip(
        "jenkins",
        Dsl::Jenkins,
        "pipeline {\n    agent any\n    stages {\n        stage('Build') {\n            steps {\n                sh 'cargo build'\n            }\n        }\n    }\n}\n",
    );
}

#[test]
fn earthly_dsl_roundtrips() {
    dsl_roundtrip(
        "earthly",
        Dsl::Earthly,
        "VERSION 0.8\nFROM rust:1.75\n\nbuild:\n    COPY . .\n    RUN cargo build\n    SAVE ARTIFACT target/release/binary\n",
    );
}
