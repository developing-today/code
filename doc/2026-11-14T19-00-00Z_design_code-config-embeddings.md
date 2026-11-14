# Code/config embeddings — definition, verdict, and qualifying criteria

Issue: developing-today/code#1518. Status: **confirmed design** —
the recommendation below was confirmed by Drew on 2026-11-12
(decisions ledger `embedding-ranking-1518`), with added guidance
recorded verbatim in §5. Triage record: task 2602. This document
is the written definition the issue's done-criteria require; the
ingestion build follows it.

## 1. Definition

A **code/config embedding entry** is one retrievable unit of a
repository artifact — source code or an infrastructure-defining
config — embedded into Atlas (the fleet vector store, per the
#1300 ruling) alongside prose memory, and carrying enough
metadata to be traced back to the exact bytes it came from.

### Unit of embedding

- **Small configs** (infrastructure-defining YAML/TOML/JSON and
  peers, up to ~32 KiB): the **whole file** is one unit. A DNS
  config means something as a whole; splitting it destroys the
  cross-references retrieval needs.
- **Large configs and source code**: **declaration-sized
  chunks** — one top-level config section, one function/type/
  resource block — never fixed-size windows that cut a
  declaration in half. Chunk boundaries come from the artifact's
  own structure (parser or top-level-key split), recorded in
  metadata.

### Required metadata (every entry)

`repo`, `path`, `commit_sha` (the revision the bytes came from),
`kind` (`prose` | `code` | `config`), `language` (yaml, toml,
rust, terraform, …), `embedded_at` (timestamp), `unit` (whole-file
or the chunk's structural address), `sensitivity` (the existing
basic sensitivity classification), and a **source-of-truth
pointer**: `repo` + `path` + `commit_sha` is that pointer — the
repo at that revision is the truth; the embedding is a cache.

In Atlas staging terms (`pkgs/atlas-search/schema/
atlas_stage_v1.sql`) an entry is one `items` row — `kind`,
`source` (the pointer), `text`, `sensitivity`, `created_at`,
`meta` (JSON bag carrying repo/path/commit_sha/language/unit) —
plus its `chunks_raw` rows; vectors are produced at promotion,
as for every other staged item.

### Freshness

- **Trigger**: a commit that touches the artifact's path
  re-embeds it (git-driven); a deploy or a manual request can
  also trigger.
- **Staleness**: an entry whose `commit_sha` differs from the
  path's current HEAD revision is **stale**: it is marked (meta
  records the superseding SHA once known) and replaced on
  re-embed. Stale entries are never presented as current
  content; retrieval may return them only labelled stale.
- **Eviction**: an artifact deleted at HEAD has its entries
  evicted at the next freshness pass, not left as ghosts.

## 2. The same-dump question — verdict

**One dump, one index.** Code/config entries live in the same
Atlas index as prose memory, discriminated by the `kind` tag,
with **kind-aware retrieval quotas** so neither corpus can
starve the other out of a result set.

Reasons: the motivating failure was a _cross_-corpus miss —
prose said a DNS project exists, and retrieval needed to reach
the config itself; two indexes make that join the caller's
problem forever. One index keeps one freshness story, one
sensitivity model, one export/portability path. The arguments
for splitting (embedding-model fit, chunk-granularity mismatch,
update-cadence difference) are real but are handled inside the
shared index: `kind` + `language` metadata for model/routing
choices, per-kind chunking rules (§1), and commit-driven
freshness (§1) that prose's append-only cadence never has to
imitate.

## 3. The three decisions, and the failure each accepts

1. **Shared index with a `kind` tag** (not a separate code
   dump). _Failure accepted:_ retrieval pollution in both
   directions — a prose query can surface a config chunk and
   vice versa. Kind-aware quotas bound it; they do not
   eliminate it. If pollution proves worse than the join cost,
   this decision is the one to revisit, with evidence.
2. **Whole-file units for small configs.** _Failure accepted:_
   section-level precision loss — a 30 KiB config whose one
   relevant section should rank can be dragged by the rest of
   the file, and a query for one record may retrieve the whole
   config. Accepted because the cross-reference loss from
   splitting configs was the original defect (§1).
3. **Commit-SHA freshness with stale marking** (not
   synchronous re-embedding on every commit). _Failure
   accepted:_ a freshness window — between a config commit and
   its re-embedding, retrieval can return the previous revision,
   labelled stale. A reader who ignores the label can act on
   old config. Accepted because synchronous embedding on the
   commit path would couple git latency to a model call.

## 4. Qualifying criteria

An artifact qualifies for embedding when any of these hold:

- It is an **infrastructure-defining config** — DNS, network,
  deployment, identity/access, or service-routing configuration
  (Drew's rule: this class qualifies **by default**).
- It is source code in a repo on the fleet's repo list, at
  declaration granularity.
- A human or an epic explicitly names it.

Exclusions, always: anything the existing basic sensitivity
filters classify as secret-bearing (keys, tokens, state files
such as `terraform.tfstate*`), generated artifacts with a
living source (embed the source), and vendored third-party
trees. Only the existing basic sensitivity filters apply
(Drew's guidance, §5) — no new filter tier is built for this.

## 5. Drew's added guidance (2026-11-12, verbatim substance)

Confirm the recommended design; allow for more down the road;
only the existing basic sensitivity filters; build into Atlas;
support sharding to many Turso DBs using workers (Durable
Objects and D1 as appropriate); benchmark various shard sizes.
The sharding/benchmark work is sequenced _after_ the minimal
ingestion path below proves retrieval; shard-size benchmarks
are recorded against the same corpus at several shard sizes
before any sharded rollout.

## 6. First ingestion (the minimal path)

First entry: `infra/dns/dns_config.yaml` (31 KiB, whole-file
unit, kind `config`, language `yaml`) — the artifact whose
absence motivated the issue (dezren39/nix#974; the concrete
gap is also tracked on #2469, which this work coordinates
with). Path: stage one `items` row + `chunks_raw` into an
Atlas staging unit (`atlas_core::stage`), promote (embedding
at promotion), then prove a semantic query about the DNS
setup retrieves the config entry itself — not prose about it.
