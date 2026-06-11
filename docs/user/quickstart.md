# Quickstart

A five-minute tour with the CLI, using a small GitLab pipeline. Every
output below is real — reproduce it with the fixture file
[`tests/cross_corpus/gitlab/ci_build_test.yml`](../../tests/cross_corpus/gitlab/ci_build_test.yml)
from the repo, or your own CI file.

## 0. Get the binary on your PATH

```bash
cargo build --release -p pipeline-cli
export PATH="$PWD/target/release:$PATH"
pipewright --help        # prints the 13 subcommands
```

## 1. Detect & inspect — what is this, and what's in it?

```bash
pipewright detect ci.yml
# gitlab

pipewright inspect ci.yml | jq '.pipeline.jobs[].name'
# "test"
# "build"
```

`inspect` returns the full structured model (stages, dependencies,
parameters, steps, byte offsets) as JSON — ready for `jq` or any
script. Auto-detection covers all 17 platforms; force one with
`-p <key>` when a dialect is ambiguous.

## 2. Plan — what would run, in what order?

```bash
pipewright plan ci.yml
```

```text
Execution plan — 2 job(s):

1. build
   image: rust:1.75
   $ cargo build --release

2. test  (after: build)
   image: (none — would default to alpine:latest)
   $ cargo test
```

## 3. Run it locally

```bash
pipewright run ci.yml --job build     # needs a reachable Docker daemon
```

Jobs run in dependency order, each in its declared image, output
streamed live.

## 4. Will it migrate cleanly?

```bash
pipewright capabilities ci.yml | jq '{overall, summary}'
```

```json
{
  "overall": "PossibleWithCaveats",
  "summary": "2 job(s), 2 step(s); 3 non-universal capability families in use."
}
```

`PossibleWithCaveats` means the pipeline uses features not every
platform expresses identically — the per-feature list in the full
output (and the Migrate tab's friction report) names them.

## 5. Migrate

```bash
pipewright migrate ci.yml --to drone
```

```yaml
kind: pipeline
name: pipeline
steps:
  - name: test
    depends_on: [build]
    commands:
      - 'cargo test'
  - name: build
    image: 'rust:1.75'
    commands:
      - 'cargo build --release'
```

That's a **cross-family** migration — GitLab's job-based shape into
drone's flat step list — handled structurally, not by string
templates. All 17×16 platform pairs are exercised by the project's
interop gate.

## 6. Document it

```bash
pipewright render ci.yml --format md --locale de > RUNBOOK.de.md
```

A generated, human-readable runbook (en/de) of what the pipeline
does, job by job.

## 7. Or do all of it visually

Build and launch the desktop app (see [Install](install.md)) and drop
any CI file onto the window: jobs list, editable DAG diagram,
capability profile, migration with friction report, recipe
composition, local runs, exportable runbook — same engine, same
results. The **[User manual](manual.md)** walks every tab, organised
by role.
