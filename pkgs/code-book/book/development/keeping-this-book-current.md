# Keeping this book current

A book that drifts from the project is worse than no book: readers plan
against what it says. This book's claims are dated, and this chapter is the
sync rule that keeps the dates honest. It extends the house pattern — the
Fleet book's keeping-current chapter states the same rule for its binary:
the book is part of the same change as the thing it documents.

## The rule

**Any change to the project's shape updates this book in the same body of
work.** Concretely:

1. **The retrospective gains a week entry as each week closes.** The new
   entry goes into [Week by week](../retrospective/week-by-week.md) in the
   census's five fields — biggest things built; hardest challenges; where
   the time went; capabilities unlocked; what we can do now — with its
   merged-PR counts read from GitHub and its claims cited to issues, PRs,
   records or notes. A week with no evidence is entered as uncovered, in
   place, at its true length. The month rollup in [The story so
   far](../retrospective/the-story-so-far.md) is updated in the same pass,
   and any capability that landed joins [Capabilities: what we can do
   now](../retrospective/capabilities.md) with its week and citation.
2. **The merged list is re-censused when programmes change.** A new
   programme, a completed tier item, or a programme's cancellation triggers
   a re-census of [The merged priority
   list](../priorities/the-merged-list.md) — not a spot edit. Between
   re-censuses, status changes land as dated corrections that name their
   source, so the list never silently disagrees with its own census.
3. **Statuses are re-verified at each edit.** Every status this book states
   carries its census date (2026-11-23/24 at first publication). An edit
   that touches a status re-reads the tracker behind it and moves the date.
   An edit that cannot re-verify a status leaves the old date standing —
   it never freshens a date without a fresh read. In-flux claims (held,
   blocked, parked) are re-verified against current main and the living
   tracker immediately before any merge that restates them: the ID book's
   run is the standing lesson, where a landing reversed a status three
   chapters stated, and only a fresh-main merge plus a grep for the stale
   framing caught it.
4. **A system's chapter changes when its book or tracker moves.** The
   systems chapters are condensations of the per-system books. When a
   per-system book is rebuilt or its programme tracker is replaced, the
   matching chapter here is re-condensed in the same programme of work —
   from the book, not from memory.

## The gates

Two mechanical gates keep the book honest, inherited from the house form:

1. **`create-missing = false`.** Every chapter in `SUMMARY.md` must exist
   on disk; the build never invents stubs. Adding a summary entry without
   the file fails the build.
2. **The Nix build, with zero warnings.** The book builds as the
   `.#code-docs` package (see [Deploying this
   book](../operations/deploying-this-book.md)). The build-only workflow
   `code-book-docs.yml` runs it on `main`. A chapter that breaks mdBook —
   a bad link target, a missing file, an unescaped placeholder that mdBook
   reads as a tag — fails the build by name rather than shipping broken.

Internal links in this book are relative `.md` links, and every file starts
with its exact chapter title as its single top-level heading, equal to its
`SUMMARY.md` entry. Both are part of the contract: relative links keep the
book readable in the repository as well as on the rendered site, and the
title rule keeps the summary, the file and the rendered page in agreement.

## Where the rule is recorded

This chapter is the record for this book. The per-system books carry their
own versions of the rule (the Fleet book's chapter is the exemplar), and
the programme tracker #13905 carries this book's commission. If this
chapter and a per-system book's rule disagree about that system's chapter
here, the per-system rule wins for its own book and this chapter wins for
this one — and the disagreement itself gets fixed in the same pass it is
noticed.
