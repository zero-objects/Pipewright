#!/usr/bin/env python3
"""Verify catalog/ir.toml accounts for every construct AND every key.

Two levels of completeness, both with the same rule — nothing may
be discarded, a CI construct never carries zero information:

  construct level: every [section] of every catalog/<platform>.toml
    must be mapped under some [<ir-construct>.maps].<platform>.

  key level: every key of every mapped section must be claimed by
    some IR field — a `[<ir-construct>.field.<name>]` table lists,
    per platform, the source keys that feed that field.

A section or key that is unaccounted is a GAP — a hole the IR
would paper over with `opaque`. A name in ir.toml not present in
the inventory is a GHOST (typo).

Usage:
  check_ir_coverage.py              full check, exit non-zero on gap
  check_ir_coverage.py <construct>  dump that construct's keys with
                                    per-key mapped/UNMAPPED status
"""
import pathlib
import sys
import tomllib

CAT = pathlib.Path(__file__).parent


def load(name):
    with open(CAT / name, "rb") as f:
        return tomllib.load(f)


def inventories(platforms):
    inv = {}
    for p in platforms:
        path = CAT / f"{p}.toml"
        if path.exists():
            inv[p] = load(f"{p}.toml")
    return inv


def section_keys(inv_p, section):
    """Keys of one construct section in a platform inventory."""
    s = inv_p.get(section, {})
    return {k for k in s if isinstance(s.get(k), dict)} if isinstance(s, dict) else set()


def construct_field_claims(construct):
    """field-name -> {platform -> set(keys)} for one IR construct."""
    out = {}
    for fname, ftab in construct.get("field", {}).items():
        if not isinstance(ftab, dict):
            continue
        out[fname] = {
            plat: set(keys)
            for plat, keys in ftab.items()
            if isinstance(keys, list)
        }
    return out


def main():
    ir = load("ir.toml")
    targets = load("targets.toml")
    platforms = [p for p in targets if p != "meta"]
    inv = inventories(platforms)

    constructs = {
        name: c
        for name, c in ir.items()
        if name != "meta" and isinstance(c, dict) and "maps" in c
    }

    # ── single-construct key dump mode ──────────────────────────
    if len(sys.argv) > 1:
        name = sys.argv[1]
        c = constructs.get(name)
        if c is None:
            sys.exit(f"no such IR construct: {name}")
        claims = construct_field_claims(c)
        print(f"# {name} — key coverage")
        for plat, secs in c.get("maps", {}).items():
            claimed = set()
            for pmap in claims.values():
                claimed |= pmap.get(plat, set())
            for section in secs:
                keys = sorted(section_keys(inv.get(plat, {}), section))
                if not keys:
                    continue
                print(f"\n[{plat}.{section}]")
                for k in keys:
                    mark = "  " if k in claimed else "??"
                    print(f"  {mark} {k}")
        return

    # ── full check ──────────────────────────────────────────────
    mapped = {p: set() for p in platforms}
    for c in constructs.values():
        for plat, secs in c.get("maps", {}).items():
            mapped.setdefault(plat, set()).update(secs)

    sec_gaps = sec_ghosts = key_gaps = 0
    for p in platforms:
        if p not in inv:
            continue
        sections = {k for k in inv[p] if k != "meta"}
        gap = sorted(sections - mapped.get(p, set()))
        ghost = sorted(mapped.get(p, set()) - sections)
        if gap:
            sec_gaps += len(gap)
            print(f"SECTION-GAP   {p}: {', '.join(gap)}")
        if ghost:
            sec_ghosts += len(ghost)
            print(f"SECTION-GHOST {p}: {', '.join(ghost)}")

    # key level — per construct
    for name, c in constructs.items():
        claims = construct_field_claims(c)
        for plat, secs in c.get("maps", {}).items():
            claimed = set()
            for pmap in claims.values():
                claimed |= pmap.get(plat, set())
            for section in secs:
                keys = section_keys(inv.get(plat, {}), section)
                un = sorted(keys - claimed)
                if un:
                    key_gaps += len(un)
                    print(f"KEY-GAP {name} <- {plat}.{section}: {', '.join(un)}")

    print(
        f"\nsection gaps: {sec_gaps}  ghosts: {sec_ghosts}  "
        f"key gaps: {key_gaps}"
    )
    sys.exit(1 if (sec_gaps or sec_ghosts or key_gaps) else 0)


if __name__ == "__main__":
    main()
