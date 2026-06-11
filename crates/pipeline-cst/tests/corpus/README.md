# pipeline-cst test corpus

Total: 32 files (27 synthetic fixtures + 5 anonymized real pipelines).

## Synthetic fixtures (`fixtures/`)

| File | Tests |
|------|-------|
| 01_single_job.yml | Trivial: 1 job, 1 step |
| 02_two_jobs_no_needs.yml | Stage-implicit ordering |
| 03_with_comments.yml | Comments before/between/after |
| 04_with_anchors.yml | Anchors + aliases + merge keys |
| 05_complex_indentation.yml | Block scalars (`\|`, `>`), nested mappings |
| 06_empty_file.yml | Zero bytes |
| 07_comments_only.yml | File with only comments, no structure |
| 08_only_stages.yml | Only top-level `stages:` list |
| 09_variables_block.yml | `variables:` mapping |
| 10_double_quoted_strings.yml | Double-quoted scalars, embedded `:` |
| 11_single_quoted_strings.yml | Single-quoted with `''` escape |
| 12_block_literal.yml | `\|` style |
| 13_block_folded.yml | `>` style |
| 14_needs_explicit.yml | Sequence-form `needs:` |
| 15_rules_if.yml | Mapping-list `rules:` with `if:` + `when:` |
| 16_when_manual.yml | `when: manual` + `allow_failure: false` |
| 17_cache_block.yml | `cache:` with `key:`, `paths:`, `policy:` |
| 18_artifacts.yml | `artifacts:` with `reports:` sub-mapping |
| 19_services_block.yml | `services:` with name+alias mapping and bare string |
| 20_parallel_matrix.yml | `parallel: matrix:` with flow-lists |
| 21_extends.yml | `extends:` scalar and list forms |
| 22_include_local.yml | `include:` mapping-list |
| 23_reference_tag.yml | `!reference [path, segments]` |
| 24_retry_timeout.yml | `retry:` mapping with `when:` list |
| 25_environment.yml | `environment:` with `on_stop:` |
| 26_workflow_rules.yml | Top-level `workflow:rules:` |
| 27_child_pipeline.yml | `trigger:` with `include:` and `project:` |
| 28_tags_runner.yml | `tags:` sequence |
| 29_default_block.yml | Top-level `default:` |
| 30_deeply_nested.yml | 4-level deep mapping |
| 31_trailing_whitespace.yml | Lines with trailing whitespace (must survive round-trip) |
| 32_no_trailing_newline.yml | File ends mid-line (no `\n` at end) |

## Real (`real/`)

| File | Source | Anonymization |
|------|--------|---------------|
| 01_rust_workspace.yml | Internal Rust monorepo CI | hostnames, crate names |
| 02_godot_game.yml | Internal Godot game project | hostnames, asset names |
| 03_flutter_app.yml | BergparkFlutter / bergpark (own project) | trivial pipeline, no anonymization needed |

All real files: self-owned by the user. Anonymization documented above.
