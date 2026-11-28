# agents-docs — hand-written agent twins

This directory holds the hand-written agent-markdown sources
("overrides") for the mcpx book's agent twins (#1947). `docs-twins
emit` uses the file here named by a page's book-relative source path
(`index.md` for the home page) instead of generating that page's twin
from its book source.

Per the design defaults recorded on #1947 (task 15179), overrides are
expected only for:

- the home page / install prompt (`index.md`), and
- the AGENTS.md-derived loop pages, when they join the book.

Any other override must state its reason in an `override-reason:`
comment near the top of the file; `docs-twins emit` and `docs-twins
check` refuse an override without one. That refusal is the drift
guard: a hand-written twin silently shadowing a generated one is how
agent docs go stale.
