use pipeline_cst::{parse, AnchorTable};

#[test]
fn anchor_table_finds_simple_anchor() {
    let s = ".defaults: &defaults\n  image: alpine\n";
    let doc = parse(s).unwrap();
    let table = AnchorTable::collect(&doc);
    assert_eq!(table.names(), vec!["defaults"]);
    assert!(table.resolve("defaults").is_some());
}

#[test]
fn anchor_table_finds_multiple_anchors() {
    let s = ".a: &one 1\n.b: &two 2\n.c: &three 3\n";
    let doc = parse(s).unwrap();
    let table = AnchorTable::collect(&doc);
    assert_eq!(table.names(), vec!["one", "three", "two"]);
}

#[test]
fn anchor_table_finds_fixture_04_anchor() {
    let s = include_str!("corpus/fixtures/04_with_anchors.yml");
    let doc = parse(s).unwrap();
    let table = AnchorTable::collect(&doc);
    assert!(
        table.resolve("defaults").is_some(),
        "defaults anchor present in fixture 04"
    );
}

#[test]
fn anchor_resolves_to_value_node_not_entry() {
    let s = ".defaults: &defaults the_value\n";
    let doc = parse(s).unwrap();
    let table = AnchorTable::collect(&doc);
    let resolved = table.resolve("defaults").expect("resolved");
    // It should be the VALUE, not the MappingEntry — i.e., the scalar 'the_value'.
    let span_text = doc.span_text(resolved.span);
    assert_eq!(span_text, "the_value");
}
