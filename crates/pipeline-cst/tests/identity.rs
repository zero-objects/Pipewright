use pipeline_cst::{parse, serialize};

#[test]
fn identity_empty() {
    assert_eq!(serialize(&parse("").unwrap()), "");
}

#[test]
fn identity_single_line() {
    let s = "build:\n  script:\n    - echo hi\n";
    assert_eq!(serialize(&parse(s).unwrap()), s);
}

#[test]
fn identity_with_comments() {
    let s = "# top\nbuild:  # trail\n  script:\n    - x\n";
    assert_eq!(serialize(&parse(s).unwrap()), s);
}
