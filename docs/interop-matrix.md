# Cross-platform interop matrix

Each cell is the fixpoint trip **row → col → row' → col'** over chaos seeds 1–3.

Legend: `ok` faithful + stable + non-empty · `∅` empty (target can't represent the source) · `≠` drift (b'≠b) · `x` error · `—` diagonal (round-trip, proven separately).

| from\to | dro | wdp | bkt | tkt | arg | gcb | gh | az | acb | acp | trv | gl | bb | cci | jen | ear | dag |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| **dro** | — | ok | ok | ok | ok | ok | ok | ok | ok | ok | ok | ok | ok | ok | ok | ok | ok |
| **wdp** | ok | — | ok | ok | ok | ok | ok | ok | ok | ok | ok | ok | ok | ok | ok | ok | ok |
| **bkt** | ok | ok | — | ok | ok | ok | ok | ok | ok | ok | ok | ok | ok | ok | ok | ok | ok |
| **tkt** | ok | ok | ok | — | ok | ok | ok | ok | ok | ok | ok | ok | ok | ok | ok | ok | ok |
| **arg** | ok | ok | ok | ok | — | ok | ok | ok | ok | ok | ok | ok | ok | ok | ok | ok | ok |
| **gcb** | ok | ok | ok | ok | ok | — | ok | ok | ok | ok | ok | ok | ok | ok | ok | ok | ok |
| **gh** | ok | ok | ok | ok | ok | ok | — | ok | ok | ok | ok | ok | ok | ok | ok | ok | ok |
| **az** | ok | ok | ok | ok | ok | ok | ok | — | ok | ok | ok | ok | ok | ok | ok | ok | ok |
| **acb** | ok | ok | ok | ok | ok | ok | ok | ok | — | ok | ok | ok | ok | ok | ok | ok | ok |
| **acp** | ok | ok | ok | ok | ok | ok | ok | ok | ok | — | ok | ok | ok | ok | ok | ok | ok |
| **trv** | ok | ok | ok | ok | ok | ok | ok | ok | ok | ok | — | ok | ok | ok | ok | ok | ok |
| **gl** | ok | ok | ok | ok | ok | ok | ok | ok | ok | ok | ok | — | ok | ok | ok | ok | ok |
| **bb** | ok | ok | ok | ok | ok | ok | ok | ok | ok | ok | ok | ok | — | ok | ok | ok | ok |
| **cci** | ok | ok | ok | ok | ok | ok | ok | ok | ok | ok | ok | ok | ok | — | ok | ok | ok |
| **jen** | ok | ok | ok | ok | ok | ok | ok | ok | ok | ok | ok | ok | ok | ok | — | ok | ok |
| **ear** | ok | ok | ok | ok | ok | ok | ok | ok | ok | ok | ok | ok | ok | ok | ok | — | ok |
| **dag** | ok | ok | ok | ok | ok | ok | ok | ok | ok | ok | ok | ok | ok | ok | ok | ok | — |

**Summary** (272 ordered pairs): ok=272, ∅=0, ≠=0, x=0

## Findings

- **Complete.** Every one of the 272 ordered cross-platform pairs migrates
  faithfully AND stably (`a→b→a'→b'` ≡ `a→b`): `ok=272`, zero drift, zero empty,
  zero error. The full interop wall — every CI platform losslessly to every
  other through the neutral Hub-IR — stands.

- **How the last earthly cells fell (no heuristics — all model/structure level):**
  1. **VERSION→version** mapping + **auto-emitted job-containment rule** + the
     **Earthfile parser tolerating empty input** + **file-level key dedup** (see
     git log) cleared all `x` and most drift (→ 270).
  2. **Bidirectional job-containment normalization** (`normalize_job_containment`,
     in both the harness `cross_emit` AND `pipeline-forward::migrate`): keyless
     `pipeline --has_job--> job` (gitlab/earthly) and `attr[name=jobs]`+collection
     (github/circleci/travis/…) are the SAME jobs in two shapes — re-shape the hub
     to the TARGET's form (in place, tombstone+add) so jobs cross faithfully
     instead of as bare top-level keys (which travis even mis-reads as a
     toolchain) or being dropped. First-wins on the primary jobs key (circleci
     `jobs` before `job-groups`).
  3. **Content-less jobs are semantically absent** — a `hub:job` with no steps
     and no attribute but its `name` is a seeder quirk (gitlab seeds an empty
     `build: {}`, github/earthly don't); `hub_signature` drops it (and its lone
     name-attr), matching the existing empty-attr rule. earthly's seeder also no
     longer seeds an empty recipe as a job.

- **Verified:** diagonal grand-total 0; gated fixpoint suite green;
  `pipeline-forward` 11 tests + clippy clean. The runtime tool (`migrate`)
  carries the same normalization, so it migrates exactly as this matrix shows.

Regenerate matrix: `cargo test -p chaos-generator --test roundtrip interop_matrix -- --ignored --nocapture`
