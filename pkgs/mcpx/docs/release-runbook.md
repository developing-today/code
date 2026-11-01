# Release runbook

How `docs/story.md` stays true. `README.md` is a symlink to it, so this is the
first thing anyone reads about mcpx.

The file has two halves and they have different rules.

## The two halves

**Above `<!-- table-of-contents-marker -->` is the story.** One agent, one
prompt, one script, with the exact bytes it saw. It is the whole interface in
one read. It is *closed by default*: fix it, do not grow it.

**Below the marker is the feature list.** It is *open by default*: any commit
that lands on `main` may edit an entry or add one. It grows forever and that is
fine, because nobody reads past the part that answers their question.

## Rules for the story

Change it only to:

- correct something that is now false — a flag renamed, output changed, a
  measured number that has moved;
- replace a quoted block whose source changed. Every fenced block in the story
  is real output or a real file. Re-run the command, paste the result.

Do **not** add a new capability to the story. A new capability gets an entry
below the marker. The story stays one prompt long.

Extending the story needs explicit sign-off from the repository owner. If a
change would make the walk longer or add a second scenario, open it as a
question first.

When a quoted block changes, verify it rather than editing by hand:

```bash
mcpx ls
mcpx search screenshot click
mcpx types chrome_devtools | head -20
mcpx scripts
mcpx run shot-flow
```

The instruction file quoted in full is `AGENTS.md`. If that file changes, the
quote and the line-by-line breakdown after it both change.

## Rules for the feature list

Ordering is meaning. Roughly:

1. **Core** — what mcpx is for. Zero-context discovery, pools, the client.
   These change rarely and are read first.
2. **Standard** — shipped, supporting. Scripts, config, ops, packaging.
3. **Proposed** — wanted, not built. Everything under the `## Proposed` rule.

New shipped work goes after the last `standard` entry unless it is genuinely
core. New wants go under `## Proposed`. Within a release, append in merge order.

### Entry format

A small addition can be a heading and two links:

```markdown
### [feat(a11y): support for Teletypewriter](../docs/tty.md) #123

2026-10-02T11:30:00-05:00
```

Anything larger opens with a key block:

```markdown
## Session-isolated process pools

​```
created:      2026-09-25T18:30:00-05:00
last-updated: 2026-09-27T05:05:00-05:00
id:           session-isolated-process-pools
since:        pre-release
status:       core
tags:         area:pools, area:concurrency
description:  one sentence, present tense, what it does
​```

Prose, examples, the reasoning that is not obvious from the code.

2026-09-27T05:05:00-05:00
```

Required keys: `created`, `last-updated`, `id`, `since`, `status`, `description`.

Optional: `owner`, `creator`, `tags`, `dek`, `note`, `editors-note`, `links`.

`increment` is retired (removed in the #1865 migration). It counted commits
inexactly; `changed` below counts recorded changes of meaning, exactly.

### History metadata: `id`, `since`, `changed`

Every section carries a stable ID, frozen from the heading's slug the day the
metadata landed, so links never break when a heading is reworded:

- `id` — the section's ID, first key in the block.
- `since` — the first release containing the section: a version (`0.2.0`), or
  `unreleased` for work not yet released, or `pre-release` for everything that
  predates the first release (the migration sentinel; no release has shipped).
- `changed` — the section's recorded history, one list item per change:

```markdown
changed:
  - 0.2.0 (2026-11-02): idle exit is now a setting
```

Headings without a key block use the compact comment form, directly under the
heading:

```markdown
<!-- id: pools-lifetime since: 0.2.0 -->
```

A change is recorded per PR as one fragment file, `docs/changes/<slug>.md`:

```markdown
sections: pools, pools-lifetime
kind: Changed
summary: idle exit is now a setting
```

`kind` is a Keep a Changelog kind (`Added`, `Changed`, `Deprecated`,
`Removed`, `Fixed`, `Security`). At release time `stamp` folds fragments into
the named sections' `changed` lists and deletes them. An edit that changes no
meaning (typos, reflow, link repair) needs no fragment: mark the PR with the
`docs:trivial` label or a `Docs-Trivial: yes` trailer.

`status` also accepts the closed-proposal values `shipped`, `implemented` and
`rejected` (used by `proposals.md`); a status that needs more words becomes a
vocabulary `status` plus a free-text `note` key.

The tool is `tools/docs-history` (one Go program, `render` / `stamp` /
`check` on a single parser; not part of the shipped binary). `render` emits
the `since` marker and one closed-by-default `<details>` history per heading
whose subtree has history — its own entries in full, then one line per
descendant change grouped by release — and with `--agent` the agent-twin form
(no disclosures, a since/status line and a history link per section). It also
speaks the mdBook external-preprocessor protocol on stdin/stdout.

`tags` are Datadog-style: bare tags (`area:pools`) and key-value labels
(`platform:nix`, `cost:context`). Reuse existing keys before inventing one —
current keys are `area`, `platform`, `cost`.

`status` is one word:

| status | meaning |
| --- | --- |
| `core` | mcpx is not mcpx without it |
| `standard` | shipped and supported |
| `candidate` | shipped, may still change shape |
| `alpha` / `beta` | shipped behind a flag or unstable |
| `proposed` | wanted, not built. Say so in the prose too |
| `partial` | some of it works; the entry says which part |
| `deprecated` | works, will be removed. Link the replacement |
| `broken` | known broken, not yet removed |
| `retired` | removed. Struck through, or moved to `docs/archive.md` |
| `shipped` / `implemented` | closed proposal: built (proposals, document-level blocks) |
| `rejected` | closed proposal: decided against |

**Every top-level entry ends with its `last-updated` timestamp on its own
line**, RFC 3339 with offset. It duplicates the key block on purpose: the key
block is for machines, the trailing line is for whoever is scrolling.

### Removing things

Never delete an entry. Either strike the heading through:

```markdown
## ~~Old thing~~
```

and set `status: retired` with a note pointing at the replacement, or move the
whole entry to `docs/archive.md` and leave a one-line stub linking to it.

Heavy refactors move the old text to the archive too. The archive is append-only
and has no ordering rules.

## Per-commit

Any commit that lands on `main` and changes behaviour:

1. Edit or add the entry below the marker.
2. Set `last-updated`, update the trailing timestamp, and add a change
   fragment naming the section IDs you changed (`docs/changes/<slug>.md`),
   or mark the PR trivial (`docs:trivial` label / `Docs-Trivial: yes`
   trailer). CI runs the docs-history `check` on PRs touching `docs/**`.
3. If it changed something the story quotes, re-run the command and paste the
   real output.
4. If it changed `AGENTS.md`, update the quote and the breakdown.

A commit that only changes behaviour covered by an existing entry does not need
a new one; it needs that entry updated.

## Per-release

1. Run the docs-history `check` (see Checks): every entry touched since the
   last release must carry a change fragment or a trivial marker, and every
   `since` must be valid.
2. Re-run the story's commands and diff against what is quoted. This is the
   real check — the story is the only part that can silently rot.
3. Move anything now `retired` into `docs/archive.md`.
4. Promote `proposed` entries that shipped: change `status`, move them up into
   the shipped run, set `created` to when the work landed rather than when it
   was first wanted.
5. Run `stamp` with the release version and date
   (`go run ./tools/docs-history stamp --version X.Y.Z --date YYYY-MM-DD docs/*.md`):
   it replaces `since: unreleased`, folds the fragments into `changed` lists
   and deletes them. Commit the stamped docs, then tag that commit — the
   tagged tree is the stamped tree.
6. Re-read the first screen. If the first thing a newcomer sees is no longer
   the most important thing, reorder.

## Checks

```bash
cd pkgs/mcpx
gofmt -l .
go vet ./...
go test ./... -count=1
MCPX=./bin/mcpx scripts/stress.sh      # concurrency, leaks, orphans
MCPX=./bin/mcpx scripts/bench.sh       # latency, against lootbox
(cd ../.. && nix build .#mcpx)         # unit suites in the sandbox
```

The docs-history check (duplicate IDs, bad `since` values, fragments naming
unknown sections, and — with `--diff` — prose changes with no fragment and no
trivial marker):

```bash
go run ./tools/docs-history check docs/story.md docs/proposals.md
git diff origin/main...HEAD -- docs/ > /tmp/docs.diff
go run ./tools/docs-history check --diff /tmp/docs.diff docs/story.md
```

`README.md` must stay a symlink to `docs/story.md`:

```bash
test -L README.md && readlink README.md   # -> docs/story.md
```
