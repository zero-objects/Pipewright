//! Migration friction is derived, not declared: a lossy migration must report
//! exactly the capability families the target dropped.
#[test]
fn gitlab_to_drone_reports_dropped_cache_and_services() {
    let src = "build:\n  image: rust:1.75\n  cache:\n    paths: [target]\n  services:\n    - postgres:16\n  script:\n    - cargo test\n";
    let (yaml, report) = pipeline_forward::migrate_with_report("gitlab", src, "drone").unwrap();
    assert!(yaml.contains("cargo test"), "still migrates: {yaml}");
    let feats: Vec<&str> = report.iter().map(|f| f.feature.as_str()).collect();
    // cache and services are non-universal and drone's flat steps drop them.
    assert!(feats.contains(&"cache"), "cache loss reported: {report:?}");
    assert!(
        feats.contains(&"service"),
        "service loss reported: {report:?}"
    );
    // every reported loss carries a severity + a human note.
    for f in &report {
        assert!(matches!(f.severity, "info" | "approximated" | "manual"));
        assert!(!f.note.is_empty());
    }
}

#[test]
fn lossless_migration_reports_nothing() {
    // gitlab → github (same family, both job-based) keeps image/script/needs.
    let src = "build:\n  image: rust:1.75\n  script:\n    - cargo build\n";
    let (_, report) = pipeline_forward::migrate_with_report("gitlab", src, "github").unwrap();
    assert!(
        report.is_empty(),
        "no friction for a clean migration: {report:?}"
    );
}
