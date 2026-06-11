#!/usr/bin/env python3
"""Generate the Hub-IR schema from catalog/ir.toml.

catalog/ir.toml is the *normalization* catalog — it records, per IR
construct, which platform key feeds each field. The Hub-IR schema is
the IR's own definition stripped of that platform detail: the
node-kinds, their fields, and — for every field — whether it is a
scalar attribute or a `ref` edge to another node-kind.

This is the spec `pipeline-hub-ir` and the forward/backward TGG
rules build against. Output: catalog/hub_schema.toml. Regenerate
whenever ir.toml changes.
"""
import pathlib
import tomllib

CAT = pathlib.Path(__file__).parent


def main():
    ir = tomllib.load(open(CAT / "ir.toml", "rb"))
    nodes = {k: v for k, v in ir.items() if k != "meta" and isinstance(v, dict)}
    mapped = [k for k, v in nodes.items() if "maps" in v]
    lexical = [k for k, v in nodes.items() if "maps" not in v]
    cset = set(mapped)

    def classify(fname, ftab=None):
        """A field is a `ref:<kind>` edge or a `scalar` attribute.

        An explicit `ref = "<construct>"` in the ir.toml field table wins —
        for fields whose NAME doesn't encode the target construct (e.g.
        `artifact.subtypes` is a list of nested artifacts, but "subtypes"
        matches no construct name)."""
        if isinstance(ftab, dict) and isinstance(ftab.get("ref"), str):
            return f"ref:{ftab['ref']}"
        # Explicit `scalar = true` forces a scalar attribute even when the
        # field NAME happens to contain a construct name (circleci's
        # `restore_cache`/`save_cache` are opaque step bodies captured as a
        # block, NOT ref:cache edges).
        if isinstance(ftab, dict) and ftab.get("scalar") is True:
            return "scalar"
        for cand in (fname, fname.rstrip("s")):
            if cand in cset:
                return f"ref:{cand}"
        for c in sorted(cset, key=len, reverse=True):
            if fname.endswith("_" + c) or fname.endswith("_" + c + "s") or fname.endswith(c + "s"):
                return f"ref:{c}"
        if fname in ("needs", "depends_on"):
            return "ref:dependency_edge"
        return "scalar"

    out = [
        "# catalog/hub_schema.toml — the Hub-IR schema. Generated from",
        "# catalog/ir.toml by gen_hub_schema.py. Node-kinds + fields;",
        "# each field is a scalar attribute or a `ref:<kind>` edge to",
        "# another node. This is the spec pipeline-hub-ir and the TGG",
        "# rules are built against — do not hand-edit; regenerate.",
        "#",
        "# SATELLITE MODEL (a): a `scalar` field is NOT an attribute on",
        "# the construct node — the TGG engine cannot bind onto a",
        "# shared-anchor node. It is realised as a `hub:attr` child",
        "# node {name, value} on a `has_attr` edge. A `ref` field is a",
        "# child node of the referenced kind. Every field is a child;",
        "# only the always-fresh `hub:attr` node carries real bound",
        "# attributes.",
        "",
        "[meta]",
        "kind = \"hub-schema\"",
        "model = \"satellite\"",
        "generated_from = \"catalog/ir.toml\"",
        f"node_kinds = {len(mapped)}",
        f"lexical_kinds = {len(lexical)}",
        "",
    ]

    edge_kinds = set()
    for kind in sorted(mapped):
        node = nodes[kind]
        fields = node.get("field", {})
        out.append(f"[node.{kind}]")
        doc = node.get("doc", "")
        out.append(f"doc = {_s(doc)}")
        out.append(f"[node.{kind}.fields]")
        for fname in sorted(fields):
            cls = classify(fname, fields.get(fname))
            out.append(f"{_k(fname)} = {_s(cls)}")
            if cls.startswith("ref:"):
                edge_kinds.add(f"has_{cls[4:]}")
        out.append("")

    for kind in sorted(lexical):
        node = nodes[kind]
        out.append(f"[lexical.{kind}]")
        out.append(f"doc = {_s(node.get('doc', ''))}")
        out.append("")

    # The universal satellite node-kind (model a). Every scalar field
    # of every construct is realised as one of these.
    edge_kinds.add("has_attr")
    out.append("[node.attr]")
    out.append(
        'doc = "Satellite carrying one scalar field of its parent '
        'construct — name = the field, value = the resolved scalar. '
        'Always freshly created, so its own attributes are bindable."'
    )
    out.append("[node.attr.fields]")
    out.append('name = "scalar"')
    out.append('value = "scalar"')
    out.append("")

    # Unified field-value model: collections + leaf values as first-class
    # nodes, so list fields are symmetric with scalar fields. A field is
    # `hub:attr{name} -has_value-> {hub:value | hub:collection}`; a
    # collection's elements hang off `has_item` edges.
    edge_kinds.add("has_value")
    edge_kinds.add("has_item")
    out.append("[node.collection]")
    out.append(
        'doc = "An ordered list value of a field — has_item edges to its '
        "elements (a hub:value for scalar lists, a construct for ref "
        'lists). Identity from its parent hub:attr."'
    )
    out.append("[node.collection.fields]")
    out.append("")
    out.append("[node.value]")
    out.append(
        'doc = "A leaf scalar value node — the resolved text of a scalar '
        "field or a scalar-list element. Always freshly created, so its "
        'attributes are bindable."'
    )
    out.append("[node.value.fields]")
    out.append('text = "scalar"')
    out.append("")

    meta_idx = out.index("[meta]")
    out.insert(
        meta_idx + 2, f"edge_kinds = {sorted(edge_kinds)!r}".replace("'", '"')
    )

    (CAT / "hub_schema.toml").write_text("\n".join(out))
    print(
        f"hub_schema.toml — {len(mapped)} node-kinds, "
        f"{len(lexical)} lexical, {len(edge_kinds)} edge-kinds"
    )


def _s(v):
    return '"' + str(v).replace("\\", "\\\\").replace('"', '\\"') + '"'


def _k(k):
    import re

    return k if re.fullmatch(r"[A-Za-z0-9_-]+", k) else _s(k)


if __name__ == "__main__":
    main()
