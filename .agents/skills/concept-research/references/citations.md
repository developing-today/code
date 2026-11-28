# Sources, citations and evidence (S2)

These rules adapt the cairn research spec (`~/git/cairn/SPEC.md`, rules 3 to 5 and 9, and its quality checklist) to concept documents. Cairn's spec is the long form; where the two disagree for a concept document, this file wins.

## Every fact has a label

- A fact from a source found in this run carries `[n]`, matching the numbered Sources list.
- A fact from memory, or a claim from the transcript or a note that no source in this run confirms, is allowed when marked `[unsourced]`. It never counts toward a facet's two sources.
- Your own reasoning starts with "Inference:".
- Nothing is dropped for lack of a source. It goes under "Unsourced ideas" with a case for and a case against.

## Cite only what a tool returned

- Every URL in the document appeared in a search result or a page fetch in this run, copied exactly. Never build, guess or normalize a URL, including GitHub or Hacker News URLs that look predictable.
- When a page redirected, cite the final URL.
- Record the publication or update date when the page shows one; otherwise write "undated".
- When a site blocks plain requests, follow the web-retrieval fallback protocol in `AGENTS.md` (the `web-retrieval-fallback` skill) and record what you tried in the research log.

## Sources are typed and weighted

Each Sources entry ends with its type: `docs`, `spec`, `paper`, `repo`, `issue`, `blog`, `forum`, `talk` or `anecdote`.

When sources conflict, weight them in this order and record the conflict in the facet and the research log:

1. docs and specs;
2. papers and blogs by the people who built the thing;
3. practitioner posts and repositories;
4. comments and anecdotes.

The text presents an anecdote as one ("one user reports [12]"), and every count of reports states where you looked.

## Quotes, numbers and repository facts

- A quote is copied exactly, at most 25 words, and is about the same thing as the claim it supports. Read the heading above it before using it. A paraphrase is not a quote.
- Every count states its sample and scope: "11 of the last 100 issues", never "the issues". A claim about a whole set needs the whole set queried, or it says "sampled k of N".
- A repository fact gives `path:line`, the SHA it was read at, and the command and output where one was run. A negative claim names the search ("`rg -n 'remote-notes' scripts/` returns nothing at `<sha>`").
- Re-measure anything a subagent or a note reports before you repeat it. Restating a report is not verifying it; keep its hedges.

## Two sources per facet

Each facet ends with at least two independent sources, or an explicit "one source only" or "no source found" with the queries tried. `[unsourced]` claims do not count.

## Numbering and later runs

- Number sources by first use. One number per exact URL; merge duplicates.
- Later runs append new sources after the last number. Never renumber: renumbering rewrites every citation line and hides what changed.
- A source that turns out wrong stays listed with "(withdrawn <UTC>: <reason>)", and the claims that used it are refined below, not deleted.

## Public repository

This repository is public. Nothing private goes in: no secrets, no credentials, no company-internal material, no personal data. When a source is private, describe it generically and keep the exact anchor outside the repository.

## Checklist before publishing

- Every factual sentence has `[n]`, `[unsourced]` or "Inference:".
- Every URL came from a tool result in this run.
- Every source ends with its type, and anecdotes read as anecdotes.
- Every quote matched its re-fetched page; every count has its sample and scope.
- Every facet has two sources or says why not.
- Every unsourced idea has a case for and a case against.
