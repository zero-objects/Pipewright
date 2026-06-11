use pipeline_cst::{parse, NodeKind};

#[test]
fn tree_top_level_mapping_has_entries() {
    let s = "build:\n  script:\n    - x\ntest:\n  script:\n    - y\n";
    let doc = parse(s).unwrap();
    assert!(matches!(doc.root().kind, NodeKind::Document));
    assert_eq!(doc.root().children.len(), 1);
    let map = &doc.root().children[0];
    assert!(matches!(map.kind, NodeKind::Mapping));
    assert_eq!(map.children.len(), 2);
    for entry in &map.children {
        assert!(matches!(entry.kind, NodeKind::MappingEntry { .. }));
    }
}

#[test]
fn tree_same_indent_block_sequence_is_the_key_value() {
    // A block sequence value may sit at the SAME indent as its key (valid
    // YAML: `stages:\n- a\n- b`). Regression: the parser used to require the
    // value to be MORE-indented, leaving the dash orphaned and aborting the
    // whole top-level mapping — so `build:` below silently vanished.
    let s = "stages:\n- build\n- test\nbuild:\n  stage: build\n  script:\n  - make\n";
    let doc = parse(s).unwrap();
    let map = &doc.root().children[0];
    assert!(matches!(map.kind, NodeKind::Mapping));
    let keys: Vec<String> = map
        .children
        .iter()
        .filter_map(|e| match &e.kind {
            NodeKind::MappingEntry { key_text } => Some(key_text.clone()),
            _ => None,
        })
        .collect();
    // Both top-level keys survive (the bug dropped `build`).
    assert_eq!(keys, vec!["stages", "build"]);
    // `stages` value is a 2-item sequence, not an empty scalar.
    let stages = &map.children[0];
    let stages_value = &stages.children[1];
    assert!(
        matches!(stages_value.kind, NodeKind::Sequence),
        "stages value: {:?}",
        stages_value.kind
    );
    assert_eq!(stages_value.children.len(), 2);
    // `build.script` (also a same-indent sequence) keeps its one item.
    let build = &map.children[1];
    let build_map = &build.children[1];
    let script = build_map
        .children
        .iter()
        .find(|e| matches!(&e.kind, NodeKind::MappingEntry { key_text } if key_text == "script"))
        .expect("script entry");
    assert!(matches!(script.children[1].kind, NodeKind::Sequence));
    assert_eq!(script.children[1].children.len(), 1);
}

#[test]
fn tree_plain_flow_list_expands_to_a_sequence() {
    // `needs: [build, lint]` must become a real sequence so downstream sees the
    // same shape as the block form (else the items are dropped). Regression J2.
    let s = "test:\n  needs: [build, lint]\n";
    let doc = parse(s).unwrap();
    let map = &doc.root().children[0];
    let entry = &map.children[0]; // test:
    let needs_entry = &entry.children[1].children[0]; // job mapping → needs entry
    let value = &needs_entry.children[1];
    assert!(
        matches!(value.kind, NodeKind::Sequence),
        "needs value: {:?}",
        value.kind
    );
    let items: Vec<&str> = value
        .children
        .iter()
        .map(|it| &s[it.span.start..it.span.end])
        .collect();
    assert_eq!(items, vec!["build", "lint"]);
}

#[test]
fn tree_nested_flow_list_stays_opaque() {
    // A flow list nesting a map (`[{name: x}]`) can't be comma-split safely, so
    // it's left as an opaque scalar rather than mangled.
    let s = "x:\n  arguments: [{name: a, value: b}]\n";
    let doc = parse(s).unwrap();
    let map = &doc.root().children[0];
    let args_entry = &map.children[0].children[1].children[0];
    let value = &args_entry.children[1];
    assert!(
        matches!(value.kind, NodeKind::Scalar { .. }),
        "nested flow stays scalar: {:?}",
        value.kind
    );
}

#[test]
fn tree_mapping_entry_holds_key_text() {
    let s = "alpha:\n  script:\n    - 1\nbeta:\n  script:\n    - 2\n";
    let doc = parse(s).unwrap();
    let map = &doc.root().children[0];
    let key_texts: Vec<String> = map
        .children
        .iter()
        .filter_map(|e| match &e.kind {
            NodeKind::MappingEntry { key_text } => Some(key_text.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(key_texts, vec!["alpha", "beta"]);
}

#[test]
fn tree_anchor_attached_to_value_node() {
    let s = ".x: &xx some_value\n";
    let doc = parse(s).unwrap();
    let map = &doc.root().children[0];
    let entry = &map.children[0];
    let value = &entry.children[1];
    assert_eq!(value.anchor.as_deref(), Some("xx"));
    // Entry itself does NOT carry the anchor any more (it lives on the value).
    assert!(entry.anchor.is_none());
}

fn collect_comments(
    n: &pipeline_cst::Node,
    src: &str,
    out: &mut Vec<(pipeline_cst::CommentKind, String)>,
) {
    match n.kind {
        NodeKind::Comment { kind } => {
            out.push((kind, src[n.span.start..n.span.end].to_string()));
        }
        _ => {
            for c in &n.children {
                collect_comments(c, src, out);
            }
        }
    }
}

#[test]
fn tree_comments_appear_as_comment_nodes() {
    use pipeline_cst::CommentKind;
    let s = "# top\nbuild:\n  # @hub:pipeline.name=\"My Pipeline\"\n  script:\n    - cargo build  # trailing\n";
    let doc = parse(s).unwrap();
    let mut comments = Vec::new();
    collect_comments(doc.root(), s, &mut comments);
    assert_eq!(
        comments,
        vec![
            (CommentKind::FullLine, "# top".to_string()),
            (
                CommentKind::FullLine,
                "# @hub:pipeline.name=\"My Pipeline\"".to_string(),
            ),
            (CommentKind::Trailing, "# trailing".to_string()),
        ],
    );
}

#[test]
fn tree_alias_appears_as_alias_node() {
    let s = ".x: &xx 1\nuse: *xx\n";
    let doc = parse(s).unwrap();
    let map = &doc.root().children[0];
    // Second entry's value should be an Alias node.
    let second = &map.children[1];
    let value = &second.children[1];
    assert!(matches!(&value.kind, NodeKind::Alias { name } if name == "xx"));
}
