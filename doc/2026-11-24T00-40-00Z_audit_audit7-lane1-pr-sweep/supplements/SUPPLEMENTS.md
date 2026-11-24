# Supplements — AUDIT-7 Lane 1 (PR-SWEEP) raw gathered material

Per Drew's steer (2026-11-23 ~18:46 CST): all data gathered and files produced by the lane land in the dated-record folder as supplements, not only in the lane workspace. This folder is `supplements/` inside `doc/2026-11-24T00-40-00Z_audit_audit7-lane1-pr-sweep/`, alongside the main report (`2026-11-24T00-40-00Z_audit_audit7-lane1-pr-sweep.md`).

| File | What it is | Provenance |
|---|---|---|
| `pr-full-records-refetch.json` | Full PR records (title, body, comments, files, state, merged flags, base/head) for the census window across all four repos, all states | Refetched 2026-11-24 via `fetch_full.py` + `q3-full.graphql`, unioned with `fetch_merged.py` (all-states windows; GraphQL `states:[OPEN,CLOSED]` silently excludes MERGED PRs) and `fetch_missing.py` (per-number fetch for 19 census PRs older than the windows). 1,803 records total (code 1,092 · opencode 406 · openchamber 254 · fleet 51). All 103 PRs the sweep merged read back in this file as MERGED. See loss note below |
| `graphql-oldest-50-open.json` | Raw GraphQL response: oldest 50 open code PRs at refetch time (post-restart), used to classify the Nov 13–23 tail | `fleet gh api graphql` with `q2.graphql`, 2026-11-24 |
| `digest-oldest-50-open-at-refetch.md` | Human-readable digest generated from the JSON above (body + comments + files per PR) | Generated in-session from `graphql-oldest-50-open.json` |
| `census-open-after-sweep-code.tsv` | Open code PRs after the sweep (91 lines incl. 6 opened by other lanes during/after) — the post-sweep census | `fleet gh api graphql`, 2026-11-24T00:47Z |
| `q2.graphql` | The bulk query used for the in-session classification reads (body/comments/files/mergeable per PR, 50/page) | Written in-session |
| `q2-all-states.graphql` | Variant of `q2` with `states:[OPEN,CLOSED,MERGED]` | Written in-session for the supplement refetch |
| `q3-full.graphql` | The query used by `fetch_full.py` (adds state/merged/closedAt/mergedAt/base/head) | Written in-session for the supplement refetch |
| `fetch_full.py` | Recreated fetch driver (pagination + retry) that produced the first pass of `pr-full-records-refetch.json` | Recreated in-session (see loss note) |
| `fetch_merged.py` + `q3-full-all-states.graphql` | Second pass: all-states windows unioned in (MERGED PRs) | Written in-session for the supplement refetch |
| `fetch_missing.py` + `q4-one.graphql` | Third pass: per-number fetch of the 19 census PRs older than the DESC windows | Written in-session for the supplement refetch |
| `rebase_fix.py` | Conflict-resolution script for this record's own report PR (#13909) README rebase | As run in-session |
| `followup-issue-13903-body.md` | Verbatim body of follow-up issue #13903 (blocked batches A–H) | As posted via `~/tools/gh-post` |
| `readme_insert.py` | The script that inserted this record's line into `doc/README.md` on the laptop worktree | As run in-session |

## Loss note (what is NOT here, and why)

The lane's original working files were kept in `/tmp/audit7-lane1/` on the sandbox. A service restart on 2026-11-24 (~00:31Z) wiped `/tmp`, losing: the original census TSVs (`census.tsv`, `census-code-p2.tsv`), the first GraphQL mergeability snapshots (`gql-*.json`), the per-PR body pages (`body-*.json`), and the full 202-entry digest (`digest.md`) after it had been read and acted on.

Recovery: every substantive datum in those files came from the GitHub API and was re-gathered — `pr-full-records-refetch.json` above is that re-gathering (it also proves the merge outcomes: the 103 merged PRs read back as MERGED with their merge commits). The disposition ledger itself was already durable before the loss: merge SHAs and slice progress were posted to tracking issue #13872 at every slice boundary, and the ledger in the main report was written from those records. The two original fetch scripts (`fetch_bodies.py`, `fetch2.sh`) were not recoverable verbatim; `fetch_full.py` is their recreated equivalent and is labelled as such in its docstring.

Lesson recorded for the codification lane: lane working files belong in `~/workspace/` (durable), never `/tmp`, from the first command.
