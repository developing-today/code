# Keeping this book current

The Lab Book describes physical things and a programme that will
change state in jumps: a circuit gets installed, a switch gets
racked, a node gets its first OS. A book like this goes stale
silently unless the sync rule is explicit. This chapter is that
rule.

## The rule

**Every hardware or cluster claim in this book is dated, and the
book is updated when the underlying record changes.** Concretely,
a change to any of the following is not done until this book
matches it:

- **A physical change at the site** — a machine inventoried,
  racked, moved or retired; a circuit installed; a switch racked,
  configured or removed; a drive installed or replaced; a board
  bought, flashed or run. The change's own ticket (for example
  #6073–#6075 for the Dell switches, #1095/#1096 for the
  inventory) records the fact; the book chapter that carries the
  claim is updated in the same pass.
- **A cluster state change** — a node installed, the TALM flow
  proven on bare metal, Cozystack deployed, a storage pool
  created, a workload landed. Status words move only when a
  record shows the new state: planned becomes in progress becomes
  proven on evidence, never on intention.
- **A census refresh** — when a new lab census supersedes the
  2026-11-23/24 census this book was built from, the chapters are
  re-derived from the new census: figures, statuses, the gap
  list and the roadmap timings are replaced, not patched over.
- **A programme change** — an epic opened or closed, the P9 draft
  ratified or withdrawn, a contradiction reconciled (gap 9's OCI
  pair is the standing example), a new owning issue filed (the
  auto-join gap in
  [Boot and auto-join](../cluster/boot-and-auto-join.md) is the
  standing example).

## How the update is made

1. Edit the chapter whose claim changed. Keep the status
   vocabulary of the [Lab Book](../index.md) introduction —
   proven, in progress, planned, researched-only — and keep the
   date of the record the new claim rests on in the text.
2. If a chapter is added, renamed or retitled, add it to
   `book/SUMMARY.md` in the same change, keep its sole H1 equal
   to its SUMMARY entry text, and update the link text in every
   chapter that links to it.
3. Stage the new files in git before building — Nix flakes only
   see tracked files, and an unstaged chapter fails the build as
   an invisible path.
4. Build and require zero warnings:

   ```sh
   nix build .#lab-docs
   ```

   A warning is a defect: an unclosed tag warning in particular
   means a bare angle-bracket placeholder is being read as HTML
   and text is silently vanishing from the rendered page. Escape
   placeholders in prose (`&lt;name&gt;`) rather than accepting
   the warning.

## What this book never does

- It never promotes a claim past its evidence. Research in
  hardware-doc is not ownership; a closed research issue's
  unextracted verdict is not a verdict (gap 8); a draft is not a
  decision (gap 13).
- It never carries cost material. Costs belong to the Project
  Management book.
- It never resolves a recorded contradiction by editorial
  choice. Contradictions are flagged as unreconciled until the
  programme reconciles them on their issues.
