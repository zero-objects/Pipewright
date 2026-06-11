#!/usr/bin/env python3
"""Generate the TGG rule manifest for every platform from the catalog.

Every line of catalog/ir.toml — `[<ir-construct>.field.<f>].<plat> =
[keys]` — IS a rule: it says a platform key feeds an IR field. This
turns those lines into an explicit per-platform rule manifest.

Each rule's SHAPE is derived mechanically from two facts already in
the catalog:
  * the platform key's TYPE (from catalog/<platform>.toml), and
  * the IR field's kind — scalar attribute or `ref` edge (from
    catalog/hub_schema.toml).
The (type, field-kind) pair selects one of a small fixed set of
rule shapes. Lowering a shape to a concrete seesaw RuleSpec is the
mechanical next step — the shapes ARE the L-pattern builders
already hand-written for GitLab (scalar / seq / mapping forms).

Output: catalog/rules/<platform>.toml — the manifest a per-platform
RuleSetSpec is generated from. A TGG rule set is
bidirectional — one set, run either direction by the engine.
"""
import pathlib
import tomllib

CAT = pathlib.Path(__file__).parent
SCALAR_TYPES = {"string", "boolean", "integer", "number", "enum", "null"}
# `any` is intentionally not scalar: a `list<any>` field on a ref
# IR field (e.g. drone pipeline.steps) carries structured objects
# in practice; classifying it as `seq_scalar_nodes` makes the
# generated rule expect cst:Scalar items where the seeder placed
# tagged cst:Mapping items. Treating `any` as a non-scalar yields
# `seq_mapping_nodes`, which matches what the cascade sees.


def load(name):
    with open(CAT / name, "rb") as f:
        return tomllib.load(f)


def top_split(t, sep):
    """Split `t` on `sep`, but only at angle-bracket depth 0 — so
    `list<a | b> | c` splits into `list<a | b>` and `c`."""
    parts, depth, cur = [], 0, ""
    for ch in t:
        if ch == "<":
            depth += 1
        elif ch == ">":
            depth -= 1
        if ch == sep and depth == 0:
            parts.append(cur.strip())
            cur = ""
        else:
            cur += ch
    parts.append(cur.strip())
    return parts


def is_scalar_type(t):
    """A type string denotes a scalar value."""
    t = t.strip()
    if t in SCALAR_TYPES:
        return True
    arms = top_split(t, "|")
    # a union counts as scalar only if every arm is scalar
    if len(arms) > 1:
        return all(is_scalar_type(a) for a in arms)
    return False


def classify(type_str, field_kind):
    """Pick a rule shape from (platform key type, IR field kind)."""
    t = (type_str or "any").strip()
    ref = field_kind.startswith("ref:")

    if len(top_split(t, "|")) > 1:
        return "union"
    if t.startswith("list<") and t.endswith(">"):
        inner = t[5:-1].strip()
        if not ref:
            return "seq_attr"
        return "seq_scalar_nodes" if is_scalar_type(inner) else "seq_mapping_nodes"
    if is_scalar_type(t):
        return "scalar_node" if ref else "scalar_attr"
    if t.startswith("map<"):
        return "map_nodes"
    # a bare construct name / object / mapping
    return "mapping_node" if ref else "block_attr"


# (platform, "<construct>.<field>", source-key) -> (shape, type). See the
# inline comment at the lookup site for why each entry exists.
SHAPE_OVERRIDES = {
    ("azure", "job.name", "job"): ("scalar_attr", "string"),
    ("azure", "job.name", "deployment"): ("scalar_attr", "string"),
}


def main():
    ir = load("ir.toml")
    hub = load("hub_schema.toml")
    targets = [t for t in load("targets.toml") if t != "meta"]

    # IR field kinds: node.<c>.fields.<f> -> "scalar" | "ref:hub:x"
    field_kind = {}
    for cname, node in hub.get("node", {}).items():
        for fname, k in node.get("fields", {}).items():
            field_kind[(cname, fname)] = k

    out_dir = CAT / "rules"
    out_dir.mkdir(exist_ok=True)
    grand = 0
    summary = []

    for plat in targets:
        inv_path = CAT / f"{plat}.toml"
        if not inv_path.exists():
            continue
        inv = load(f"{plat}.toml")
        # Per-section key types AND a global fallback. The type of a
        # platform key depends on WHICH platform construct (section) it
        # sits in: `parameters` is `object` under [extends] but
        # `list<pipelineTemplateParameter>` under [pipelineBase]. A global
        # key→type map keyed only by name (first-wins) picked [extends]'s
        # `object` for pipeline.parameters → a single mapping_node rule that
        # never matched the real LIST form → the parameters list was lost
        # backward. Resolve each IR field's type from the platform sections
        # the IR construct actually maps to ([<ir>.maps].<platform>), and
        # only fall back to the global map when the key appears in none.
        section_key_type = {}
        key_type = {}
        for sec, body in inv.items():
            if sec == "meta" or not isinstance(body, dict):
                continue
            st = {}
            for key, spec in body.items():
                if isinstance(spec, dict) and "type" in spec:
                    st[key] = spec["type"]
                    key_type.setdefault(key, spec["type"])
            section_key_type[sec] = st

        rules = []
        for cname, node in ir.items():
            if cname == "meta" or not isinstance(node, dict) or "maps" not in node:
                continue
            if not node.get("maps", {}).get(plat):
                continue
            mapped_sections = node.get("maps", {}).get(plat, [])
            for fname, ftab in node.get("field", {}).items():
                if not isinstance(ftab, dict):
                    continue
                fk = field_kind.get((cname, fname), "scalar")
                seen = set()
                for key in ftab.get(plat, []):
                    if key in seen:
                        continue
                    seen.add(key)
                    # Prefer the type from a platform section this IR
                    # construct maps to; fall back to the global map.
                    t = None
                    for sec in mapped_sections:
                        if key in section_key_type.get(sec, {}):
                            t = section_key_type[sec][key]
                            break
                    if t is None:
                        t = key_type.get(key, "any")
                    shape = classify(t, fk)
                    # Per-platform shape overrides for DISCRIMINATOR keys the
                    # catalog cannot type: azure `- job: <name>` / `- deployment:
                    # <name>` — the key collides with the construct name, no
                    # section declares it, so the lookup yields `any` →
                    # block_attr, which expects a mapping and silently drops the
                    # scalar name (F3). The truth is a plain string. Scoped per
                    # (platform, to, from) so the SHARED job.field.name of other
                    # platforms is untouched (forcing the shared field scalar
                    # regressed 6 platforms — see docs/gaps-backlog.md F3).
                    ovr = SHAPE_OVERRIDES.get((plat, f"{cname}.{fname}", key))
                    if ovr is not None:
                        shape, t = ovr
                    rules.append((f"{cname}.{fname}", key, shape, t))

        # Keep the catalog's declared key order (ir.toml lists the
        # canonical/primary key first per (construct, field)) — the
        # reverse-cascade dedupe picks the first rule per IR field,
        # so the primary key must lead.
        rules.sort(key=lambda r: r[0])  # stable: only by `to`, preserves original `from` order
        lines = [
            f"# catalog/rules/{plat}.toml — generated TGG rule manifest.",
            "# Each [[rule]] maps one platform key to one IR field;",
            "# `shape` is derived from the key type + IR field kind.",
            "",
            "[meta]",
            f'platform = "{plat}"',
            f"rule_count = {len(rules)}",
            "",
        ]
        for to, frm, shape, t in rules:
            lines.append("[[rule]]")
            lines.append(f'to = "{to}"')
            lines.append(f'from = "{frm}"')
            lines.append(f'shape = "{shape}"')
            lines.append(f'type = "{t}"')
            lines.append("")
        (out_dir / f"{plat}.toml").write_text("\n".join(lines))
        grand += len(rules)
        summary.append((plat, len(rules)))

    for plat, n in summary:
        print(f"  {plat:20s} {n:4d} rules")
    print(f"  {'TOTAL':20s} {grand:4d} rules across {len(summary)} platforms")


if __name__ == "__main__":
    main()
