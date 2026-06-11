# Contributing

Thanks for your interest in Pipewright. This guide covers the local
workflow and what CI expects.

## Prerequisites

- Rust **1.88** (the MSRV — see `rust-toolchain.toml`).
- For the local-runner tests and `pipewright run`: a reachable Docker
  daemon.
- For the desktop UI: Qt 6 + CMake (see `ui/qt6/README.md`).

## Build & test

```bash
cargo build --workspace
cargo test  --workspace          # the fast suite (~300 tests)
```

The heavy verification gates are `#[ignore]` (slow); run them explicitly:

```bash
# Fast PR slice — 3 chaos seeds/platform + corpus fixtures (~1 min):
cargo test -p chaos-generator --test roundtrip gate_sample -- --ignored --nocapture

# Full gates (minutes each):
cargo test -p chaos-generator --test roundtrip real_config_corpus_roundtrip -- --ignored --nocapture
cargo test -p chaos-generator --test roundtrip roundtrip_wide_stress       -- --ignored --nocapture
cargo test -p chaos-generator --test roundtrip interop_matrix              -- --ignored --nocapture

# Real Docker end-to-end for `pipewright run`:
cargo test -p pipeline-cli --test e2e_run -- --ignored
```

## Before you open a PR

CI enforces all of these — run them locally first:

```bash
cargo fmt --all --check
cargo clippy --workspace -- -D warnings
cargo test --workspace
cargo deny check licenses bans sources      # supply-chain
```

## The rule sets are generated — don't hand-edit them

The per-platform TGG rule sets in `catalog/rules/*.ruleset.json` are
**generated** from the declarative catalog. If you change platform
behaviour, edit the catalog and regenerate, never the JSON directly:

```bash
cd catalog
python3 gen_hub_schema.py     # if you touched the hub schema
python3 gen_rules.py
python3 gen_ruleset.py
```

A platform's mapping lives in `catalog/concepts.toml` and `catalog/ir.toml`;
`docs/user/about.md` explains the forward → Hub-IR → emit pipeline.

## Adding or fixing a platform

1. Add a real fixture under `tests/cross_corpus/<platform>/`.
2. Express the mapping in the catalog; regenerate the rule sets.
3. Make the round-trip green: `forward → Hub-IR → emit` must reproduce
   the source with equal IR content (that's what the gates check).
4. If it's a runnable (container-shell) platform, confirm `pipewright run`
   lifts real commands (see `crates/pipeline-ffi/tests/lift_runnable.rs`).

## Robustness & safety invariants

- **Parsers must never panic.** Every public entry point (`detect`,
  `forward`, …) returns `Err`/`None` on malformed input, never an
  unwind. `crates/pipeline-forward/tests/no_panic.rs` enforces this with
  proptest (random + YAML-shaped noise across all platforms) plus a set
  of pathological inputs. If you add a parser path, it stays panic-free.
- **`panic = "unwind"` is required** (the default — don't set
  `panic = "abort"`). The FFI wraps every call in `catch_unwind` to turn
  a panic into an `{"error": …}` payload instead of crossing the C
  boundary; `abort` would defeat that and take down the whole host
  process (including the Qt UI).

## Style

Match the surrounding code. Comments explain *why*, not *what*. The CI's
`clippy -D warnings` is the bar; load-bearing functions that trip a
pedantic lint use `#[allow(..., reason = "...")]` rather than being split
artificially.
