# User manual

Complete reference for the Qt6 desktop application and the `pipewright`
CLI. For background concepts (Hub-IR, TGG, capabilities, friction
reports) see **[About](about.md)**; to build the binaries see
**[Install](install.md)**.

## Who is this manual for?

Pipewright serves several distinct roles. Find yours, then jump to
the sections that matter for it:

| You are… | Your task | Read |
|---|---|---|
| **Migration engineer** | "We're moving from platform A to platform B — what do we have, what survives, what needs manual work?" | [Capabilities tab](#capabilities-tab), [Migrate tab](#migrate-tab), [`pipewright migrate`](#pipeline-migrate) |
| **Developer joining a project** | "What does this pipeline actually do?" | [Loading a pipeline](#loading-a-pipeline), [Runbook tab](#runbook-tab), [DAG tab](#dag-tab) |
| **Pipeline author** | Build or change a pipeline — visually, from recipes, or in source | [Source tab](#source-tab), [DAG tab](#dag-tab), [Recipes tab](#recipes-tab) |
| **Local tester** | "Will this run on my laptop? Run it." | [Run tab](#run-tab), [`pipewright plan` / `run`](#pipeline-plan) |
| **Automation / CI engineer** | Script everything headlessly | [CLI reference](#cli-reference) |
| **Embedder** | Call the engine from another language | [C-ABI](#c-abi-for-embedders) |

All functions operate on the same engine: the source file is parsed
to a concrete syntax tree, lifted by TGG rules into the neutral
**Hub-IR**, and every view, migration, and export is a projection of
that one representation. 17 platforms are supported end-to-end:

> argo, aws_codebuild, aws_codepipeline, azure, bitbucket, buildkite,
> circleci, dagger, drone, earthly, github, gitlab,
> google_cloudbuild, jenkins, tekton, travis, woodpecker

### Inspect/translate vs. run locally

All 17 platforms are supported for **inspect, render, document, and
migrate**. **Local execution** (the Run tab / `pipewright run`) is a
narrower claim, and the tool is honest about it:

- **Runs locally (11)** — the container-shell platforms: gitlab,
  github, drone, woodpecker, bitbucket, circleci, azure, travis,
  aws_codebuild, google_cloudbuild, buildkite. Their jobs declare an
  image and shell commands, so a local Docker run reproduces them.
- **Translate-only (6)** — argo & tekton (Kubernetes CRDs that
  reference external task definitions), aws_codepipeline (orchestrates
  cloud-service actions), dagger (pipelines defined in SDK code),
  earthly (its own BuildKit engine), jenkins (server runtime). There's
  no local shell script to run; the Run tab disables itself with a
  reason and `pipewright run` refuses with a pointer to inspect/migrate.

---

## The desktop app

Launch `pipewright-ui`. The window is: a top bar (file, platform,
language), a **jobs list** on the left, and seven tabs on the right —
ordered along the typical workflow: *understand* (Source, DAG,
Capabilities), *transform* (Migrate, Recipes), *execute* (Run),
*document* (Runbook).

### Loading a pipeline

Four ways, all equivalent:

1. **File → Open…** (⌘O). The native macOS panel hides dotfiles like
   `.gitlab-ci.yml` — press **⇧⌘.** in the panel to show them.
2. **Paste a full path** into the top-bar field and press Enter —
   handy when the path is already in your clipboard (tab-complete it
   in a terminal first).
3. **Drag & drop** a file from Finder anywhere onto the window.
4. **Paste source text** directly into the Source tab.

The platform is **auto-detected** from distinctive markers and shown
in the top-bar *Platform* combo. If detection guesses wrong (some
YAML dialects overlap), override it there — every view re-renders,
and the override carries through to local runs.

The **Language** combo (en/de) switches the whole app at once: UI
chrome *and* all generated prose (Runbook, recipe descriptions).

### The jobs list

Every job the engine lifted from the source, with stage, step count
and dependencies. Two interactions:

- **Click** — selects the job and jumps the Source tab to its
  definition (byte-accurate, from CST provenance).
- **Double-click** — offers to run just that job locally in Docker
  (a confirmation dialog opens; the Run tab streams the output).

### Source tab

The raw source with syntax highlighting. **Editable**: any change
re-parses and re-renders every other tab live. The header shows the
loaded file path.

### DAG tab

The pipeline as a diagram: stage swimlanes, one UML-style node per
job (header, parameters, steps), bezier dependency edges.

This is also an **editor**:

- **Double-click any value** (a parameter like `image`, or a step
  command) to edit it in place. The edit is applied through the TGG
  backward cascade, so the *source text* is rewritten minimally and
  every view updates. Escape cancels.
- **Click a node** to select it → a floating toolbar offers
  **Duplicate** and **Delete** (both structural edits, same
  source-rewriting mechanism).
- **+ Recipe…** opens a searchable picker; the chosen recipe's jobs
  are merged into the current pipeline (namespaced, collision-free).

### Capabilities tab

A feature profile of the loaded pipeline, derived from the Hub-IR:
which constructs it uses (services, caching, conditions, variables,
retry, …) and how often. Each feature is tagged
**universal** (every platform can express it) or **caveat** (some
targets approximate or lack it). The headline verdict:

- **Possible** — only universal constructs; migrates cleanly anywhere.
- **PossibleWithCaveats** — uses features that aren't universal;
  check the Migrate tab's friction report for the specific target.

### Migrate tab

Three panes: source, target, friction report. Pick any of the 17
platforms in *Migrate to* — the translation runs immediately through
the Hub-IR (re-keying the neutral graph into the target's vocabulary,
or, for migrations across structural families like job-based →
step-flat, synthesising the target from the lifted model).

The **friction report** lists everything that didn't translate 1:1.
It is *derived*, not declared: the migrated output is re-parsed and
its capability families are compared against the source's, so a
dropped `cache` or `services` shows up as a real report line rather
than vanishing silently.

| Severity | Meaning |
|---|---|
| ℹ **info** | translated 1:1, surfaced for transparency |
| ≈ **approximated** | partially survived — review the rest |
| ✋ **manual** | not represented in the target — rewrite by hand |

On the CLI the same report prints to stderr (the YAML stays on
stdout, pipeable). **Save target as…** suggests the target platform's
conventional file name (`.drone.yml`, `azure-pipelines.yml`,
`Earthfile`, …).

### Recipes tab

Reusable pipeline building blocks. The **standard library** ships
built-in (`rust-ci`, `go-ci`, `python-ci`, `node-test`,
`docker-publish`); add your own via a sources config (see
[Recipe sources](#recipe-sources)).

- **Browse** with search and sort; the detail pane shows the curated
  documentation, declared input/output ports, and a **generated,
  localized description** of what the recipe's jobs actually do.
- **Apply to current pipeline** merges the focused recipe's jobs into
  the loaded pipeline (namespaced — existing jobs are never
  clobbered).
- **Compose** a standalone pipeline: double-click recipes to add them
  to the composition list, reorder with ▲▼, pick a *Target* platform,
  and **Save…** the emitted YAML.
- **↻ Reload** re-pulls configured git sources.

### Run tab

Executes the loaded pipeline **locally in Docker** via the `pipewright`
CLI — a faithful local run, not a toy. Available for the **11
container-shell platforms** (gitlab, github, drone, woodpecker,
bitbucket, circleci, azure, travis, aws_codebuild, google_cloudbuild,
buildkite); for the translate-only platforms (argo, tekton,
aws_codepipeline, dagger, jenkins, earthly — see below) the Run tab is
disabled with a reason. For a runnable platform:

- The repository (the pipeline file's directory) is **bind-mounted**
  into every job at `/workspace`, so commands run against your real
  code; artifacts written by one job are seen by the next.
- Job **variables/env** are passed into the container.
- **Conditions are evaluated**: a job whose `rules:if` doesn't match
  the trigger context is *skipped* (with a reason); `when: manual`
  jobs don't run automatically. A condition the evaluator can't read
  with confidence runs anyway, flagged — never silently dropped.
- **Service containers** (`services:`) start as sidecars on a shared
  network, reachable by hostname (a `postgres:16` service is reachable
  at `postgres`), and are torn down after the job.

Controls: **Job filter** (run a single job — the job-list
double-click pre-fills it; an explicitly named job runs
unconditionally), **Event / Ref** (the trigger context that drives
condition evaluation), **Stop**.

The CLI binary is discovered automatically (PATH, the repo's
`target/`, `~/.cargo/bin`, Homebrew); set an explicit path in
**Edit → Settings → CLI** if discovery fails. A non-default Docker
socket can be set in **Settings → Local runner** (passed to the
runner as `DOCKER_HOST`).

### Runbook tab

A human-readable description of the pipeline — what it does, job by
job, in dependency order — generated from the Hub-IR in the app
language (en/de). The table of contents on the left jumps to
sections; selecting a job elsewhere scrolls the runbook to it.

**Export** as Markdown, HTML, or Word (RTF) — e.g. to hand a
pipeline description to a reviewer or attach it to a ticket.

### Settings (Edit → Settings…)

| Group | Setting | Effect |
|---|---|---|
| CLI | pipeline-cli path | explicit `pipewright` binary for the Run tab |
| CLI | Cache dir | where git recipe sources are cloned |
| Recipes | Sources config | YAML listing extra recipe sources |
| Local runner | Docker host | `DOCKER_HOST` for local runs (empty = default socket; Docker Desktop's user socket is found automatically) |

Settings persist per-user (macOS: `~/Library/Preferences/`, Linux:
`$XDG_CONFIG_HOME`). No secrets are ever stored.

---

## CLI reference

One binary, `pipewright`, thirteen subcommands. Everything the UI does
(except the interactive editing) is scriptable. All commands
auto-detect the platform; `-p/--platform <KEY>` overrides.

### `pipewright detect`

```bash
pipewright detect .gitlab-ci.yml      # → gitlab
```

### `pipewright platforms`

Lists the 17 supported platform keys (the values `-p` and `--to`
accept).

### `pipewright inspect`

Structured JSON of the lifted pipeline — jobs with names, stages,
dependencies, parameters, steps and byte offsets:

```bash
pipewright inspect .gitlab-ci.yml | jq '.pipeline.jobs[].name'
```

### `pipewright render`

One command, five projections:

```bash
pipewright render ci.yml --format text            # runbook prose (default)
pipewright render ci.yml --format md              # runbook as Markdown
pipewright render ci.yml --format html            # runbook JSON (overview/toc/jobs)
pipewright render ci.yml --format svg > dag.svg   # the diagram
pipewright render ci.yml --format json            # same as inspect
pipewright render ci.yml --format text --locale de  # German prose
```

### `pipewright capabilities`

The portability profile (same data as the Capabilities tab):

```bash
pipewright capabilities ci.yml
```

### `pipewright migrate`

```bash
pipewright migrate .gitlab-ci.yml --to github > workflow.yml
pipewright migrate .drone.yml --to azure          # cross-family works too
```

### `pipewright compose` / `apply`

Recipe composition. `compose` builds a standalone pipeline from
recipe files; `apply` merges one recipe into an existing pipeline
(or starts fresh):

```bash
pipewright compose --to gitlab recipes/rust-ci.recipe.yml > .gitlab-ci.yml
pipewright apply recipes/docker-publish.recipe.yml --into .gitlab-ci.yml
```

### `pipewright recipes` / `describe`

```bash
pipewright recipes                      # list the library (id, version, source, description)
pipewright recipes --query docker       # filter
pipewright describe rust-ci --locale de # generated localized description
```

### `pipewright plan`

What `run` *would* do — jobs in dependency order with images and
commands, no Docker required:

```bash
pipewright plan ci.yml
```

### `pipewright run`

Execute locally in Docker (daemon must be reachable; honors
`DOCKER_HOST`, falls back to Docker Desktop's user socket). Works for
the **11 container-shell platforms**; a translate-only platform
(argo/tekton/dagger/jenkins/earthly/aws_codepipeline) is refused with a
reason and a pointer to `inspect`/`migrate` (and `plan` prepends the
same note). The repository is mounted at `/workspace`, env is passed
through, `rules:if`/`when` are evaluated against `--trigger`/`--ref`,
and `services:` start as network-linked sidecars:

```bash
pipewright run ci.yml                          # all jobs that trigger, in order
pipewright run ci.yml --job test               # one job, unconditionally
pipewright run ci.yml --trigger push --ref main   # deploy@main runs
pipewright run ci.yml --trigger push --ref dev    # …skips on dev
```

**Source access (safe by default).** A pipeline you didn't write is
untrusted code — its commands run with your Docker permissions. So the
workspace is mounted **read-only by default**: a job that tries to write
fails, and nothing on disk can be modified. Opt into writes explicitly:

```bash
pipewright run ci.yml              # read-only (default) — safest
pipewright run ci.yml --rw-copy    # read-write on a throwaway COPY — builds
                                 #   that write work; the real dir is untouched
pipewright run ci.yml --rw         # read-write IN PLACE — only for pipelines
                                 #   you trust; commands can modify your files
```

In the desktop UI the same choice is a “Source access” dropdown, and
picking read-write-in-place asks for confirmation first.

Jobs stream their output live; the first failing job stops the run.
Skipped jobs print the reason; a condition the evaluator can't read
runs anyway with a note (never silently dropped).

---

## Recipes

A recipe is a YAML file describing a reusable group of jobs:

```yaml
recipe_id: rust-ci
recipe_version: 1.0.0
description: lint, test and release-build a Rust crate
tags: [rust, ci, lint, test, build]
doc: |
  Markdown documentation shown in the UI detail pane.
input_ports:                # what the recipe consumes / produces —
  - name: source            # shown in pickers so composition is
    kind: checkout          # port-aware
output_ports:
  - name: binary
    kind: build-artifact
platform_requirements: [docker]
jobs:
  lint:
    image: rust:1.75
    steps:
      - cargo fmt --check
      - cargo clippy -- -D warnings
```

Applying a recipe merges its jobs **namespaced** into the target
pipeline, so repeated applies and multi-recipe compositions never
collide; order is preserved.

### Recipe sources

The standard library is built in. Extra sources are listed in a
config file (UI: *Settings → Recipes*; see
[`recipes-sources.example.yml`](../../recipes-sources.example.yml)):

```yaml
sources:
  - label: my-team
    dir: /home/me/ci-recipes
  - label: community
    git: https://github.com/example/ci-recipes.git
    reference: main
```

Git sources are cloned into the cache dir and re-pulled on
**↻ Reload**. User sources are additive; a later source can shadow a
standard recipe by reusing its `recipe_id`.

---

## C-ABI for embedders

`libpipeline_ffi` exposes the engine as a stable C interface —
every function takes/returns UTF-8 JSON strings (header:
[`crates/pipeline-ffi/include/pipeline_ffi.h`](../../crates/pipeline-ffi/include/pipeline_ffi.h),
regenerated with cbindgen). The Qt UI consumes exactly this ABI, so
it is exercised end-to-end. Entry points cover detect / inspect /
render (SVG + layout, runbook) / capabilities / migrate / recipes
(list, describe, apply, compose) / structural edits (edit field,
duplicate, remove) / runbook export. Memory rule: every returned
`PipelineString*` is freed with `pipeline_string_free`.

---

## Troubleshooting

| Symptom | Cause / fix |
|---|---|
| Open dialog doesn't show `.gitlab-ci.yml` | macOS hides dotfiles — press **⇧⌘.** in the panel, or paste the path into the top-bar field, or drag & drop the file |
| Wrong platform detected | Rare — detection prefers the file name (`.gitlab-ci.yml`, `Jenkinsfile`, …) over content. For an oddly-named file, override in the top-bar Platform combo (the override also applies to local runs) |
| Run tab: "couldn't find the `pipewright` binary" | Build it (`cargo build --release -p pipeline-cli`) and/or set the path in Edit → Settings → CLI |
| Run fails to reach Docker | Start the daemon; for a non-standard socket set Settings → Local runner → Docker host |
| Job shows few/odd parameters after migration | Check the friction report — `approximated`/`manual` items list exactly what didn't translate 1:1 |
| Recipe list shows a warning banner | A configured source failed (bad path / unreachable git URL); the remaining sources still load |
