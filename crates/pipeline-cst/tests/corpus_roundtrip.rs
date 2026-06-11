//! M2 `DoD`: byte-identical round-trip on full corpus, 0 failures.

use pipeline_cst::{parse, serialize};
use std::fs;
use std::path::{Path, PathBuf};

fn corpus_dir() -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.push("tests/corpus");
    p
}

fn collect_yaml_files(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Ok(entries) = fs::read_dir(dir) {
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() {
                out.extend(collect_yaml_files(&p));
            } else if p.extension().and_then(|s| s.to_str()) == Some("yml") {
                out.push(p);
            }
        }
    }
    out
}

#[test]
fn byte_identical_round_trip_on_full_corpus() {
    let dir = corpus_dir();
    let files = collect_yaml_files(&dir);
    assert!(
        files.len() >= 30,
        "expected at least 30 corpus files, got {}",
        files.len()
    );

    let mut failures = Vec::new();
    for path in &files {
        let source = match fs::read_to_string(path) {
            Ok(s) => s,
            Err(e) => {
                failures.push((path.clone(), format!("read: {e}")));
                continue;
            }
        };
        let doc = match parse(&source) {
            Ok(d) => d,
            Err(e) => {
                failures.push((path.clone(), format!("parse: {e}")));
                continue;
            }
        };
        let emitted = serialize(&doc);
        if emitted != source {
            // Compute a short diff hint.
            let first_diff = source
                .bytes()
                .zip(emitted.bytes())
                .position(|(a, b)| a != b)
                .unwrap_or(source.len().min(emitted.len()));
            failures.push((
                path.clone(),
                format!(
                    "byte mismatch at offset {first_diff} (lens {} vs {})",
                    source.len(),
                    emitted.len()
                ),
            ));
        }
    }
    if !failures.is_empty() {
        for (p, why) in &failures {
            eprintln!("FAIL  {}: {}", p.display(), why);
        }
        panic!(
            "{} of {} corpus files failed round-trip",
            failures.len(),
            files.len()
        );
    }
    eprintln!(
        "OK    {} corpus files round-trip byte-identically",
        files.len()
    );
}
