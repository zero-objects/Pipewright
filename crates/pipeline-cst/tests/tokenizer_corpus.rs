//! Tokenizer must parse all corpus fixtures without errors.

use pipeline_cst::tokenize;
use std::fs;
use std::path::PathBuf;

fn spike_fixture(name: &str) -> String {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.push("tests/corpus/fixtures");
    p.push(name);
    fs::read_to_string(&p).unwrap_or_else(|e| panic!("read {}: {}", p.display(), e))
}

fn spike_real(name: &str) -> String {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.push("tests/corpus/real");
    p.push(name);
    fs::read_to_string(&p).unwrap_or_else(|e| panic!("read {}: {}", p.display(), e))
}

#[test]
fn tokenize_fixture_01() {
    tokenize(&spike_fixture("01_single_job.yml")).expect("01");
}

#[test]
fn tokenize_fixture_02() {
    tokenize(&spike_fixture("02_two_jobs_no_needs.yml")).expect("02");
}

#[test]
fn tokenize_fixture_03() {
    tokenize(&spike_fixture("03_with_comments.yml")).expect("03");
}

#[test]
fn tokenize_fixture_04() {
    tokenize(&spike_fixture("04_with_anchors.yml")).expect("04");
}

#[test]
fn tokenize_fixture_05() {
    tokenize(&spike_fixture("05_complex_indentation.yml")).expect("05");
}

#[test]
fn tokenize_real_01_rust_workspace() {
    tokenize(&spike_real("01_rust_workspace.yml")).expect("rust workspace");
}

#[test]
fn tokenize_real_02_godot_game() {
    tokenize(&spike_real("02_godot_game.yml")).expect("godot game");
}

#[test]
fn token_spans_within_source() {
    for fixture in [
        "01_single_job.yml",
        "02_two_jobs_no_needs.yml",
        "03_with_comments.yml",
        "04_with_anchors.yml",
        "05_complex_indentation.yml",
    ] {
        let source = spike_fixture(fixture);
        let toks = tokenize(&source).expect(fixture);
        for t in &toks {
            let span_opt = match t {
                pipeline_cst::Token::Scalar { span, .. }
                | pipeline_cst::Token::Comment { span, .. } => Some(*span),
                pipeline_cst::Token::Anchor { name_span }
                | pipeline_cst::Token::Alias { name_span }
                | pipeline_cst::Token::Tag { name_span } => Some(*name_span),
                pipeline_cst::Token::MappingKey { key_span } => Some(*key_span),
                _ => None,
            };
            if let Some(span) = span_opt {
                assert!(span.end <= source.len(), "{fixture}: span oversteps");
                assert!(span.start <= span.end, "{fixture}: span inverted");
            }
        }
    }
}
