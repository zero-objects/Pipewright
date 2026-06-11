#!/usr/bin/env python3
"""Generate the Rust Hub-IR schema module from catalog/hub_schema.toml.

The catalog is a code-gen spec, not documentation: hub_schema.toml
defines the IR's node-kinds, fields and edges, and this emits the
Rust the whole workspace builds against —
crates/pipeline-hub-ir/src/schema.rs.

The emitted module is the typed-graph IR model: node-kind / edge-kind
id constants plus a `SCHEMA` table (node-kind -> fields, each a
scalar attribute or a `Ref` edge). Regenerate when hub_schema.toml
changes; never hand-edit schema.rs.
"""
import pathlib
import tomllib

CAT = pathlib.Path(__file__).parent
OUT = CAT.parent / "crates" / "pipeline-hub-ir" / "src" / "schema.rs"


def screaming(name):
    return name.upper().replace("-", "_")


def main():
    schema = tomllib.load(open(CAT / "hub_schema.toml", "rb"))
    nodes = {k.split(".", 1)[1]: v for k, v in _flat(schema, "node")}
    lexical = {k.split(".", 1)[1]: v for k, v in _flat(schema, "lexical")}

    edge_ids = {"attr"}  # the universal scalar-satellite edge (model a)
    for node in nodes.values():
        for kind in node.get("fields", {}).values():
            if isinstance(kind, str) and kind.startswith("ref:"):
                edge_ids.add(kind[4:])

    L = []
    p = L.append
    p('#![allow(clippy::doc_markdown, reason = "generated module; '
      'prose mentions TGG / acronyms")]')
    p("")
    p("//! Hub-IR schema — GENERATED from catalog/hub_schema.toml by")
    p("//! catalog/gen_hub_rs.py. Do not edit; regenerate.")
    p("//!")
    p("//! The platform-neutral IR is a typed graph. This module is")
    p("//! its model: node-kind / edge-kind id constants and the")
    p("//! `SCHEMA` table that the forward/backward TGG rule")
    p("//! generators and graph validation build against.")
    p("")
    p("/// A field of a node-kind.")
    p("#[derive(Debug, Clone, Copy, PartialEq, Eq)]")
    p("pub enum FieldKind {")
    p("    /// A scalar attribute carried on the node.")
    p("    Scalar,")
    p("    /// An edge to another node-kind (the target node-kind id).")
    p("    Ref(&'static str),")
    p("}")
    p("")
    p("/// One field of a node-kind.")
    p("#[derive(Debug, Clone, Copy)]")
    p("pub struct Field {")
    p("    pub name: &'static str,")
    p("    pub kind: FieldKind,")
    p("}")
    p("")
    p("/// One node-kind of the IR graph.")
    p("#[derive(Debug, Clone, Copy)]")
    p("pub struct NodeKind {")
    p("    pub id: &'static str,")
    p("    /// Lexical kinds (comment, anchor, …) carry round-trip")
    p("    /// fidelity, not pipeline semantics.")
    p("    pub lexical: bool,")
    p("    pub fields: &'static [Field],")
    p("}")
    p("")
    p("/// Node-kind id constants.")
    p("pub mod node {")
    for name in sorted(nodes) + sorted(lexical):
        p(f'    pub const {screaming(name)}: &str = "hub:{name}";')
    p("}")
    p("")
    p("/// Edge-kind id constants.")
    p("pub mod edge {")
    for t in sorted(edge_ids):
        p(f'    pub const HAS_{screaming(t)}: &str = "hub:has_{t}";')
    p("}")
    p("")
    p("/// The full IR schema — every node-kind and its fields.")
    p("pub const SCHEMA: &[NodeKind] = &[")
    for name in sorted(nodes):
        _emit_node(p, name, nodes[name], lexical=False)
    for name in sorted(lexical):
        _emit_node(p, name, lexical[name], lexical=True)
    p("];")
    p("")

    OUT.write_text("\n".join(L))
    print(
        f"schema.rs — {len(nodes)} node-kinds, {len(lexical)} lexical, "
        f"{len(edge_ids)} edge-kinds"
    )


def _emit_node(p, name, node, lexical):
    p(f'    NodeKind {{ id: "hub:{name}", lexical: {str(lexical).lower()}, fields: &[')
    for fname, kind in sorted(node.get("fields", {}).items()):
        if isinstance(kind, str) and kind.startswith("ref:"):
            fk = f'FieldKind::Ref("hub:{kind[4:]}")'
        else:
            fk = "FieldKind::Scalar"
        p(f'        Field {{ name: "{fname}", kind: {fk} }},')
    p("    ] },")


def _flat(schema, prefix):
    """Yield (`prefix.name`, table) for each `[prefix.name]` section."""
    section = schema.get(prefix, {})
    for name, table in section.items():
        if isinstance(table, dict):
            yield f"{prefix}.{name}", table


if __name__ == "__main__":
    main()
