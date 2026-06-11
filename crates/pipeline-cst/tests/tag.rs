use pipeline_cst::{collect_tags, parse, ResolvedTag};

#[test]
fn reference_tag_in_flow_list_resolves() {
    let s = ".x:\n  script:\n    - echo build\nuse:\n  script: !reference [.x, script]\n";
    let doc = parse(s).unwrap();
    let tags = collect_tags(&doc);
    let reference = tags
        .iter()
        .find_map(|(_, t)| match t {
            ResolvedTag::Reference { path } => Some(path.clone()),
            _ => None,
        })
        .expect("reference tag found");
    assert_eq!(reference, vec![".x", "script"]);
}

#[test]
fn unknown_tag_kept_as_opaque() {
    let s = "foo: !custom 42\n";
    let doc = parse(s).unwrap();
    let tags = collect_tags(&doc);
    let found = tags
        .iter()
        .any(|(_, t)| matches!(t, ResolvedTag::Unknown { tag_name, .. } if tag_name == "!custom"));
    assert!(found);
}

#[test]
fn secret_tag_extracts_name() {
    let s = "foo: !secret MY_KEY\n";
    let doc = parse(s).unwrap();
    let tags = collect_tags(&doc);
    let secret = tags
        .iter()
        .find_map(|(_, t)| match t {
            ResolvedTag::Secret { name } => Some(name.clone()),
            _ => None,
        })
        .expect("secret tag");
    assert_eq!(secret, "MY_KEY");
}
