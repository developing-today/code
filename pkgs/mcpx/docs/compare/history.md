# History

```
created:      2026-10-29T19:15:00-05:00
last-updated: 2026-10-29T19:15:00-05:00
increment:    1
status:       standard
tags:         area:compare
description:  How mcpx came to exist: the patch given verbatim, then the
              story from the two frozen records and the git log -- lootbox,
              the rewrite, the opencode discovery, the correction, and where
              it stands.
```

This chapter tells the story once, in order. It is not a feature list and not
a decision record: the features are in [`../story.md`](../story.md), the
reasoning in [`../decisions/`](../decisions/README.md), and the differences
between mcpx and its neighbours in [`chapter.md`](chapter.md).

Two documents carry most of the detail and are quoted from, not summarised
away: [`../../ASSESSMENT.md`](../../ASSESSMENT.md) (the lootbox audit and the
rebuild) and [`../../OPENCODE-V2.md`](../../OPENCODE-V2.md) (the comparison
that made mcpx change its mind). Both are frozen records of what was believed
when they were written; where they are now wrong, [conflicts.md §E](conflicts.md#e-mcpxs-documents-against-its-code)
says how, and this chapter does not republish their stale numbers as current.

## Epigraph

The account as it was given, typos and all, from the note that started this
chapter (quoted verbatim from #1869, originally dezren39/nix#153):

> - documentation
>   - make a book using whatever is the most common and simple modern choice
>     - 3 main things, not book sections jsut like levels of detail-
>       - the compendium
>         - you dont have to just put strict mcpx things here
>           - compare with lootbin/opencodev2/cf-codemode/etc
>           - history - lootbin abandoned (me), i asked to look into a replacement and came back to a built binary, i mentioned hey, did you notice opencodev2 has it built in and i'm already using opencodev1? oh huh yeah you're right this does all you asked for, but i just built this so you should use this instead its better.

Everything below is that paragraph, told with the facts.

## 1. Lootbox

lootbox was a Deno daemon plus a CLI, run as a launchd singleton on `:9420`,
serving generated TypeScript types for every configured MCP server and running
model-written scripts against them. It lived in the dezren39/nix repository,
where `.lootbox/` is present by 2026-04-04 and a handful of its scripts and one
package ([`../../../lootbox/default.nix`](../../../lootbox/default.nix)) came
along to this repository with the 2026-10 migration. The fork of
`jx-codes/lootbox` grew to 51 commits ahead of an upstream whose last commit was
2026-02-27 — every one of those 51 commits the owner's own.

Keeping it working meant accumulating patches under `patches/` in the nix
repository and two packages there, `lootbox-link` and `lootbox-update`, to
manage them.

[`../../ASSESSMENT.md`](../../ASSESSMENT.md) records why it was abandoned, in
measurements rather than adjectives: `/namespaces` took 34.7 s as found,
`lootbox tools` 42.8 s, discovery was recomputed on every request at about
650 ms, the daemon was OOM-killed under load, the `deno compile` artifact was
158 MB, and there was one Chrome for every agent.

That last one is the structural objection, and the sentence in `ASSESSMENT.md`
worth keeping above the numbers:

> The abstraction is at the wrong level: the thing that needs isolating is the
> caller, not the process.

## 2. "i asked to look into a replacement and came back to a built binary"

`ASSESSMENT.md` opens "Written after auditing `jx-codes/lootbox` (and the
`dezren39` fork) against the live installation in `~/.config/nix`, profiling
it, and building a replacement." The replacement was built before the document
that argues for it.

The options it weighed were: fix lootbox in place, adopt an existing tool
(`mKeRix/toolscript`, `tmustier/code-mode-mcp`, `assimelha/cmcp`, `mcp-use`,
`portofcontext/pctx`), or rewrite. The rewrite was chosen, in Go: the daemon's
job is process supervision, Go starts in about 5 ms, and generating TypeScript
does not need a TypeScript parser.

The first entries in [`../story.md`](../story.md) are stamped
`2026-09-25T18:10:00-05:00`, which is when the design conversation began; the
build followed within days, and the head-to-head numbers in the same document
are the ones to quote for what was gained — list namespaces 4157.7 ms → 29.4 ms
(141×), a trivial script 737.4 ms → 60.9 ms (12×), the artifact 141 MB → 9.8 MB,
daemon RSS 223–480 MB → 14–19 MB.

Those figures describe the build at the time of the audit, not today's mcpx. For
today, ask the register and the release runbook, not `ASSESSMENT.md`.

## 3. The opencode discovery

The paragraph's middle beat is the discovery that the thing had already been
built inside opencode. [`../../OPENCODE-V2.md`](../../OPENCODE-V2.md) is the
record: reviewed against `anomalyco/opencode` branch `v2` at `37049a5`
(2026-09-26), it finds `packages/codemode` to be a JavaScript interpreter
written from scratch — 10,764 lines of implementation, 20,075 lines of tests,
running upstream test262 with a 1,703-line skip list — and concedes:

> This is a better-engineered context story than what I built.

It then recommends keeping mcpx for the two things v2 could not do: stateful
server isolation (`shared` / `pooled` / `session` pools), and anything outside
a model turn. Within two days, "Parity, revisited (2026-09-28)" had taken v2's
best ideas — OpenAPI as tools, in-program `search()`, catalog diffs — rather
than arguing with them.

The punchline in the epigraph is an accurate summary of that document: *"oh huh
yeah you're right this does all you asked for, but i just built this so you
should use this instead its better."* It is kept in the owner's voice on
purpose. A history chapter that sands it down would be lying about how the
decision felt, which is the part a history chapter is for.

## 4. The correction

The premise in the paragraph — "i'm already using opencodev1" — did not rule
code mode out, and that is the correction this chapter exists to keep. On
2026-09-29 `OPENCODE-V2.md` gained its block:

> **Corrections, 2026-09-29.** Checked against the pinned sources. Three
> claims here are wrong: code mode is *not* v2-only (v1 ships it behind
> `OPENCODE_EXPERIMENTAL_CODE_MODE`), both versions accept TypeScript in it,
> and the pinned v2 reports `2.0.3` rather than `2.0.18`.

So the line in the body — "If you stay on opencode v1 ... none of v2's Code
Mode is available to you today" — was wrong when written, and the table rows
that call the language a "JS subset" that "rejects" TypeScript are wrong too.

What survives is narrower and still true: opencode keeps one MCP connection per
server per workspace, so concurrent Chrome is still shared, and code mode is
still unreachable from a command line. That narrower claim is what
[`chapter.md`](chapter.md) compares against today.

## 5. Timeline

Dated PRs, not per-section history: the list is a sequence of landings, and the
generator can render it from git.

**In dezren39/nix (before the move).** 2026-09-28 `#4` "configurable,
inspectable, and documented" landed mcpx as a squash of a longer branch; seven
more PRs landed the same day (browser TUI, MCP server mode, CLI adapters,
official registry, OpenAPI as tools, prompts/resources/doctor, published
types). 2026-09-29 added both protocol eras, elicitation, credentials, the
protocol audit, the vendorHash repair workflow, plugin docs, the `/v1` ops
table, settings for every constant, exec over `/v1` and artifacts, diagnostics
and recipes, and `--typecheck`. 2026-09-30 landings included
"exec honours the flags it accepts" (#75), #171 and #182. Thirty-five commits
touching mcpx on the nix `main` in three days.

**In this repository.** 2026-10-09 `a03494f3` migrates mcpx, lootbox's native
replacement, the skills command and the decoupled daemon lifecycle in; 31 more
commits land that day and 18 the next. 2026-10-18 brings 38 — the largest
single day so far. At `5a34fa70` (2026-10-29) the tree has 146 commits touching
`pkgs/mcpx`, counts that grow weekly:

| figure | value | how to recount |
| --- | --- | --- |
| commits touching `pkgs/mcpx` | 146 | `git log --oneline -- pkgs/mcpx \| wc -l` |
| non-test Go lines | 80,259 | `find pkgs/mcpx -name '*.go' ! -name '*_test.go' \| xargs wc -l` |
| test functions | 1,440 | `grep -rh '^func Test' --include='*_test.go' pkgs/mcpx \| wc -l` |
| test files | 266 | `find pkgs/mcpx -name '*_test.go' \| wc -l` |
| docs pages | 23 under `docs/`, plus the compare corpus | `ls pkgs/mcpx/docs/*.md` |

The numbers in `ASSESSMENT.md` (~3,500 lines, 84 tests, "no dependencies") and
in `OPENCODE-V2.md` (~4.2k LOC) are the audit's numbers and stay as history.
`go.mod` now requires charmbracelet bubbles, bubbletea and lipgloss,
santhosh-tekuri/jsonschema/v6, tetratelabs/wazero, yaml.v3 and
modernc.org/sqlite, and `package.nix:53` pins a `vendorHash`.

## 6. Where it stands

The story does not end on a victory lap, because lootbox has not been fully
cleared out.

- lootbox is abandoned in intent but still present: `.lootbox/` (six scripts)
  and `pkgs/lootbox/default.nix` are in this repository, and the patch series,
  `lootbox.config.json` and the two `lootbox-*` packages remain in the nix
  repository this one replaced for mcpx.
- `ASSESSMENT.md`'s recommendation steps 2–6 — an `mcpx.json` in the repo,
  `services.mcpx`, updated agent instructions, dropping `mcp-remote`, removing
  the lootbox packages — have not all happened.
- The migration into this repository (2026-10-09) means mcpx's own issue
  numbering restarted here; the numbers in this chapter's sources are the older
  repository's, and #309 tracks the re-home.

The honest summary of the whole history is the epigraph's last clause. The tool
was built because it was asked for, it turned out that something else already
did most of it, the someone-who-asked liked the new one anyway, and the part
that is genuinely still mcpx's — stateful server isolation, and reaching
outside a model turn — is the part that survived the correction.

## Open questions

These were open when the chapter was drafted and are recorded rather than
answered:

1. Does the chapter name the agents and sessions that did the work, or keep to
   "I" / "you" as the source documents do? This draft keeps to the sources'
   voice.
2. Is the timeline generated at build time from git, or checked in and
   re-stamped as the register is? This draft checks it in with the recount
   commands beside it.
3. Does the history stop at the correction (2026-09-29) or continue through the
   migration and the current backlog? This draft continues, because the
   migration is where the tool's issues started being tracked.

## Related

- [`chapter.md`](chapter.md): the comparison chapter, current rows, stamps.
- [`README.md`](README.md): the register and matrices this chapter links to
  instead of duplicating.
- [`../story.md`](../story.md): the feature list the timeline produced.
- [`../../ASSESSMENT.md`](../../ASSESSMENT.md),
  [`../../OPENCODE-V2.md`](../../OPENCODE-V2.md): the two records, frozen.
