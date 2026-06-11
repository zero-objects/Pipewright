#!/usr/bin/env python3
"""Derive the construct-classification table from the catalog.

The seeder must tag every `cst:Mapping` that represents an IR
construct with `construct = "<kind>"`. That classification is
*derivable*: in `ir.toml` every ref field `Cparent.f -> Cchild` is
realised, per platform, by a set of keys; the value-mapping of any
entry with one of those keys IS a `Cchild`.

This emits, per platform:
  * catalog/classify/<platform>.toml — the {key: construct} table
    (a reviewable catalog artifact), and
  * for GitLab, a generated Rust module the seeder consumes
    (crates/pipeline-gitlab-tgg/src/forward/classification.rs).

Two positions are NOT key-derivable and stay platform-specific in
the seeder: the root mapping is always `pipeline`; a GitLab job is
an unnamed top-level entry.
"""
import pathlib
import tomllib

CAT = pathlib.Path(__file__).parent

# YAML platforms get a generated classification module — single
# source of truth at pipeline-tgg-seeder/src/classify/<plat>.rs.
# Earthly + Dagger use non-YAML source and would need their own
# parser/CST first; Jenkins has its own Groovy CST + ruleset.
RUST_TARGETS = [
    "gitlab", "github", "azure", "circleci", "travis", "bitbucket",
    "buildkite", "drone", "woodpecker", "tekton", "argo",
    "google_cloudbuild", "aws_codebuild", "aws_codepipeline",
    # Jenkins isn't YAML, but pipeline-jenkinsfile-cst translates
    # Jenkinsfile DSL into the same pipeline_cst::Document shape,
    # so the shared seeder primitives + classify table apply.
    "jenkins",
    # Dagger configs are JSON (`dagger.json`); JSON is a strict
    # subset of YAML so the YAML parser handles it directly. The
    # build LOGIC lives in SDK code which is unparseable here —
    # only the module manifest is in scope.
    "dagger",
    # Earthly's DSL (`Earthfile`) is verb-based, not YAML.
    # pipeline-earthfile-cst translates it into the same
    # pipeline_cst::Document shape so the shared seeder applies.
    "earthly",
]
CLASSIFY_DIR = (
    CAT.parent / "crates" / "pipeline-tgg-seeder" / "src" / "classify"
)

# CURATED container-key aliases, per platform: a platform names a construct
# CONTAINER with a key that DIFFERS from the IR field name, so the name-match
# derivation (key == field name) misses it and the container's items never get
# construct-tagged → the whole container is dropped forward. These are
# hand-verified, NOT heuristically derived (a heuristic also catches schema-
# mis-modelled scalar fields like nodeSelector→agent and invents IR). Each
# entry: "the items under <key> ARE instances of <construct>".
#   tekton:  spec.tasks  -> hub:step  (a tekton task IS the pipeline's step)
#   gcb:     artifacts.<ecosystem> -> hub:artifact  (each ecosystem group —
#            goModules/mavenArtifacts/npmPackages/pythonPackages — is a list
#            of nested artifacts; the IR field is `artifact.subtypes`, whose
#            name matches none of the keys, so derivation misses them)
CONTAINER_ALIASES: dict[str, dict[str, str]] = {
    # gitlab `workflow:` is the pipeline-level trigger; its `rules:` are clauses.
    "gitlab": {"workflow": "trigger"},
    "tekton": {"tasks": "job"},
    # Azure trigger filters: `trigger.branches` / `.tags` / `.paths` (and the
    # same under `pr:` / `schedules[]`) are each an include/exclude filter
    # object. The IR models them as distinct ref fields (branch_filter /
    # tag_filter / path_filter → include_exclude_filter) whose names differ
    # from the platform keys, so the fname==key derivation skips them. Alias
    # each filter key so its `{include, exclude}` mapping is tagged
    # construct=include_exclude_filter and the patterns round-trip.
    "azure": {
        "branches": "include_exclude_filter",
        "tags": "include_exclude_filter",
        "paths": "include_exclude_filter",
        "pr": "pull_request",
    },
    # argo `spec.templates[]` IS the pipeline's JOB list ([pipeline.field.
    # jobs] argo = ["templates"], ref:job). A template is a polymorphism
    # helper: ONE construct (job) carrying optional body fields per variant
    # (container/script/steps/dag/resource/suspend/inputs/outputs/…), each
    # mapped by a job.field.* rule. The key name (templates) doesn't equal
    # the IR field name (jobs), so the fname==key derivation skips it — alias
    # it so each template is tagged construct=job and its body round-trips
    # losslessly (a flat step-tag would drop every non-container body).
    "argo": {"templates": "job"},
    "google_cloudbuild": {
        "goModules": "artifact",
        "mavenArtifacts": "artifact",
        "npmPackages": "artifact",
        "pythonPackages": "artifact",
    },
}


def main():
    ir = tomllib.load(open(CAT / "ir.toml", "rb"))
    hub = tomllib.load(open(CAT / "hub_schema.toml", "rb"))
    targets = [t for t in tomllib.load(open(CAT / "targets.toml", "rb")) if t != "meta"]

    # ref field kinds: (construct, field) -> child construct
    ref_target = {}
    for cname, node in hub.get("node", {}).items():
        for fname, k in node.get("fields", {}).items():
            if isinstance(k, str) and k.startswith("ref:"):
                ref_target[(cname, fname)] = k[4:]

    # LIST-CANONICAL construct fields, per platform. A ref-construct
    # field whose manifest type can be a list (`list<X>` or the
    # `X | list<X>` sugar union) is modelled with the LIST as the
    # canonical form — the single mapping form is YAML shorthand for a
    # one-element list and is semantically identical (so the IR must
    # NOT distinguish them; doing so would invent data). The seeder
    # canonicalises a single-mapping value of such a key into a
    # one-item sequence, so exactly ONE bijective rule (seq_mapping_
    # nodes) governs both directions — no ambiguous mapping_node↔ that
    # would otherwise collapse an N-item collection on reverse. The
    # KEY (manifest `from`) is what the seeder matches on.
    list_fields: dict[str, set] = {}
    # map<construct> keys: a ref field whose platform type is `map<X>`. Its
    # value is a MAP of named instances (`services: {main: {...}}`), so the
    # classify entry is marked `map:<construct>` and the seeder tags each
    # inner entry's value, not the outer map wrapper.
    map_construct_keys: dict[str, set] = {}
    for plat in targets:
        man_path = CAT / "rules" / f"{plat}.toml"
        if not man_path.exists():
            continue
        man = tomllib.load(open(man_path, "rb"))
        keys: set = set()
        map_keys: set = set()
        for r in man.get("rule", []):
            to = r.get("to", "")
            typ = r.get("type", "")
            frm = r.get("from", "")
            if "." not in to or not frm:
                continue
            parent_t, field_t = to.split(".", 1)
            if (parent_t, field_t) not in ref_target:
                continue
            if "list<" in typ:
                keys.add(frm)
            # `map<` ANYWHERE in the type (not just a bare prefix) — a
            # `list<X> | map<X>` union (woodpecker `steps:`) carries a map
            # arm too, and its map form must tag inner entries one level
            # deep (the seq arm is unaffected: a Sequence value has no
            # MappingEntry children, so is_map_construct no-ops on it).
            if "map<" in typ:
                map_keys.add(frm)
        list_fields[plat] = keys
        map_construct_keys[plat] = map_keys

    out_dir = CAT / "classify"
    out_dir.mkdir(exist_ok=True)

    for plat in targets:
        # key -> construct, derived from every ref field's platform keys
        table = {}
        conflicts = {}
        # First pass: collect every (key → child) candidate WITH the
        # field-name it came from. Some IR fields list bag-of-keys
        # that aren't structural creators (bitbucket's [hook.field.job]
        # lists `image`/`clone`/… — those are job-FIELDS, not
        # job-INSTANCE keys, and the gen-output had `("image",
        # "job")` instead of `("image", "image")` because hook's
        # field-loop ran first and won). Resolve by preferring the
        # field whose NAME matches the key — that's the case where
        # the catalog is saying "the value mapping under <key> IS an
        # instance of the referenced construct".
        candidates: dict[str, list[tuple[str, str, str]]] = {}
        for cname, node in ir.items():
            if not isinstance(node, dict) or "field" not in node:
                continue
            for fname, ftab in node["field"].items():
                child = ref_target.get((cname, fname))
                if not child or not isinstance(ftab, dict):
                    continue
                for key in ftab.get(plat, []):
                    candidates.setdefault(key, []).append((cname, fname, child))
        for key, cand in candidates.items():
            # Only emit a classify entry when the platform key
            # equals the IR field name — the canonical structural
            # case where "<key>: <mapping>" means "this mapping
            # IS an instance of the field's referenced construct".
            #
            # Bag-of-keys fields (e.g. `[hook.field.job] bitbucket
            # = ["name", "image", "services", ...]`) list 17
            # different platform keys for a single IR field
            # because bitbucket's hook construct exposes all of a
            # job's sub-fields at the hook level. Treating those
            # as "job-creator" entries was the bug — every `image:`
            # block at any nesting got tagged construct=job, and
            # the chaos generator's roundtrips broke because two
            # name carriers fought over the hub:job's identity.
            name_matches = [c for cname, fname, c in cand if fname == key]
            distinct_matches: set[str] = set(name_matches)
            if len(distinct_matches) == 1:
                table[key] = next(iter(distinct_matches))
                continue
            if len(distinct_matches) > 1:
                # Multiple ref fields with field_name == key claim
                # different children. Keep the first (alphabetical)
                # and record the conflict for review.
                table[key] = sorted(distinct_matches)[0]
                conflicts[key] = distinct_matches
                continue
            # No field-name-matches: the key is a bag entry only.
            # Skip classify emission — the seeder will fall through
            # to seed_top_entry_as_meta which doesn't classify-tag.
            distinct_children: set[str] = {c for _, _, c in cand}
            if len(distinct_children) > 1:
                conflicts[key] = distinct_children

        # Curated container-key aliases (hand-verified, see CONTAINER_ALIASES).
        for alias_key, alias_child in CONTAINER_ALIASES.get(plat, {}).items():
            table.setdefault(alias_key, alias_child)

        # Mark map<construct> keys: the seeder tags each inner entry's value,
        # not the outer map wrapper (see lib.rs is_map_construct / seed_value).
        for key in map_construct_keys.get(plat, set()):
            if key in table:
                table[key] = f"map:{table[key]}"

        lines = [
            f"# catalog/classify/{plat}.toml — construct classification,",
            "# derived from ir.toml: an entry with one of these keys has",
            "# a value-mapping of the named IR construct.",
            "",
            "[meta]",
            f'platform = "{plat}"',
            f"key_count = {len(table)}",
            "",
            "[classify]",
        ]
        for key in sorted(table):
            lines.append(f'{_k(key)} = "{table[key]}"')
        if conflicts:
            lines.append("")
            lines.append("# keys whose value-mapping construct is context-dependent:")
            for key, kinds in sorted(conflicts.items()):
                lines.append(f"#   {key}: {sorted(kinds)}")
        (out_dir / f"{plat}.toml").write_text("\n".join(lines) + "\n")

    # Generated Rust tables — one module per YAML platform under
    # pipeline-tgg-seeder/src/classify/.
    CLASSIFY_DIR.mkdir(parents=True, exist_ok=True)
    written = []
    mod_lines = [
        "//! GENERATED from catalog/classify/*.toml by",
        "//! catalog/gen_classification.py. Do not edit; regenerate.",
        "//!",
        "//! Per-platform construct classification tables. Each one",
        "//! maps a mapping-entry key to the IR construct kind the",
        "//! entry's value-mapping represents — the seeder tags the",
        "//! `cst:Mapping` accordingly so TGG rules can anchor on it.",
        "",
    ]
    for plat in RUST_TARGETS:
        plat_table = tomllib.load(open(out_dir / f"{plat}.toml", "rb"))["classify"]
        rs_path = CLASSIFY_DIR / f"{plat}.rs"
        rs = [
            f"//! Construct classification for {plat}. Generated.",
            "",
            "/// `(key, ir-construct)`.",
            "pub const CONSTRUCT_KEYS: &[(&str, &str)] = &[",
        ]
        for key in sorted(plat_table):
            rs.append(f'    ({_q(key)}, "{plat_table[key]}"),')
        rs.append("];")
        rs.append("")
        # List-canonical construct fields: keys whose value is modelled
        # as a (possibly one-element) list. The seeder wraps a single
        # mapping value of one of these into a one-item sequence so the
        # bijective seq rule governs both directions.
        rs.append("/// Construct-field keys whose value is canonically a LIST")
        rs.append("/// (the single-mapping form is sugar for a one-item list).")
        rs.append("pub const LIST_CONSTRUCT_KEYS: &[&str] = &[")
        for key in sorted(list_fields.get(plat, set())):
            rs.append(f"    {_q(key)},")
        rs.append("];")
        rs.append("")
        rs_path.write_text("\n".join(rs))
        mod_lines.append(f"pub mod {plat};")
        written.append((plat, len(plat_table)))

    mod_lines.append("")
    # Static dispatch: look up a platform's table by lowercase name.
    mod_lines.append("/// Look up a platform's classification table by name.")
    mod_lines.append("#[must_use]")
    mod_lines.append("pub fn for_platform(name: &str) -> Option<crate::Classify<'static>> {")
    mod_lines.append("    match name {")
    for plat, _ in written:
        mod_lines.append(f'        "{plat}" => Some({plat}::CONSTRUCT_KEYS),')
    mod_lines.append("        _ => None,")
    mod_lines.append("    }")
    mod_lines.append("}")
    mod_lines.append("")
    # Dispatch for the list-canonical construct-field keys.
    mod_lines.append("/// Look up a platform's list-canonical construct-field keys.")
    mod_lines.append("#[must_use]")
    mod_lines.append("pub fn list_fields_for_platform(name: &str) -> &'static [&'static str] {")
    mod_lines.append("    match name {")
    for plat, _ in written:
        mod_lines.append(f'        "{plat}" => {plat}::LIST_CONSTRUCT_KEYS,')
    mod_lines.append("        _ => &[],")
    mod_lines.append("    }")
    mod_lines.append("}")
    mod_lines.append("")

    (CLASSIFY_DIR / "mod.rs").write_text("\n".join(mod_lines))

    print(f"classify/*.toml for {len(targets)} platforms")
    for plat, n in written:
        print(f"  {plat:18s} {n:4d} keys")


def _k(key):
    import re

    return key if re.fullmatch(r"[A-Za-z0-9_-]+", key) else _q(key)


def _q(s):
    return '"' + s.replace("\\", "\\\\").replace('"', '\\"') + '"'


if __name__ == "__main__":
    main()
