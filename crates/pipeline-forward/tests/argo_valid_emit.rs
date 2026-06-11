//! F4: argo re-emit must produce VALID argo — `tasks:` under a `dag:` wrapper
//! and `parameters:` under `inputs:`, not bare under the template (which argo
//! rejects). The seeder hoists both wrappers on the way in, so `re_emit` re-nests
//! them on the way out.

#[test]
fn argo_emit_nests_tasks_under_dag_and_params_under_inputs() {
    let src = std::fs::read_to_string("../../tests/edge_cases/argo/dag-diamond.yaml").unwrap();
    let g = pipeline_forward::forward("argo", &src).unwrap();
    let out = pipeline_forward::re_emit("argo", &g).unwrap();

    // The wrappers are present...
    assert!(out.contains("dag:"), "tasks must be wrapped in dag:\n{out}");
    assert!(
        out.contains("inputs:"),
        "parameters must be wrapped in inputs:\n{out}"
    );

    // ...and `tasks:` / `parameters:` are NOT emitted bare (every occurrence is
    // indented under its wrapper, i.e. preceded by more whitespace than a
    // top-of-template key).
    for line in out.lines() {
        let t = line.trim_start();
        if t.starts_with("tasks:") || t.starts_with("parameters:") {
            let indent = line.len() - t.len();
            assert!(
                indent >= 6,
                "`{t}` should be nested under its wrapper, got indent {indent}:\n{out}"
            );
        }
    }

    // And it still round-trips: forward(emit) == the same hub (valid argo).
    let g2 = pipeline_forward::forward("argo", &out).unwrap();
    let jobs1 = g.iter_nodes().filter(|n| n.type_id == "hub:job").count();
    let jobs2 = g2.iter_nodes().filter(|n| n.type_id == "hub:job").count();
    assert_eq!(jobs1, jobs2, "round-trip preserves the job count");
}
