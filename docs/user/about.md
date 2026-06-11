# About Pipewright

## The problem

CI/CD configuration files have grown into substantial pieces of
software. They include other files, inherit from anchors, expand via
matrices, gate on rule expressions, define their own DSLs for
conditions and dependencies, and ultimately produce a graph of jobs
the platform runs. Yet the tooling we have to work with these files
is mostly platform-specific linting, and the moment you want to
migrate, run locally, or understand the pipeline a config will
produce, you are on your own.

Pipewright makes CI/CD configurations **first-class program text**:

- Parse the source as a real CST (with spans, comments, and
  byte-accurate round-trip reconstruction).
- Lift it into a platform-neutral intermediate representation
  (Hub-IR) by rule-based transformation, not ad-hoc string
  manipulation.
- Operate on the Hub-IR for everything else — inspecting, rendering,
  running, documenting, migrating, composing.
- Emit it back into any supported platform.

The result: one program understands **17 CI/CD platforms** well
enough to translate between them — all 272 ordered platform pairs
losslessly for what both sides can represent — while telling you
exactly where a translation is approximate.

> argo, aws_codebuild, aws_codepipeline, azure, bitbucket, buildkite,
> circleci, dagger, drone, earthly, github, gitlab,
> google_cloudbuild, jenkins, tekton, travis, woodpecker

## Why a Triple Graph Grammar?

The kernel is [`seesaw-tgg`](https://crates.io/crates/seesaw-tgg), a
delta-extended TGG engine. A Triple Graph Grammar specifies a *triple*
— source graph, target graph, correspondence graph — and rules that
extend all three consistently. From that single specification you get:

- **Forward**: source CST → Hub-IR
- **Backward**: Hub-IR → emitted source text
- **Synchronisation**: edit one side, propagate the consistent
  change to the other — this is what powers the UI's in-place DAG
  editing (change a value in the diagram, the YAML is rewritten
  minimally).

Each rule is declared **once** and compiled into both directions
(`compile_bidirectional`), so parsing and emitting can never drift
apart. The cost is upfront — a rule set is more work than a one-shot
parser — and the payoff is round-trip safety and an explicit audit
trail for every transformation.

If you'd like the theory: TGG papers by Schürr/Königs/Anjorin
(2008–2018) cover the foundation; `seesaw-tgg` adds rank-based
selection, delta tracking, and incremental matching.

## The construct catalog

The per-platform rule sets are not hand-written: they are **generated
from a declarative catalog** (`catalog/`). For each platform a TOML
inventory describes its constructs and field types (largely derived
from the platforms' published JSON schemas); `ir.toml` maps every
platform key onto a neutral IR field; generators derive the
bidirectional TGG rule sets from that mapping. Adding or sharpening a
platform is catalog work, not parser work.

## Hub-IR — one neutral graph

The Hub-IR is a typed graph, not a fixed struct: `hub:pipeline`,
`hub:job`, `hub:step` and friends carry their fields as attribute
satellites, collections, and typed edges. Two properties matter for
users:

- **Provenance**: every node keeps the byte span it was lifted from,
  so the UI can jump from any rendered element straight to its
  definition in the source — and apply edits back through the same
  correspondence.
- **Neutrality**: platforms with different shapes (GitLab's top-level
  job keys, drone's flat step lists, bitbucket's event-selector maps,
  Jenkins' Groovy DSL, Earthly's Earthfile) all normalise onto the
  same job/step model, which is what makes cross-platform migration
  structural rather than textual.

## Verification — how we know it round-trips

Three heavy gates run continuously (and in CI on the `gates` stage):

| Gate | What it proves |
|---|---|
| **Real-config corpus** (59 fixtures) | Hand-curated, real-world configs for all 17 platforms round-trip source → Hub-IR → source with equal IR content |
| **Chaos stress** (17 platforms × 30 seeds) | Schema-driven random pipelines round-trip losslessly — exercises corners no human config hits |
| **Interop matrix** (272 ordered pairs) | Every platform migrates to every other faithfully *and stably* (`a→b→a'→b'` gives `b' ≡ b`) — see [`docs/interop-matrix.md`](../interop-matrix.md) |

## Capabilities — "how portable is this?"

The capability profile classifies every feature construct a pipeline
uses (services, caching, conditions, variables, retry, artifacts, …)
as **universal** (every platform expresses it) or **caveat**
(some targets approximate or lack it). The pipeline's overall verdict
is **Possible** (only universal constructs) or
**PossibleWithCaveats**. It's a *source-side* portability hint; the
target-specific truth is the friction report.

## Friction reports

Migration is rarely 100% lossless across feature sets. Pipewright is
honest about it: every migration produces a structured report.

- **info** — translated 1:1, surfaced for transparency.
- **approximated** — translated, but the target's semantics aren't
  exactly the source's; review.
- **manual** — no automatic mapping; rewrite by hand.

## Why "Pipewright"?

Like a shipwright or a wheelwright, a *pipewright* is someone who
builds and shapes pipelines as a craft — and reshapes them when they
need to move to different material. CI configurations accumulate the
same wear over time: `extends:` chains, dead rule branches,
half-migrated job names, the one job nobody has touched in two years.
Pipewright lets you see what's underneath, decide what to keep, and
translate the rest — losslessly — to a different platform.

## See also

- **[Install](install.md)** — get the binaries on your system
- **[Quickstart](quickstart.md)** — five-minute tour
- **[User manual](manual.md)** — full reference, organised by role
- **[Interop matrix](../interop-matrix.md)** — every platform pair's
  round-trip fidelity
