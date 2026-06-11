use pipeline_cst::{tokenize, ScalarStyle, Token};

fn kinds(toks: &[Token]) -> Vec<&'static str> {
    toks.iter()
        .map(|t| match t {
            Token::Indent(_) => "Indent",
            Token::MappingKey { .. } => "MappingKey",
            Token::SequenceDash => "SequenceDash",
            Token::Scalar { .. } => "Scalar",
            Token::Anchor { .. } => "Anchor",
            Token::Alias { .. } => "Alias",
            Token::Tag { .. } => "Tag",
            Token::Comment { .. } => "Comment",
            Token::Newline => "Newline",
            Token::Eof => "Eof",
        })
        .collect()
}

#[test]
fn tokenize_empty() {
    let toks = tokenize("").unwrap();
    assert_eq!(kinds(&toks), vec!["Eof"]);
}

#[test]
fn tokenize_simple_mapping() {
    let toks = tokenize("key: value\n").unwrap();
    assert_eq!(
        kinds(&toks),
        vec!["Indent", "MappingKey", "Scalar", "Newline", "Eof"]
    );
}

#[test]
fn tokenize_comment_line() {
    let toks = tokenize("# hello\n").unwrap();
    assert_eq!(kinds(&toks), vec!["Indent", "Comment", "Newline", "Eof"]);
}

#[test]
fn tokenize_sequence_item() {
    let toks = tokenize("  - hello\n").unwrap();
    assert_eq!(
        kinds(&toks),
        vec!["Indent", "SequenceDash", "Scalar", "Newline", "Eof"]
    );
}

#[test]
fn tokenize_anchor_alias() {
    let toks = tokenize(".x: &x value\nuse: *x\n").unwrap();
    let k = kinds(&toks);
    assert!(k.contains(&"Anchor"));
    assert!(k.contains(&"Alias"));
}

#[test]
fn tokenize_block_literal() {
    let toks = tokenize("script: |\n  echo hi\n  echo bye\n").unwrap();
    let scalar = toks
        .iter()
        .find_map(|t| match t {
            Token::Scalar { style, .. } => Some(*style),
            _ => None,
        })
        .expect("scalar present");
    assert_eq!(scalar, ScalarStyle::Literal);
}

#[test]
fn tokenize_flow_list_kept_as_opaque() {
    let toks = tokenize("xs: [a, b, c]\n").unwrap();
    let scalar = toks
        .iter()
        .find_map(|t| match t {
            Token::Scalar { style, .. } => Some(*style),
            _ => None,
        })
        .expect("scalar present");
    assert_eq!(scalar, ScalarStyle::FlowList);
}

#[test]
fn tokenize_quoted_strings() {
    let toks = tokenize("a: 'sq'\nb: \"dq\"\n").unwrap();
    let styles: Vec<ScalarStyle> = toks
        .iter()
        .filter_map(|t| match t {
            Token::Scalar { style, .. } => Some(*style),
            _ => None,
        })
        .collect();
    assert_eq!(
        styles,
        vec![ScalarStyle::SingleQuoted, ScalarStyle::DoubleQuoted]
    );
}

#[test]
fn tokenize_tag() {
    let toks = tokenize("script: !reference [.x, y]\n").unwrap();
    assert!(toks.iter().any(|t| matches!(t, Token::Tag { .. })));
}

#[test]
fn tokenize_indent_levels() {
    let toks = tokenize("a:\n  b:\n    c: 1\n").unwrap();
    let indents: Vec<usize> = toks
        .iter()
        .filter_map(|t| {
            if let Token::Indent(n) = t {
                Some(*n)
            } else {
                None
            }
        })
        .collect();
    assert_eq!(indents, vec![0, 2, 4]);
}
