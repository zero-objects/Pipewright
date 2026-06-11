# Standard Recipe Library

Reusable, platform-neutral CI/CD **pipeline fragments** for the pipeline
migration tool. A *recipe* is a small set of named jobs — an image, some
shell steps, and intra-recipe dependencies — that you compose into a pipeline
and render to **any** supported target platform. Composition is migration: a
recipe is forwarded to the neutral Hub-IR and re-emitted to your target, so
the same `rust-ci` recipe produces a GitLab `.gitlab-ci.yml`, a GitHub Actions
workflow, an Azure pipeline, and so on.

This directory is the **canonical standard library**. Its files are embedded
in the tool at build time (always available, no on-disk dependency), and it is
also published as a standalone Git repository so it can be cloned and pinned
like any other source.

## Layout

```
recipes/
  README.md            ← this file
  <id>.recipe.yml      ← one recipe per file; the file stem is informational,
                          the recipe_id field is authoritative
```

A recipe file MUST end in `.recipe.yml` — that suffix is how both the embedded
loader and directory sources discover recipes.

## Recipe format

```yaml
recipe_id: rust-ci                 # required — unique id, used everywhere
recipe_version: 1.0.0              # optional — SemVer-ish
description: lint, test and build  # optional — one line, shown in lists/search
tags: [rust, ci, lint, test]       # optional — search/sort facets
doc: |                             # optional — long-form Markdown documentation
  Three sequential jobs for a Rust crate ...
platform_requirements:             # optional — capabilities a target must have
  - docker
input_ports:                       # optional — typed connection points for
  - name: source                   #   graph-edit wiring
    kind: checkout
output_ports:
  - name: binary
    kind: build-artifact
jobs:                              # required — the actual pipeline fragment
  lint:
    image: rust:1.75               # optional
    steps:                         # the shell commands
      - cargo fmt --check
      - cargo clippy -- -D warnings
  test:
    image: rust:1.75
    needs: [lint]                  # intra-recipe dependency (job name)
    steps:
      - cargo test --all
```

Only `recipe_id` and `jobs` are required. Everything else enriches how the
recipe is listed, searched, documented, and wired in the graph editor.

### Descriptions

Each recipe carries **two** kinds of description, which complement each other:

- `description` / `doc` — the curated, human-authored summary and long-form
  Markdown. Write these for intent ("why / when to use this").
- A generated **structural** description, produced on demand through the prose
  doc mechanism (the recipe is forwarded to the Hub-IR and rendered as a
  localized natural-language runbook). This always reflects the actual jobs,
  steps and dependencies, in the reader's language.

## Using this library

The standard library is always loaded — you get these recipes for free.

To add your **own** recipes, configure additional sources (see the tool's
recipe-source configuration). A source is either a local directory of
`*.recipe.yml` files or a Git repository (cloned/updated into a local cache):

```yaml
sources:
  - label: my-team
    dir: /home/me/ci-recipes
  - label: community
    git: https://github.com/example/ci-recipes.git
    reference: main
```

User sources are **additive** and source-labelled; a later source can shadow a
standard recipe by reusing its `recipe_id`.

## Publishing this directory as the standard recipe repo

This directory is self-contained and CC0-licensed (see `LICENSE`), so it can be
pushed to GitHub as the canonical standard library that users point a `git`
source at:

```sh
# from the repo root — publish recipes/ as its own repo history
git subtree split --prefix=recipes -b recipes-standalone
git push git@github.com:<org>/ci-recipes.git recipes-standalone:main
```

Or simply copy `recipes/` into a fresh repository. Every recipe is validated on
each CI run (the `dogfood-recipes` job composes/describes/applies them via the
`pipeline` CLI), so what's published here is known-good. Once pushed, users add
it as a source:

```yaml
sources:
  - label: standard-online
    git: https://github.com/<org>/ci-recipes.git
    reference: main
```

(The same recipes are also embedded in the tool, so this online source is only
needed to pick up updates between tool releases.)

## Contributing a standard recipe

1. Add `<id>.recipe.yml` here, following the format above.
2. Give it a clear `description`, useful `tags`, and a `doc` block.
3. Keep jobs platform-neutral — prefer plain shell `steps` and a container
   `image`; the migration layer maps those to every target.
4. Register it in the embedded list (`crates/pipeline-recipe/src/registry.rs`,
   the `STANDARD` array) so the built tool ships it.
