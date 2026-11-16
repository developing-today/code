# Atlas — content versioning and retention

Design doc — 2026-11-08. Deliverable for #6026 (parent: Atlas ingestion
program #6018; statements S16–S20, S29, S30 of the 2026-11-04 brief).
Companions: the Atlas binary+service design
(`doc/2026-11-04T08-07-13Z_design_atlas_binary_service/`), the
manifest/filter-surface design
(`doc/2026-11-05T13-03-49Z_design_atlas_manifest_filter_surface/`), and
the PITR-vs-retention decision
(`doc/2026-11-05T15-50-41Z_decision_atlas_pitr_vs_retention/`). Design
only; no implementation is authorized by this note. Implementation is
split into follow-ons in §8.

State verified at `origin/main` `f8b8ebccbe` (2026-11-08): no `doc/`
entry covers content versioning — the Atlas design docs present are
binary_service, scaler_revision, and manifest_filter_surface, and the
only retention record is the PITR boundary decision above. The inputs
below are settled and are not re-litigated here: the research recorded
on #6026 (2026-11-05), the keep-all rule of #3125, and the PITR boundary
of PR #6269.

---

## 1. Requirement in one paragraph

Atlas ingests files from many sources and must treat **change as
versioning, not replacement**: a changed file lands as a new version of
the same object; a renamed file's versions continue the same lineage;
resolution defaults to the newest version ("the max"); old versions
roll out to cold storage over time; and old versions are eventually
removed only under an explicit policy with three named triggers — by
date, by increments, by access. Everything else in this note is the
model that makes those five sentences implementable without breaking
the two standing constraints: #3125's keep-all rule, and the
query-trace requirement (#6027) that a served result forever resolves
to the exact state that was served.

## 2. Version object model

### 2.1 Identity: file id vs version id

- A **logical file** is identified by a stable `file_id`, minted by
  Atlas at first ingestion. A path is an _attribute of a version_,
  never the identity: paths change (renames), collide across sources,
  and disappear. Two sources may hold files at the same path; they are
  different logical files unless a lineage edge (§3) says otherwise.
- A **version** is identified by `(file_id, version_seq)`, where
  `version_seq` is a per-file monotone integer assigned by Atlas in
  ingestion order. A version is immutable once written: content,
  path-at-that-version, source coordinates (source id, source-native
  id/commit, byte range or message id), ingestion time, and content
  hash. A changed file is a new version row, never an update.
- Content identity is separate from version identity: the content hash
  names the bytes. Byte-identical content — across versions of one
  file or across files — is stored once; versions reference it. (This
  is the one cleanup #3125 already permits: byte-identical dupes.)

### 2.2 Lineage edges

Versions of one file form a chain; lineage is also where a file's
identity survives structural change. Edges are first-class rows,
each with a `kind`, evidence, and confidence:

| edge kind   | meaning                                                        |
| ----------- | -------------------------------------------------------------- |
| `edit`      | version _n+1_ is the successor of version _n_ at the same path |
| `rename`    | successor at a different path, same logical file               |
| `split`     | one file's lineage continues in two or more files              |
| `merge`     | two or more files' lineages continue in one file               |
| `tombstone` | the file was deleted at the source; head resolves to nothing   |

The **head** of a file is the version reachable by following the
successor chain to its end — or a tombstone marker (§5.2). Split and
merge edges make the lineage a DAG at the file level even though the
common case is a chain; resolution rules in §5 are stated over the
DAG, not the chain, so splits never silently fork identity.

### 2.3 Chunks map to versions by occurrence, not by copy

Chunking and embedding run per version, but storage is by occurrence,
per #3125's occurrence-registry semantics: a chunk row names its
content once; an **occurrence** row binds `(chunk, version)` with the
chunk's span in that version. A chunk shared by twelve versions of a
file is one chunk and twelve occurrences. Two consequences are load-
bearing for retention (§4, §6):

- Deleting a version deletes its occurrence rows; it may delete a
  chunk row **only when no occurrence references it any more**. A
  retention pass can therefore never orphan a chunk that a surviving
  version still points at, and never strands an occurrence pointing
  at a deleted version.
- Retrieval statistics (§4.3) attach to versions through occurrences:
  "this version was surfaced" is recorded per occurrence hit and
  rolled up per version, which is exactly the granularity the
  by-access trigger consumes.

## 3. Lineage rules per source type

Atlas computes and stores lineage edges itself. No source system
records lineage in a form Atlas can trust outright; each source
contributes signals, ranked by strength, and an edge is written with
the evidence that produced it so a later pass can revise it.

### 3.1 Git sources

Git stores snapshots, not renames: rename detection is a per-diff
heuristic re-guess (similarity threshold `-M50%` by default; copies
need `-C`), and it fails exactly where lineage matters most —
rename-plus-heavy-edit in one commit reads as delete+create, splits
and merges are invisible to whole-file similarity, and large diffs
silently hit the rename-detection limit. Therefore:

- Ingestion is per commit, in commit order. For each path added at a
  commit, Atlas looks for a lineage predecessor among paths deleted
  or modified in the same commit and in the file's recent history,
  scoring candidates by (a) git's own `-M`/`-C` output at ingestion
  time, (b) content similarity computed by Atlas's segment/dedup
  machinery (shared-chunk ratio), and (c) same-basename and
  same-directory priors. The best candidate above threshold becomes
  a `rename` (or `split`/`merge`) edge with its evidence recorded;
  below threshold, the path starts a new logical file. A later,
  stronger signal may supersede an edge — edges are versioned too,
  and resolution (§5) uses the current edge set.
- Commit hygiene is a _hint quality_ lever, not a correctness
  requirement: source repos SHOULD land renames as rename-only
  commits (`git mv`, no simultaneous edit) so signal (a) is strong.
  Atlas must remain correct when they do not — that is the normal
  case for repos the fleet does not control.

### 3.2 Non-git sources (session stores, doc trees, chat logs)

No diff machinery exists at all, so lineage is inferred entirely:

- **Append-mostly sources** (chat logs, session stores): the source-
  native conversation/session id is the lineage anchor. Each sync
  that observes grown or edited content mints a new version of the
  same logical file; identity never depends on the file's name in
  the store.
- **Doc trees and file shares**: identity is anchored on the
  source-native object id where the source provides one (drive file
  ids, note ids); where it does not, on `(source, content-hash
neighbourhood)` — a new object whose chunks substantially overlap
  a recently deleted object's is proposed as its rename successor,
  using the same segment-association scoring as §3.1(b).
- **Splits/merges** in non-git sources are detected the same way:
  one deleted object's chunks fanning out into two new objects is a
  `split`; the reverse is a `merge`. Confidence is recorded; a
  low-confidence structural edge is stored as a candidate and does
  not affect resolution until confirmed by a second signal.

## 4. Retention policy

### 4.1 The governing rule (reconciliation with #3125)

#3125 (2026-10-31) rules: **keep ALL data; nothing stripped to save
space; only byte-identical dupes cleaned.** The 2026-11-04 brief
deliberately evolves that rule for _versions_: a version may be
retired, but the default is keep, and retirement is a conjunction,
never a single signal:

> A version is retired only when **(a)** at least one trigger below
> fires for it, **and (b)** the retention rule for its class (§4.2)
> permits retirement. If either side says keep, the version is kept.

Class rules may only _narrow_ what triggers can retire (longer
windows, more increments, keep-head-always); no class rule may retire
a version no trigger fired on. The head version of a live file and a
tombstoned file's last version follow §4.2's per-class rule — the
head is never retired by the increments trigger alone while it is
still the head.

### 4.2 Per-class defaults

Classes follow the classification manifest's vocabulary. Defaults
below are starting values for the rule engine (#6041) to parameterize,
not constants in code; who may override follows the row.

| class             | by date                                   | by increments                    | by access                              | overrides                  |
| ----------------- | ----------------------------------------- | -------------------------------- | -------------------------------------- | -------------------------- |
| code              | retire non-head versions older than 400 d | keep newest 50 versions per file | never-retrieved after 180 d may retire | repo owner (per-repo rule) |
| docs              | older than 730 d                          | keep newest 25                   | never-retrieved after 365 d            | corpus owner               |
| session logs      | older than 180 d                          | keep newest 10                   | never-retrieved after 90 d             | program owner (#6018)      |
| chat logs         | older than 365 d                          | keep newest 10                   | never-retrieved after 180 d            | program owner              |
| generated reports | older than 90 d                           | keep newest 5                    | never-retrieved after 30 d             | generating job's owner     |

Reading the table: a version retires when it is past its class's date
threshold, **or** more than the class's increment count of newer
versions exist beyond it, **or** its access record (§4.3) shows it
qualifying as never/no-longer retrieved past the class's access
window — subject always to §4.1(b): the class row is the permission,
the trigger is the cause, both are required. "Keep newest N" is a
floor: increments never retire any of the newest N versions of a
file, whatever the other triggers say about them individually (date
and access triggers still apply to them only if the class row's
other columns independently fire — the floor binds the increments
column only).

### 4.3 The three triggers, precisely

- **By date.** Version's ingestion time is older than the class
  threshold. Wall-clock only; the simplest trigger, and the backstop
  that bounds total growth for classes nothing ever queries.
- **By increments.** At least N newer versions of the same file
  exist beyond this one (N = class increments column). This bounds
  per-file depth: hot files with long edit histories shed their
  oldest versions first, regardless of age.
- **By access.** The version has never been surfaced by retrieval,
  or has not been surfaced within the class access window. "Surfaced"
  means returned in a result set, recorded through the occurrence
  roll-up of §2.3 and fed by the retrieval statistics of the sibling
  stats/feedback design under #6018. A version that keeps being
  surfaced is, by definition, still earning its storage; this trigger
  is what retires the long tail that date and increments cannot see
  (a file edited twice, years ago, never read since).

### 4.4 Retirement is a report before it is a deletion

No silent vacuum. The rule engine's first mode is a **dry-run
reporter** (#6041): it lists the versions each rule _would_ retire,
with the trigger and class rule that fired, and nothing is deleted
from a report alone. Deletion runs are separate, logged acts: every
retired version leaves a tombstone record (file id, version seq,
content hash, trigger, rule, run id) so the corpus can always answer
"what was here and why is it gone" — and so a served-result trace
(#6027) that references a retired version resolves to "retired under
policy", not to a dangling reference.

## 5. Query-side version contract

### 5.1 Newest-resolution is a view

"The max" is a **query-time view over the lineage graph**, never a
destructive replace at ingestion. Storage keeps every version (§2);
resolution, at read time, maps each logical file to its head version
and searches that. This is what keeps history addressable (§5.3) and
keeps served results stable: a result served yesterday names version
ids, and those ids still mean the same bytes today.

### 5.2 Tombstones do not resurface

If a file's head edge is a `tombstone` (deleted at the source), the
default view excludes the file entirely. The previous version does
**not** resurface as "the newest that exists" — that would make
deletion at the source unobservable and would serve content the
source removed. Historical addressing (§5.3) can still reach the
pre-tombstone versions explicitly, subject to retention (§4).

### 5.3 Explicit history addressing

The query surface (with #5774's parameter design; fan-out behaviour
per #5755) gains three version parameters:

- `version = latest` (default) — the §5.1 view.
- `version = <version_seq>` or a version id — pin to one version;
  valid in per-file addressing and in trace replay.
- `as_of = <timestamp>` — resolve each file to the newest version
  ingested at or before the timestamp. `as_of` past a version's
  retirement (§4.4) resolves that file to its retirement tombstone,
  stated in the response, never silently to a different version.

### 5.4 Multi-version hits and full-version logging

Within one query, hits are grouped by logical file: **one file, best
version** — the response carries the best-scoring version per file,
not one hit per version, unless the query is explicitly historical
(pinned or `as_of`), in which case each addressed version is its own
result. Every served run logs, per result, the exact `(file_id,
version_seq, content hash)` it came from (full-version logging),
feeding the query-trace design (#6027): a trace must be replayable
bit-for-bit against the versions it names, which is the concrete
reason §5.1 forbids destructive replacement.

## 6. Cold rollout

- **Tiers.** Hot storage holds heads and recent versions; cold
  storage is the rustfs tier (2026-11-04 storage ruling). Movement
  hot → cold is driven by the same per-class table as §4.2 with its
  own, earlier thresholds — a version rolls cold well before any
  retirement trigger could fire on it. The exact cold thresholds are
  #6042's to fix alongside the shard format; this design fixes only
  the contract below.
- **Resolvability is preserved.** A cold version still resolves —
  slower, through the restore path — until a deletion trigger fires.
  Cold is a latency class, not a visibility class: default-latest
  resolution, `as_of`, and pinning all reach cold versions without
  caller action. **Deletion is the only hard end** of a version's
  life.
- **Format.** Cold versions are packed in self-describing rustfs
  shards with their restore path (#6042): a shard carries the
  metadata needed to restore its versions without the hot index, so
  cold storage is recoverable on its own evidence.
- **Boundary with PITR.** Turso point-in-time recovery is whole-
  database disaster recovery (≤90-day windows) and is _never_ a
  corpus-retention mechanism — per the decision record of PR #6269,
  per-version lineage, cold rollout, and policy deletion all live in
  Atlas, not in database recovery.

## 7. What this design does not decide

- Numeric thresholds beyond the §4.2 starting defaults — they are
  rule-engine parameters (#6041), tuned from its dry-run reports.
- The cold shard layout and restore protocol (#6042).
- The stats/feedback pipeline's own schema (sibling design under
  #6018); §4.3 consumes its per-version surfaced record, and that
  interface — version id in, surfaced-events out — is the only
  coupling.

## 8. Implementation split (follow-ons)

1. **Version + lineage storage** — `file`/`version`/`lineage_edge`/
   `occurrence` tables per §2, ingestion minting versions instead of
   replacing, per §3's source adapters (git first, then session/doc
   sources). New follow-on under #6018.
2. **Retention rule engine + dry-run reporter** — already filed as
   #6041: the §4 table as data, the three triggers, retirement
   tombstones, report-first operation.
3. **Cold tier** — already filed as #6042: rustfs shard format,
   hot → cold mover on §6's contract, restore path.
4. **Query-side contract** — `version` / `as_of` parameters, head-view
   resolution with tombstone handling, per-file hit grouping, and
   full-version run logging, coordinated with #5755 (fan-out) and
   #5774 (parameter surface) and feeding #6027 (query trace). New
   follow-on under #6018.
