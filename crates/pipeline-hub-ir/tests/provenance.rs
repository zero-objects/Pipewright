//! Tests for `Provenance::from_byte_span` line/col computation
//! and serde round-trip.

use pipeline_hub_ir::Provenance;
use pretty_assertions::assert_eq;

#[test]
fn first_byte_is_line_1_col_1() {
    let src = "abc\ndef\n";
    let p = Provenance::from_byte_span("f.yml", src, (0, 3));
    assert_eq!(p.range.line_start, 1);
    assert_eq!(p.range.col_start, 1);
    assert_eq!(p.range.line_end, 1);
    assert_eq!(p.range.col_end, 4);
}

#[test]
fn span_on_second_line_computes_line_2() {
    let src = "abc\ndef\nghij\n";
    let p = Provenance::from_byte_span("f.yml", src, (4, 7));
    assert_eq!(p.range.line_start, 2);
    assert_eq!(p.range.col_start, 1);
    assert_eq!(p.range.line_end, 2);
    assert_eq!(p.range.col_end, 4);
}

#[test]
fn span_across_lines() {
    let src = "abc\ndef\nghij\n";
    let p = Provenance::from_byte_span("f.yml", src, (2, 9));
    assert_eq!(p.range.line_start, 1);
    assert_eq!(p.range.col_start, 3);
    assert_eq!(p.range.line_end, 3);
    assert_eq!(p.range.col_end, 2);
}

#[test]
fn defined_by_reference_is_none_in_m3() {
    let src = "x";
    let p = Provenance::from_byte_span("f.yml", src, (0, 1));
    assert!(p.defined_by_reference.is_none());
}

#[test]
fn provenance_serde_round_trip_json() {
    let src = "a\nb\n";
    let p = Provenance::from_byte_span("f.yml", src, (2, 3));
    let json = serde_json::to_string(&p).expect("ser");
    let round: Provenance = serde_json::from_str(&json).expect("de");
    assert_eq!(p, round);
}

#[test]
fn serde_omits_none_defined_by_reference() {
    let src = "x";
    let p = Provenance::from_byte_span("f.yml", src, (0, 1));
    let json = serde_json::to_string(&p).expect("ser");
    assert!(!json.contains("defined_by_reference"));
}

#[test]
fn unicode_chars_count_as_one_column_each() {
    // "äb" has bytes 0,1,2,3 ('ä' is 2 bytes). After 'ä' col should be 2.
    let src = "äb\nc";
    let p = Provenance::from_byte_span("f.yml", src, (2, 3));
    assert_eq!(p.range.line_start, 1);
    assert_eq!(p.range.col_start, 2);
}

#[test]
fn byte_past_end_clamps_to_final_position() {
    let src = "a\nb";
    let p = Provenance::from_byte_span("f.yml", src, (0, 1000));
    assert_eq!(p.range.line_end, 2);
    assert_eq!(p.range.col_end, 2);
}
