use pipeline_cst::{parse, top_level_logical, EntrySource};

#[test]
fn logical_view_expands_merge_for_fixture_04() {
    let s = include_str!("corpus/fixtures/04_with_anchors.yml");
    let doc = parse(s).unwrap();
    let entries = top_level_logical(&doc);
    // The .defaults entry comes first, then build, then test.
    // We expect 'build' to be present, and 'test', and '.defaults'.
    let keys: Vec<String> = entries.iter().map(|e| e.key.clone()).collect();
    assert!(keys.contains(&".defaults".to_string()));
    assert!(keys.contains(&"build".to_string()));
    assert!(keys.contains(&"test".to_string()));
}

#[test]
fn merged_keys_marked_as_merged() {
    // Construct a tiny scenario: defaults has `image:` + `before_script:`;
    // build merges defaults AND has its own `script:`.
    let s = ".defaults: &defaults\n  image: alpine\n  before_script:\n    - x\nbuild:\n  <<: *defaults\n  script:\n    - y\n";
    let doc = parse(s).unwrap();
    // Find build entry via top-level mapping.
    let mapping = doc
        .root()
        .children
        .iter()
        .find(|c| matches!(c.kind, pipeline_cst::NodeKind::Mapping))
        .unwrap();
    let build_entry = mapping
        .children
        .iter()
        .find(|e| {
            matches!(
                &e.kind,
                pipeline_cst::NodeKind::MappingEntry { key_text } if key_text == "build"
            )
        })
        .expect("build entry");
    let build_value = &build_entry.children[1];

    let table = pipeline_cst::AnchorTable::collect(&doc);
    let entries = pipeline_cst::mapping_entries_logical(build_value, &table);

    // 'script' is own; 'image' and 'before_script' come from merge.
    let by_key: std::collections::HashMap<String, EntrySource> = entries
        .iter()
        .map(|e| (e.key.clone(), e.source.clone()))
        .collect();
    assert_eq!(by_key.get("script"), Some(&EntrySource::Direct));
    assert!(matches!(
        by_key.get("image"),
        Some(EntrySource::Merged { .. })
    ));
    assert!(matches!(
        by_key.get("before_script"),
        Some(EntrySource::Merged { .. })
    ));
}

#[test]
fn own_keys_override_merged() {
    let s = ".defaults: &defaults\n  image: alpine\nbuild:\n  <<: *defaults\n  image: rust:1.88\n";
    let doc = parse(s).unwrap();
    let mapping = doc
        .root()
        .children
        .iter()
        .find(|c| matches!(c.kind, pipeline_cst::NodeKind::Mapping))
        .unwrap();
    let build_entry = mapping
        .children
        .iter()
        .find(|e| {
            matches!(
                &e.kind,
                pipeline_cst::NodeKind::MappingEntry { key_text } if key_text == "build"
            )
        })
        .expect("build entry");
    let build_value = &build_entry.children[1];

    let table = pipeline_cst::AnchorTable::collect(&doc);
    let entries = pipeline_cst::mapping_entries_logical(build_value, &table);

    let image_entry = entries.iter().find(|e| e.key == "image").expect("image");
    assert_eq!(image_entry.source, EntrySource::Direct, "own wins");
    let image_value_span = image_entry.value.span;
    let image_text = doc.span_text(image_value_span);
    assert!(image_text.contains("rust"), "got: {image_text:?}");
}
