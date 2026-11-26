# HYD-02 — Safe cross-repository source keys and artifact paths

Issue: [#9459](https://github.com/developing-today/code/issues/9459)
(slice `HYD-02` of [#9207](https://github.com/developing-today/code/issues/9207),
program [#9196](https://github.com/developing-today/code/issues/9196)).
Status: **design proposal — not an authorization to implement.**

This record elaborates the HYD-02 row of the merged design package
(`doc/2026-11-13T03-20-24Z_design_pm-stack-fleet-orchestration/`,
PR #9225). The row's goal is to "prevent collisions/path traversal
for same PR/issue number in different repos; preserve stable remote
IDs". Its first oracle is
`pmo_hyd_artifact_keys_are_repo_scoped_and_traversal_safe`, which
"generates same issue/PR number in two repositories plus `../`,
control and long Unicode titles". Its acceptance is "stable
deterministic unique digest-suffixed keys; rejects traversal/symlink
escape/control chars; normalization does not collapse distinct
refs". Its negatives are "case-fold collisions, long title truncates
digest".

This record _decides_:

- what identifies a source, and what is only its display locator;
- the canonical source key and its digest;
- the work key that names a work item's directory;
- the file name of every per-source artifact;
- the path rules a writer and a reader must both enforce;
- the typed errors for each refusal.

It authorizes no code, schema, migration, deployment or live
acceptance (§10).

## 0. Governing decisions consumed, not re-opened

- **Architecture, identity rule 2**
  (`architecture.md#identity-and-authority`): "GitHub
  `owner/repo#number` is an input/display locator. Retain stable
  provider resource and repository IDs plus kind; repository renames
  and equal numbers in different repos must not merge identities."
- **Architecture, identity rule 1**: a work reference is the pair
  `(authority_id, work_id)`, serialized as strings; the queue's
  decimal task ID stays the work ID.
- **Architecture, digest contract**
  (`architecture.md#digest-and-serialization-contract`):
  `digest(domain, dto) = "sha256:" + lower_hex(SHA256(ASCII(domain)
|| 0x00 || UTF8(JCS(dto))))`, with `domain` chosen only from a
  fixed registry.
- **Component #9207 §2.1**: input is a canonical `WorkId`, never an
  issue number alone; each source ref carries provider,
  owner/account, repository, resource kind and stable remote ID;
  file names use "a safe slug plus collision-resistant source
  identity, not raw title/path/owner input"; reject path
  separators, `..`, control characters and symlinks escaping the
  root; "path shortening must not remove a digest suffix".
- **Component #9207 §5**, failure row "Same issue/PR number in
  different repos": "Canonical repository identity in stable
  artifact key prevents overwrite."
- **Component #9207 §6**, PBT layer: "generate arbitrary repo names,
  PR numbers, titles, paths, Unicode/control/`..` input; artifact
  paths stay under root and distinct source refs never collide."
- **HYD-01 record** (#9458, open PR #15184): the manifest lists
  `artifact_refs` as `{path, sha256, bytes}`; an invalid path in a
  presented manifest is a named, typed error; its §2 lists fixed
  sibling files plus `diffs/pr-<n>.patch`. This record reads HYD-01
  as proposed in that PR, and §6 states the one change it needs.
- **HYD-03 record** (#9460, merged as #15275): the layout is
  `<root>/<work-key>/context/{staging,generations}/<generation_id>/`
  and `current.json`. It defers `<work-key>` and every file name
  inside a generation to this record. Its publisher rejects any path
  outside the staging directory and any symlink.
- **HYD-04 record** (#9461): HYD-04 writes artifact content only
  through HYD-02's paths.

## 1. Verified state (2026-11-25, main at `7d9f0b62`)

- `pkgs/fleet/crates/fleet-queue/src/work/` holds `authority.rs`,
  `completion.rs`, `contracts.rs`, `degraded.rs`, `journal.rs`,
  `lifecycle.rs`, `mod.rs`, `operations.rs`, `priority.rs`,
  `routing.rs`, `selection.rs` and `sqlite_replica.rs`. There is no
  `context.rs`, so nothing writes a hydration artifact today.
- No source-key or artifact-key helper exists in the fleet
  workspace. The nearest prior art is display-only slugging:
  - `fleet/src/derived_version.rs` `slugify_branch` (lowercase ASCII
    alphanumerics and `-`, capped at 32);
  - `fleet-cert/src/naming.rs` `slugify`, whose doc comment states
    "a slug is never the identity, only a path component".

  This record adopts that rule.

- `fleet-queue` already depends on `sha2` and carries `libc` as an
  optional dependency. It has no `cap-std` or `rustix`.
- Repo-health dedupe keys
  (`repo-health:v1:fix:developing-today/code:issue:9459`) name the
  repository by `owner/repo`. They are queue dedupe strings, not
  artifact identity, and this record does not change them.

## 2. Intent

A hydrated bundle for one work item may hold artifacts for several
issues and pull requests across several repositories. Two things
must never happen:

1. **Collision.** Two different remote resources map to one file, so
   one overwrites or masquerades as the other. `alpha/repo#10` and
   `beta/repo#10` are the canonical case.
2. **Escape.** Any input — a repository name, a title, a branch, a
   path in a presented manifest, a planted link on disk — makes a
   read or write land outside the bundle root.

The design makes collision a property of identity, not of string
cleaning. It also makes escape impossible by construction: no
untrusted text ever reaches a path.

## 3. Source identity

### 3.1 The identity tuple

A source is identified by exactly these fields, all resolved from
the provider before any artifact is named:

| Field           | Value                                                          | Alphabet                 |
| --------------- | -------------------------------------------------------------- | ------------------------ |
| `provider`      | `github` (the only provider in V1)                             | closed enum              |
| `host`          | the API host, lowercased (`github.com`, or a GHES host)        | DNS name, `[a-z0-9.-]`   |
| `repository_id` | the provider's stable numeric repository ID                    | decimal, no leading zero |
| `kind`          | one of the kinds in §3.2                                       | closed enum              |
| `remote_id`     | the provider's stable ID of the resource                       | per kind, §3.2           |
| `qualifier`     | the pinned revision for kinds bound to one (§3.2), else `null` | lowercase hex, or `null` |

Owner login, repository name and issue or PR number are **locator**
fields. They are recorded beside the identity, in the manifest's
`source_ref` display URI and for rendering. They never take part in
the key. So:

- a repository rename keeps every key;
- equal numbers in different repositories give different keys;
- owner or name case differences (`Alpha/Repo`, `alpha/repo`) cannot
  split one repository into two keys.

### 3.2 Kinds

| `kind`           | `remote_id`                              | `qualifier`                        |
| ---------------- | ---------------------------------------- | ---------------------------------- |
| `repository`     | the repository ID again                  | `null`                             |
| `issue`          | the issue's numeric database ID          | `null`                             |
| `pull_request`   | the pull request's numeric database ID   | `null`                             |
| `issue_comment`  | the comment's numeric database ID        | `null`                             |
| `review`         | the review's numeric database ID         | `null`                             |
| `review_comment` | the review comment's numeric database ID | `null`                             |
| `check_run`      | the check run's numeric database ID      | `null`                             |
| `commit`         | the 40- or 64-hex commit SHA             | `null`                             |
| `diff`           | the pull request's numeric database ID   | the pinned `head_sha` (#9207 §2.3) |
| `timeline`       | the issue's or PR's numeric database ID  | `null`                             |

Numeric IDs are decimal strings in the key. A JSON number is never
used, so the lossy-numeric-ID rule of the digest contract cannot
bite. Adding a kind is an additive change to this table and to the
schema version of §4.2.

`diff` carries its `head_sha` because #9207 §2.3 binds all PR-derived
artifacts to one captured head. Two heads of one PR are two keys, and
one head is always the same key.

### 3.3 Resolution comes first

A locator the provider has not resolved to the tuple of §3.1 has no
key. The hydrator records such a source in the manifest with
`status: unavailable` (or `not_found`/`forbidden` as the adapter
reports) and **no** `artifact_refs`. It never falls back to naming
an artifact from the locator. Error: `SOURCE_UNRESOLVED` (§7). This
is what stops `owner/repo#10` from becoming a file name when the
repository ID is unknown.

Issue transfer between repositories is not assumed to preserve or
change any ID. The key follows whatever IDs the provider returns
after the transfer. If they change, the old key stays in older
generations and the new key appears in new ones, and HYD-06 links
the two through the activity record.

## 4. Canonical key and digest

### 4.1 The canonical key string

```text
source_key = provider ":" host ":" "r" repository_id ":" kind ":" remote_id [ "@" qualifier ]
```

Example: `github:github.com:r123456:pull_request:987654321`, and for
a diff `github:github.com:r123456:diff:987654321@<40-hex>`.

The string is injective. No field's alphabet contains `:` or `@`
(host names included), so two different tuples cannot produce one
string. A
writer builds the string only from a validated tuple. A reader that
meets a `source_key` re-parses it, re-validates each field and
rebuilds the string; any mismatch is `UNSAFE_SOURCE_KEY`.

### 4.2 The digest

```text
key_digest = digest("fleet.source-key/v1", {
  "schema": "fleet.source-key/v1",
  "provider": ..., "host": ..., "repository_id": ...,
  "kind": ..., "remote_id": ..., "qualifier": ... | null
})
```

It uses the architecture's exact framing (JCS DTO, one NUL, SHA-256,
`sha256:` + lowercase hex), with every value a string or `null`.
`fleet.source-key/v1` is a **new registry domain**. Adding it is an
amendment to the architecture's domain table and lands with this
record's implementation gate (§10), never as free client text.

The artifact name uses the first 32 hex characters (128 bits) of the
digest, the **name digest**. Collision across one work item's bundle
is negligible at that width. §5.4 still makes a collision a refusal,
never an overwrite.

### 4.3 The work key

HYD-03 calls `<work-key>` "repo-scoped"; precisely, it is
**authority-scoped**. A work item belongs to an authority, not to a
repository. Its sources may span repositories, and those are scoped
per source by §4.1.

```text
work_key = "w" work_id "-" first 16 hex of digest("fleet.source-key/v1",
           {"schema": "fleet.work-key/v1", "authority_id": ..., "work_id": ...})
```

- `work_id` is the decimal queue task ID. It is validated as decimal
  with no leading zero before use, so it is path-safe as is.
- `authority_id` never appears in the path. It is hashed, so any
  authority string (including one with `/` or Unicode) is safe.
- Two authorities with the same `work_id` get different keys.

The work key DTO shares the source-key domain and is distinguished by
its `schema` member, so the registry gains one domain, not two.

## 5. Artifact names and paths

### 5.1 File names

Every per-source artifact file is named:

```text
<label>--<name digest>.<ext>
```

- `<label>` is presentation only. It is built from the closed `kind`
  enum and, for `issue`/`pull_request`/`diff`/`timeline`, the display
  number: `issue-10`, `pr-231`, `diff-pr-231`, `timeline-pr-231`,
  `comment`, `review`, `check-run`, `commit`, `repo`. It never
  contains an owner, a repository name, a title, a branch or a path,
  so it is stable across renames and bounded in length.
- `<name digest>` is the 32-hex prefix of §4.2.
- `<ext>` is from a closed set: `json`, `patch`, `md`.

So `alpha/repo#10` and `beta/repo#10` become `issue-10--<a>.json` and
`issue-10--<b>.json` with `a != b`.

Length rule: a name is at most 96 bytes. The label is at most 40
bytes, numbers included. If a future label would exceed that, the
label is cut, never the digest or the extension. The digest is
always the last thing before the extension (#9207 §2.1: shortening
never removes the digest).

### 5.2 Fixed files and directories

HYD-01 §2's fixed sibling files are aggregate files, one per type
(`summary.md`, `queue.json`, `issues.json`, `pull_requests.json`,
`comments.json`, `reviews.json`, `checks.json`, `timeline.json`,
`commits.json`, `dependencies.json`, `activity-cursors.json`). They
keep their fixed names. Each record inside carries its `source_key`,
so the aggregate holds many repositories without collision.

Per-source files live in one directory per type. For V1 that is
`diffs/`, with names from §5.1. HYD-01's `diffs/pr-<n>.patch` is
exactly the cross-repository collision this slice removes (§6).

A generation directory therefore has depth at most 2 below the
generation root: a fixed file, or a type directory plus one §5.1
name.

### 5.3 The safe component rule

Every path component that Fleet writes or accepts below the bundle
root must:

- be 1 to 96 bytes;
- use only `[a-z0-9._-]`;
- not be `.` or `..`;
- not start with `.` or `-`.

Every name in §4.3, §5.1 and §5.2, and every `generation_id`, meets
this by construction. The rule is still checked on every build and
every read, so a bug upstream fails closed.

Lowercase-only names cannot collide on a case-insensitive file
system. ASCII-only names cannot be split or merged by a file system
that normalizes Unicode (for example NFD). No name can equal a
reserved device name (such as `CON` or `NUL`) because every
per-source name contains `--` and a digest.

### 5.4 Writing

- Paths are built by joining validated components to the root
  handle. A path string is never parsed or concatenated with
  untrusted text.
- Each artifact is created with create-new semantics (`O_CREAT |
O_EXCL`), never truncate-or-create, so a pre-existing file or link
  at the name fails the write instead of being followed or replaced.
- Directories inside staging are created by the writer only. Each
  directory open from the root is no-follow (`O_NOFOLLOW |
O_DIRECTORY`, component by component, via `openat` on a held
  directory descriptor, or an equivalent capability API).
- The writer keeps the map of name to `source_key` for the
  generation. Two different keys that produce one name is
  `ARTIFACT_KEY_COLLISION`. The generation stops; the publisher
  never sees it; the prior current generation is untouched (HYD-03).

### 5.5 Reading a presented manifest

An `artifact_refs[].path` in a manifest is untrusted until checked.
A reader:

1. splits it on `/` only;
2. rejects an empty path, an absolute path, a backslash, a NUL, an
   empty segment, a trailing `/` and any segment failing §5.3;
3. rejects a depth above 2;
4. opens it from the generation's directory handle no-follow,
   component by component, and rejects any symbolic link;
5. verifies `bytes` and `sha256` against what it read (HYD-01).

Any failure is `UNSAFE_ARTIFACT_PATH`, HYD-01's "invalid … path in
a presented manifest" error, never a skipped artifact.

### 5.6 The root

The artifact root is the storage owner's directory (#9198). It is
opened once per command and canonicalized once. The root itself may
be a symbolic link that the operator configured. No link **below**
it is ever followed (§5.4, §5.5), and HYD-03's publisher already
rejects symlinks in staging.

## 6. What changes for neighbours

- **HYD-01** (open PR #15184): replace `diffs/pr-<n>.patch` with
  `diffs/<§5.1 name>.patch`. Each `sources[]` entry gains
  `source_key` (the §4.1 string) beside its display `source_ref`,
  so a reader can map an artifact to its identity without parsing
  the display URI. If HYD-01 freezes without that field, the key is
  still recoverable from the artifact name's digest by recomputation,
  and this record's oracles do not depend on the field. These are
  requests to HYD-01, not edits to it.
- **HYD-03**: consumes §4.3 for `<work-key>` and §5.3 for every
  component it creates. Nothing in its protocol changes.
- **HYD-04**: names every per-source artifact through §5.1 and
  records every unresolved locator per §3.3.
- **HYD-06**: activity records key their source by §4.1, which gives
  revisions and tombstones a repository-scoped identity.
- **CLI-03 / #9206 resource-ref normalization**: owns parsing a
  user's locator (`owner/repo#n`, URL) and resolving it. This record
  starts after resolution and owns only the resolved tuple's key.

## 7. Errors

All are named, matchable kinds in the shared result envelope (CLI-02),
never strings to parse:

| Kind                     | Raised when                                                               | Effect                                                   |
| ------------------------ | ------------------------------------------------------------------------- | -------------------------------------------------------- |
| `SOURCE_UNRESOLVED`      | a locator has no resolved identity tuple (§3.3)                           | per-source; the source is listed with no artifacts       |
| `UNSAFE_SOURCE_KEY`      | a key fails re-validation or round-trip (§4.1)                            | the key is rejected; nothing is named from it            |
| `UNSAFE_PATH_COMPONENT`  | a component built for writing fails §5.3 (a bug upstream)                 | the generation stops before publish                      |
| `ARTIFACT_KEY_COLLISION` | two keys give one name in one generation (§5.4)                           | the generation stops before publish; prior current stays |
| `ARTIFACT_EXISTS`        | create-new finds a file or link already at the name (§5.4)                | the generation stops before publish                      |
| `UNSAFE_ARTIFACT_PATH`   | a presented manifest path fails §5.5, or a link is met reading below root | the manifest is not presented as valid; no partial read  |

## 8. Oracles

All are proposed names. None exists yet. Each is listed by exact
name with `-- --list` before it runs, per the issue's done rule.

1. `pmo_hyd_artifact_keys_are_repo_scoped_and_traversal_safe`
   (first failing oracle). A property test generates pairs of
   tuples that share a number but differ in `repository_id`, with
   owner, repository and title locator text drawn from `../`,
   `/abs`, `a\b`, NUL and other control characters, very long
   strings and arbitrary Unicode. It asserts that:
   - distinct tuples give distinct keys and distinct names;
   - every built path stays under the root;
   - no name component contains a byte from any locator or title.
2. `pmo_hyd_key_is_stable_across_rename_and_case`: the same IDs with
   a renamed or re-cased owner and repository give the same key and
   name.
3. `pmo_hyd_names_never_collide_under_case_fold`: for generated
   distinct keys, the ASCII-lowercase of their names are distinct
   (names are already lowercase; the oracle pins that).
4. `pmo_hyd_long_label_keeps_digest`: a label forced past its cap is
   cut before the digest, and the name still ends `--<32 hex>.<ext>`
   within 96 bytes.
5. `pmo_hyd_diff_key_binds_head_sha`: one PR at two heads gives two
   keys; one PR at one head always gives one key.
6. `pmo_hyd_source_key_digest_golden`: fixed tuples give fixed
   `sha256:` digests under `fleet.source-key/v1` framing, so Rust
   and any later TS reader agree byte for byte.
7. `pmo_hyd_unresolved_locator_names_nothing`: a locator with no
   resolved repository ID yields a manifest source with no
   `artifact_refs` and writes no file.
8. `pmo_hyd_manifest_path_rejects_traversal_on_read`: `../x`,
   `/etc/x`, `a//b`, `a\b`, `.hidden`, `-x`, a NUL, upper case, depth
   3 and a trailing `/` are each `UNSAFE_ARTIFACT_PATH`.
9. `pmo_hyd_planted_symlink_is_refused`: a symbolic link planted at a
   name in staging, or at a type directory, makes the write
   `ARTIFACT_EXISTS` or `UNSAFE_ARTIFACT_PATH`. The link target is
   never written or read.
10. `pmo_hyd_key_collision_refuses_overwrite`: with the name digest
    function replaced in test by a constant, two keys collide. The
    generation stops with `ARTIFACT_KEY_COLLISION`, and the first
    artifact's bytes are unchanged.
11. `pmo_hyd_work_key_scopes_authority`: one `work_id` under two
    `authority_id` values gives two work keys, and an `authority_id`
    containing `/`, `..` or Unicode never reaches the path.

## 9. Requirement trace

The issue allocates C-R005, C-R009, C-R107, C-R112, C-R114, C-R119,
H-R017, H-R024, H-R031, O-R023, P-R004, P-R009 and P-R016. This
record serves them as follows:

- **C-R119** (per-source cursors, event identity and provenance):
  §3 and §4 give every source a stable, repository-scoped identity
  that cursors (HYD-05) and activity records (HYD-06) key on.
- **C-R112 / P-R004 / P-R016** (durable context bundle with manifest
  and resource files): §5 names every resource file so many
  repositories share one bundle without overwrite.
- **C-R114** (short summary plus actual file paths): §5.1 names are
  short, stable and safe to hand back as paths.
- **C-R005 / C-R107 / H-R017 / O-R023 / P-R009** (cohesive CLI
  families): the keys are the identity that `work hydrate`,
  `work changes` and `work link` (CLI-12, which lists HYD-02 as
  supporting) share. This record adds no command.
- **C-R009** (hydrate GitHub context into files): §3.3 and §5 are the
  file side of that requirement.
- **H-R024 / H-R031** (keep every original failure case in
  acceptance): §7 and §8 carry the source's collision and traversal
  failure cases as named errors and oracles.

## 10. Gates, non-goals and next steps

Gates before implementation (from the issue and #9207 §8):

- HYD-01 (#9458) merged, with §6's two requests answered;
- the granular #9206 resource-ref normalization slice (CLI-03)
  resolving locators to the §3.1 tuple;
- the architecture registry amended with `fleet.source-key/v1`;
- #9198's artifact-root decision (§5.6).

Each gate is recorded, not waived, by this record.

Non-goals:

- no change to queue rows, repo-health dedupe keys or any schema;
- no GitHub client, no new crate (`fleet gh` stays the adapter);
- no garbage collection of old names (HYD-03 owns orphans);
- no provider beyond GitHub;
- no decision on how a transferred issue's IDs behave (§3.3).

Next steps once the gates clear:

- an implementation issue owning `src/work/context.rs` path-key
  helpers and the §8 oracles, starting with oracle 1;
- the registry amendment as its own small change to
  `architecture.md`, reviewed with HYD-01.
