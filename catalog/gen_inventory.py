#!/usr/bin/env python3
"""Generate a construct-inventory TOML from a CI/CD JSON-Schema.

The inventory is the raw material for the cross-platform
normalization that defines the IR. One [section] per construct (the
root, named `pipeline`, plus every object-typed schema definition);
each property carries its type, enum options and required-ness. A
type rendered as a bare construct name is a nesting into that
section — that is how the inventory captures dependencies.

This is a design-time tool: its output (catalog/<id>.toml) is the
committed artifact. CI/CD schemas vary widely in shape, so the
generator handles JSON-pointer `$ref`, `allOf` composition, and
three definition homes (`definitions`, `$defs`,
`components/schemas`). Union-typed constructs (`oneOf` of
string-or-object) are only partially mined — those inventories are
finished by hand.

Usage:
  gen_inventory.py <schema.json> <platform-id> <config-file> <tier>
"""
import json
import re
import sys


def toml_str(s):
    return '"' + str(s).replace("\\", "\\\\").replace('"', '\\"') + '"'


def toml_key(k):
    if re.fullmatch(r"[A-Za-z0-9_-]+", k):
        return k
    return '"' + k.replace("\\", "\\\\").replace('"', '\\"') + '"'


def main():
    if len(sys.argv) != 5:
        sys.exit(__doc__)
    schema_path, pid, cfg, tier = sys.argv[1:5]
    schema = json.load(open(schema_path))

    def deref(node):
        """Follow a JSON-pointer `$ref` chain to the pointed-at node."""
        seen = set()
        while isinstance(node, dict) and "$ref" in node:
            ref = node["$ref"]
            if ref in seen or not ref.startswith("#/"):
                return {}
            seen.add(ref)
            target = schema
            for part in ref[2:].split("/"):
                part = part.replace("~1", "/").replace("~0", "~")
                target = target.get(part, {}) if isinstance(target, dict) else {}
            node = target
        return node if isinstance(node, dict) else {}

    def resolved(node, depth=0):
        """Deref, then merge `allOf` members into one object view."""
        node = deref(node)
        if "allOf" in node and depth < 8:
            merged = {k: v for k, v in node.items() if k != "allOf"}
            props = dict(merged.get("properties", {}) or {})
            required = list(merged.get("required", []) or [])
            for sub in node["allOf"]:
                sr = resolved(sub, depth + 1)
                props.update(sr.get("properties", {}) or {})
                required += sr.get("required", []) or []
            merged["properties"] = props
            merged["required"] = required
            if props:
                merged.setdefault("type", "object")
            return merged
        return node

    def ref_name(node):
        """Section name for a `$ref` to an object construct, else None."""
        if isinstance(node, dict) and "$ref" in node:
            name = node["$ref"].split("/")[-1]
            if "properties" in resolved(node):
                return name
        return None

    def render_type(node, depth=0):
        if not isinstance(node, dict) or depth > 6:
            return "any"
        rn = ref_name(node)
        if rn:
            return rn
        r = resolved(node)
        if "enum" in r:
            return "enum"
        for combiner in ("oneOf", "anyOf"):
            if combiner in r:
                forms = sorted({render_type(f, depth + 1) for f in r[combiner]})
                forms = [f for f in forms if f != "any"]
                return " | ".join(forms) if forms else "any"
        t = r.get("type")
        if isinstance(t, list):
            t = [x for x in t if x != "null"]
            t = t[0] if len(t) == 1 else " | ".join(t)
        if t == "array":
            return "list<" + render_type(r.get("items", {}), depth + 1) + ">"
        if t == "object":
            ap = r.get("additionalProperties")
            if isinstance(ap, dict):
                return "map<" + render_type(ap, depth + 1) + ">"
            return "object"
        return t or "any"

    def options(node):
        r = resolved(node)
        if "enum" in r:
            return [v for v in r["enum"] if v is not None]
        return None

    # Definition homes: classic `definitions`, draft-2019 `$defs`,
    # and OpenAPI-style `components/schemas`.
    def_homes = [
        schema.get("definitions", {}),
        schema.get("$defs", {}),
        schema.get("components", {}).get("schemas", {})
        if isinstance(schema.get("components"), dict)
        else {},
    ]
    constructs = {"pipeline": schema}
    for home in def_homes:
        if not isinstance(home, dict):
            continue
        for name, d in home.items():
            if "properties" in resolved(d):
                constructs.setdefault(name, d)

    out = [
        f"# catalog/{pid}.toml — construct inventory, generated from the JSON-Schema.",
        "# One [section] per construct; each key carries type / options / required.",
        "# A type that is a bare construct name is a nesting into that section.",
        "",
        "[meta]",
        f"platform = {toml_str(pid)}",
        f"config_file = {toml_str(cfg)}",
        f"tier = {tier}",
        'source = "json-schema"',
        "",
    ]

    emitted = 0
    for cname in sorted(constructs):
        node = resolved(constructs[cname])
        props = node.get("properties", {})
        if not isinstance(props, dict) or not props:
            continue
        emitted += 1
        required = set(node.get("required", []) or [])
        out.append(f"[{toml_key(cname)}]")
        for key in sorted(props):
            spec = props[key]
            parts = [f"type = {toml_str(render_type(spec))}"]
            opts = options(spec)
            if opts:
                parts.append("options = [" + ", ".join(toml_str(o) for o in opts) + "]")
            if key in required:
                parts.append("required = true")
            out.append(f"{toml_key(key)} = {{ {', '.join(parts)} }}")
        out.append("")

    sys.stderr.write(f"{pid}: {emitted} constructs\n")
    sys.stdout.write("\n".join(out))


if __name__ == "__main__":
    main()
