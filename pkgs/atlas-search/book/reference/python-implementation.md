# atlas_search.py: the Python reference implementation

Source: `pkgs/atlas-search/atlas_search.py`, `eval/README.md`,
`crates/atlas-search/src/main.rs` (the Rust port's header),
`crates/fleet/crates/fleet-search/src/{pyjson,render}.rs` and
`.github/scripts/check-fleet-search-parity.sh` in
https://github.com/developing-today/code/blob/main/pkgs/atlas-search/ —
provenance only; this chapter is complete without following it.

`atlas_search.py` is the original Atlas search implementation, written in
Python, and the parity reference the Rust implementation was certified
against. It lives at the root of the `pkgs/atlas-search/` package. The Rust
binary is described in its own source header as a surface-parity port of this
file's front end over `atlas-core` and `atlas-embed`: wording, defaults,
fatal-message texts and exit codes were all taken from the Python. The Python
also remains the permanent evaluation instrument — the parity gates and the
golden-set scoring run through its lineage — and it is retained, not retired:
the migration plan renames and keeps it.

It is also still the only surface with two capabilities the Rust binary
refuses: the fastembed/bge embedder path, and non-default embedding models
for vector and hybrid search. In the Rust binary, a vector or hybrid search
with a fastembed embedder or a non-Snowflake model is a loud error pointing
back at the Python.

## Invocation

```text
python3 atlas_search.py search &lt;query&gt; [options]
python3 atlas_search.py get-chunk &lt;chunk_id&gt; [options]
```

Output is JSON by default; `--text` renders a human-readable form. FTS-only
search and `get-chunk` need nothing beyond the Python standard library. The
module header records the corpus model it searches: every conversation
normalised into one canonical store (sessions, then messages, then chunks),
every session classified under scheme-1.0, every chunk embedded with Snowflake
`snowflake-arctic-embed-m-v2.0` at the full 768 dimensions, and a hot copy
published to Turso. The production search shape it records — decided
2026-10-31 — is one-stage hybrid search: an FTS5 (BM25) branch and a vector
(cosine) branch fused with reciprocal rank fusion at k = 10, with a pool of
100 candidates per branch. Sensitivity is a caller ceiling, enforced before
ranking, with default `personal`.

## The two commands and their flags

The surface is 2 commands and 25 flags in the survey's count, plus the two
positionals `&lt;query&gt;` and `&lt;chunk_id&gt;`. There are no other verbs:
no `serve`, `init`, `fetch-model`, `traces` or `feedback` — those exist only
in Rust.

### `search`

| Flag | Argument | Default | Meaning |
|---|---|---|---|
| `--k` | integer | `10` | Final results |
| `--pool` | integer | `100` | Per-branch candidates |
| `--mode` | `hybrid`, `fts` or `vector` | `hybrid` | Which branches run |
| `--kind` | CSV string | none | Classification kind filter |
| `--topic` | CSV string | none | Topic filter |
| `--project` | CSV string | none | Project filter |
| `--client` | CSV string | none | Client filter |
| `--since` | `YYYY-MM-DD` | none | Sessions created on or after the date (inclusive, on `sessions.created_at`) |
| `--until` | `YYYY-MM-DD` | none | Sessions created on or before the date (inclusive) |
| `--context` | string | `local-owner` | Disclosure context of the destination surface. Free-form and unvalidated in the Python; an unknown value fails closed inside the disclosure logic |
| `--max-sensitivity` | ladder name | `personal` | Caller ceiling, in `shareable`, `personal`, `local-only`, `secret-bearing` order |
| `--group-by-session` | switch | off | Roll chunk hits up to sessions |
| `--expand` | integer | `0` | Include that many neighbouring chunks around each hit |
| `--with-message` | switch | off | Include the parent message text |
| `--text` | switch | off | Human-readable output instead of JSON |
| `--backend` | `auto`, `turso` or `local` | `auto` | Data backend |
| `--db` | path | the canonical `atlas.db` (resolution: the `ATLAS_DB` environment variable, then the turso-atlas data path, then a script-relative data path) | Local backend database |
| `--vecdb` | comma-separated paths | none (environment fallback `ATLAS_VEC_DB`) | Local backend vector DB path(s) |
| `--model` | string | `Snowflake/snowflake-arctic-embed-m-v2.0` (environment `ATLAS_SEARCH_MODEL`) | Embedding model |
| `--vec-table` | string | `vec_snowflake_m_v2_768_f32` (environment `ATLAS_VEC_TABLE`) | Vector table name |
| `--embedder` | `auto`, `fastembed` or `st` | `auto` | Query embedder: fastembed (bge) or sentence-transformers with the query prompt (Snowflake) |
| `--trunc` | integer | `0` | Truncate query and document vectors to this many dimensions (0 = full width) |
| `--vector-backend` | `scan`, `index-fill` or `index-fixed` | `scan` | Vector branch strategy on the Turso backend |

### `get-chunk`

| Flag | Argument | Default | Meaning |
|---|---|---|---|
| `--neighbors` | integer | `0` | Neighbouring chunks to include, either side in the same message |
| `--text` | switch | off | Human-readable output instead of JSON |
| `--context` | string | `local-owner` | Disclosure context of the destination surface |
| `--max-sensitivity` | ladder name | `secret-bearing` | Caller ceiling for this explicit-id lookup — note the default differs from `search` |
| `--backend`, `--db`, `--vecdb` | as `search` | as `search` | The shared backend argument group |

Worked example — the documented dormant fallback, which only this
implementation can run:

```text
python3 atlas_search.py search "queue broker lease" \
  --model BAAI/bge-small-en-v1.5 \
  --vec-table vec_bge_small_384_f32 --embedder fastembed
```

A second example, at the defaults a parity run uses:

```text
python3 atlas_search.py search "onnx build failure" --mode hybrid --k 10
python3 atlas_search.py get-chunk 112081 --neighbors 2 --text
```

(The chunk id is a toy example, not corpus content.)

## The parity contract

Parity runs in both directions, and both are guarded by recorded fixtures
rather than by reading the two implementations side by side:

- **The Rust port is certified against the Python.** The embedder must
  reproduce the Python corpus pipeline bit-identically on a fixed probe set
  (Gate A), and the search core must reproduce envelopes and SQL strings
  recorded from the production Python module (the core-expectations fixture,
  recorded by its generator from `atlas_search.py` itself). Fatal messages
  mirror the Python's `die()` texts; a `die()` prints
  `atlas-search: &lt;message&gt;` to stderr.
- **The fleet port's writers are byte-parity guarded.** The fleet binary's
  search surface carries copies of the Rust binary's Python-JSON writer
  (`pyjson.rs`) and envelope renderer (`render.rs`). A parity guard script
  compares the fleet copies against the `atlas-search` originals byte for
  byte outside each copy's pinned parity header, so a fleet service-mode
  search renders its envelope byte-identically to `atlas-search`. Note the
  limit, stated plainly: the guard workflow is dispatch-only — nothing runs
  it automatically when either copy changes. The guard exists; automatic
  gating does not.
- **The in-repo copy is a recorded copy.** The `eval/` documentation records
  that the in-repo `atlas_search.py` was copied byte for byte from the
  turso-atlas scripts tree, with its SHA-256 recorded there. If the two
  copies ever diverge, that record is how the divergence is detected.

## Where parity breaks — source wins

The following are recorded disagreements between the Python and the shipped
Rust. In each case the book states the source behaviour of the implementation
named; neither side is silently corrected here.

1. **`search --context` default and validation.** The Python defaults
   `--context` to `local-owner` for both verbs, hardcoded in its shared
   disclosure-arguments builder, and does not validate the value — an
   unknown context fails closed inside the disclosure logic. The Rust
   `atlas-search search` defaults to `fleet-internal` (validated; an unknown
   value is a usage error, exit 2), and only its `get-chunk` defaults to
   `local-owner`. The Rust header's surface-parity claim does not hold for
   this flag.
2. **`get-chunk --max-sensitivity`.** The Python's `get-chunk` has the flag,
   defaulting to `secret-bearing`. The Rust `get-chunk` has no
   `--max-sensitivity` flag at all — passing one is a usage error — and the
   equivalent ceiling arrives through `--context local-owner`, whose context
   ceiling is `secret-bearing`. (For `search`, both implementations default
   `--max-sensitivity` to `personal`.)
3. **Reciprocal rank fusion k: 60 versus 10.** The shipped Rust fuses at
   k = 10, from the versioned query-parameter contract, and the Python
   production header likewise records k = 10 for the production shape. But
   the Python *evaluation* lineage — `eval3.py`'s hybrid, copied from the
   earlier vector evaluation script, and the Jaccard gate built on it —
   fuses its top-30 branches at k = 60. Evaluation numbers from that lineage
   are therefore not measurements of the production configuration, whatever
   their surrounding prose calls them. See
   [Evaluation: golden sets and gates](../concepts/evaluation.md).
4. **The header points at a README that is not there.** The file's header
   says "(see README.md in this directory)". There is no `README.md` in
   `pkgs/atlas-search/`; the README matching that description is the
   turso-atlas README in the laptop and hatch-personal tree. A reader who
   follows the pointer inside the repository will not find it.
5. **Two copies, two lengths.** The design of record describes the Python as
   1,177 lines — true of the hatch-personal copy in the turso-atlas scripts
   tree. The in-repo copy at `pkgs/atlas-search/atlas_search.py` is 1,629
   lines. The two copies are not the same file version, so "the Python
   reference" needs a which-copy qualifier wherever a line count or a
   specific behaviour is cited.

There are also deliberate, documented non-parities on the Rust side — for
example the Rust filter builder escapes topic and project matching so a value
matches only a whole JSON element, where the Python's unescaped pattern has a
wildcard bug that remains unfixed in the Python. Those are recorded in the
fixtures documentation as intentional deviations, not breaks discovered after
the fact.

## Exit codes and failure shapes

| Code | Meaning |
|---|---|
| `0` | Success |
| `1` | `get-chunk` for a chunk that does not exist (a `die()` call with code 1); unhandled backend failures surface as tracebacks, also exit 1 |
| `2` | Argument errors, and `die()`'s default: validation failures, token resolution failures, and backend HTTP or SQL errors raised through `die()` |

One shape worth knowing before scripting against either implementation: in
the Python, `die()` calls inside worker threads raise `SystemExit` there and
are ferried to the calling thread, so the observable behaviour — message plus
exit 2 — matches the Rust port even though the mechanism does not. The Rust
binary prints the message where the Python's backend failures could surface
as a traceback; that difference is intentional and recorded in the Rust
header.

For the Rust surface this file is the reference for, see
[atlas-search: search and get-chunk](atlas-search.md). For the stores both
implementations read, see [Data stores and schemas](data-stores.md).
