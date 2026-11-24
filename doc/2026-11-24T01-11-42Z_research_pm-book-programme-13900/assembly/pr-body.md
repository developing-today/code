## Project Management book (#13900, lane PM-ASSEMBLY)

The fifth house book, assembled from the programme's finished drafts and landed in the fleet book's shape.

**Layout (ruled):** `pm-stack/book/` + `pm-stack/book.toml`, mirroring `pkgs/fleet` (book.toml at the package root, chapters directly under `book/`, no `src/`). 58 chapters + SUMMARY.

**Plumbing:**
- `pm-stack/docs.nix` — mirrors `pkgs/fleet/docs.nix`; flake output `.#pm-docs` wired in `lib/default.nix` (`pmDocs`).
- `.github/workflows/pm-docs.yml` — build-only workflow mirroring `fleet-docs.yml`; never deploys.
- `.github/workflows/mcpx-docs.yml` — the shared deploy job gains two steps after the id book: build `pm-docs` with siteUrl `/code/pm/` and stage under `site/pm/`. Published URL: https://developing-today.github.io/code/pm/

**Assembly fix-ups applied:** hedge re-grep (clean — only deliberate `-shaped` compounds remain); `fleet-dispatch-jobs` cited as the fleet book's gap, its chapter landing in the same programme (FLEET-SIDELINE-2); survey top-up 1 folded (T1 reconciled bootstrap order → installation; T2 table inventory → shared-schema + projector; T3 mechanical conflict rule → projector; T4 console verdict → installation + shape; T5 port 4871 freshness IPC → aggregator + projector; the top-up's three still-unverified marks kept); env census folded into configuration (318 verified names by component; 13 docs-only names labelled docs-only); workers supplement merged into `reference/workers.md` with the final reconciliation stated once (24 ledger rows = 21 true Workers + the exec door + the pm-aggregator + the dashboard duplicate; census 22 = 19 Workers + 3 non-worker rows; registry 20; intersection 18); disagreement citations normalised to the coordinator's canonical numbering by content (incl. D19 for the `agg_*` two-directions item, D20–D29/D42 in the LLM chapters, D40 read-driver attribution confirmed against the #13844 audit: `fleet-queue` is 81% of all reads); fleet-book URLs in dispatch-methods verified live against the published fleet book; deploying-this-book names the real output/workflow.

**Gates:** `nix build .#pm-docs` green with **zero mdBook warnings**; deploy-variant expression (the exact string the deploy job runs) built locally green; SUMMARY↔files exact; internal links↔targets all exist; H1 == SUMMARY text for all 58; link text == target H1 for every internal cross-link; census↔chapters set difference empty (24/24 worker rows, 50/50 Turso DBs); both workflow files YAML-parsed after editing.

Programme record goes on #13900 after merge + live verification of all five books.
