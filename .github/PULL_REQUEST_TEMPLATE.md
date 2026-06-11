<!-- Thanks for contributing! Keep this short. -->

## What & why

<!-- One or two sentences: what this changes and the reason. -->

## Checklist

- [ ] `cargo fmt --all --check` and `cargo clippy --workspace -- -D warnings` pass
- [ ] `cargo test --workspace` passes
- [ ] Round-trip still green if I touched a platform: `gate_sample` (or the full gates)
- [ ] I edited the **catalog**, not the generated `catalog/rules/*.ruleset.json`, for any rule change
- [ ] Added/updated a fixture under `tests/` for new behaviour
- [ ] Updated docs / CHANGELOG if user-facing
