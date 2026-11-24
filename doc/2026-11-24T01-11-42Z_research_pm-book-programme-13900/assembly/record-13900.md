## PM-ASSEMBLY closing record — the Project Management book is landed and live

**PR:** [#13987](https://github.com/developing-today/code/pull/13987) — merged, merge commit `2dedbe152d953162730dfe23e2115d5bd749b1c8`.
**Published:** https://developing-today.github.io/code/pm/ (siteUrl `/code/pm/`, staged under `pm/` by the shared deploy job in `mcpx-docs.yml`).
**Chapters:** 58 + SUMMARY (59 files under `pm-stack/book/`).

### Layout (as ruled)

- `pm-stack/book/` + `pm-stack/book.toml`, mirroring `pkgs/fleet` (book.toml at the package root, chapters directly under `book/`, no `src/`).
- `pm-stack/docs.nix` + `lib/default.nix` wiring: flake output `.#pm-docs` (`pmDocs`).
- `.github/workflows/pm-docs.yml`: build-only workflow mirroring `fleet-docs.yml`.
- `.github/workflows/mcpx-docs.yml`: the shared deploy job builds the PM book with siteUrl `/code/pm/` and stages it under `site/pm/`. During assembly, PR #13989 added the Code Book and Lab Book to the same job; the merge conflict was resolved by keeping their steps and inserting the PM steps before configure-pages — the deploy job now stages mcpx (root), fleet, atlas, id, code, lab, and pm. The exact deploy-variant expression was built locally before merge (green, `/code/pm/` confirmed in the rendered 404 page), and the resolved YAML was parsed and its step list verified.

### Gates (bookwright)

- **mdBook build with ZERO warnings** — `nix build .#pm-docs` green on the final content (warning grep count 0); deploy-variant build also green. Rebuilt after every audit edit, including the prettier pass and the mid-assembly FLEET-SIDELINE-2 correction.
- **SUMMARY↔files exact** — 58 entries, set difference empty, run on the final formatted tree.
- **Internal links↔targets** — every internal link resolves; **H1 == SUMMARY text** for all 58 chapters; **link text == target H1** for every internal cross-link (audit script run pre- and post-format, exit 0).
- **Census↔chapters set difference** — empty: all 24 worker ledger rows (21 true Workers + exec door + pm-aggregator + dashboard duplicate) have entries in `reference/workers.md`; all 50 Turso database names from the settled estate list appear in `reference/data-stores.md`.
- **treefmt/formatters on touched files** — nixfmt clean on `pm-stack/docs.nix` and `lib/default.nix`; prettier clean on all book chapters and both workflow files (27 chapters took a formatting-only pass, commit `3e1a4774f71`). Note: `nix fmt` in this repo is a repo-wide lint-fix recipe, not treefmt-on-paths — it dirtied the assembly worktree once and was reverted untouched; the per-tool checks above are the gate as run.

### Fix-ups applied (coordinator list)

1. **Hedge re-grep** — all chapters re-grepped for reportedly/appears to/seems/roughly/probably/maybe/unclear and `-shaped`: zero hedge hits; the only `-shaped` uses are deliberate compound descriptors (route-shaped, bot-shaped, file-shaped), kept.
2. **data-stores / fleet-dispatch-jobs** — cited as the fleet book's gap with the fleet book as its home. FLEET-SIDELINE-2 then landed mid-assembly (PR #13978), so the chapter now names the landed fleet chapter: `concepts/dispatch-jobs-store.md` (The dispatch-test jobs store: fleet-dispatch-jobs).
3. **Top-up 1 folded** — T1 reconciled bootstrap order (10 steps) into `getting-started/installation.md`; T2 table inventory (10 projector `pmp_*` tables + 2 views; 9 shared-schema tables, live copies in Turso) into `concepts/shared-schema.md` + `concepts/projector.md`; T3 mechanical conflict rule into `concepts/projector.md`; T4 console verdict (pm-console = hand-installed user unit on 4872 + per-minute redeploy timer, NOT NixOS-declared) into `installation.md` + `getting-started/shape.md`; T5 port 4871 freshness IPC into `concepts/aggregator.md` + `concepts/projector.md`. The top-up's three still-unverified marks are kept (container first-start order; tunnel-install path; ready-for-agent native store schema).
4. **Top-up env folded** — `reference/configuration.md` now carries the full census: 318 verified names by component (aggregator 175, console 71, dispatch 23, projector 15, decision-events-worker 3, voice-line-worker 9, notify-relay 3, ready-for-agent 7, platform-scripts 28, nixos-pm 39; 373 rows), and the 13 docs-only names stay labelled docs-only in-chapter.
5. **Workers supplement merged** — the two D17 entries (`cold-store-rehydrator`, `developing-today-backup`) are full PM entries in `reference/workers.md`; the supplement file is deleted. The reconciliation is stated once and plainly: **24 ledger rows = 21 true Workers + the exec door (a tunnel ingress, not a Worker) + the pm-aggregator (a laptop service, not a Worker) + the dashboard row (a duplicate — the dashboard IS the aggregator UI; its only Worker, pm-console-edge, is a separate row). Census 22 = 19 Workers + 3 non-worker rows; registry 20; intersection 18.** The three non-worker entries carry the `survey/worker-evidence.md` verdicts (D18). Covering-book table updated for the landed sidelines: mcpx worker → mcpx book `worker.md` (PR #13906); id mail/worker-poc/d1-journal-poc → id book chapters (PR #13914); fleet rows → fleet book incl. `concepts/commander.md` (PR #13915) and the FLEET-SIDELINE-2 chapters `concepts/fleet-hosted.md` + `concepts/queue-transport-alert.md` (PR #13978).
6. **Disagreement citations normalised by content** — D1–D14 survey items unchanged (verified by content); D15 placement, D17 non-nested sets as canonical; the two `agg_*` two-directions citations renumbered D16→**D19** (program-roadmap, issue-map); LLM items cited at their content: D20 OpenAI fee, D21 Go caps, D22 Meta inactive-vs-serving, D23 Antigravity disabled-but-healthy, D24 SambaNova superseded, D25 LongCat superseded, D26 census timing PR #13691, D27 OpenRouter probe, D28 registry conflict markers, D29 pre-2026-11-07 pool language; D40 read-driver attribution (costs); D41 worker-count reconciliation (workers + costs); D42 Claude plan state across sources (costs + walls-and-resets); OC items D50–D58 verified against the OC survey §6 order — already canonical, unchanged. No citation's content failed to match the canonical list.
7. **dispatch-methods fleet URLs verified live** — queue, bridge concept, bridge reference, and queue-client URLs all fetched from the live fleet book (200, titles as linked); the previously UNVERIFIED fleet dispatch path is now linked: `concepts/dispatch.html` (Dispatch and dispatch-test) + `reference/dispatch.html`. Voice spot-check against `pkgs/fleet/book/concepts/queue.md`: the claim rule (priority ASC, created_at ASC) matches this book's priority-then-age statement.
8. **deploying-this-book** — UNVERIFIED names replaced with the real ones created here (`pm-stack/docs.nix`, `.#pm-docs`, `.github/workflows/pm-docs.yml`); its site tree now lists all staged books; the build-dir error in the draft (`book`) corrected to `rendered` per the fleet book.toml it mirrors.
9. **D40 check** — the #13844 audit record on main (`doc/2026-11-23T23-15-07Z_design_turso-d1-migration-audit/`) attributes the estate's read volume to `fleet-queue`: **81% of all reads (6.08B), the broker and its pollers**. costs.md's fleet-queue sentence is confirmed and now carries the figures + the D40 citation.
10. **Run 45299935945** (mcpx docs) — checked twice via `fleet gh api` during assembly: **status `queued`, conclusion null** at both reads (created 2026-11-24T00:46:55Z). It had not concluded; it sits in the same Actions backlog as this book's own deploy run. Stated plainly: no conclusion exists to report yet.

### Deploy verification

The merge's own deploy run **45306818203** (head `2dedbe15…`) was **cancelled** mid-run in the Actions backlog shuffle (several sibling runs were cancelled the same way); the deploy was carried by the next run, **45306993257** (head `cd8641257ab`, PR #13994's merge), which completed with conclusion **success** — ancestry verified: `cd8641257ab` descends from this book's merge commit `2dedbe15…`, so the deployed site includes this book. Live fetches at 2026-11-24 ~01:51 UTC, all serving:

- https://developing-today.github.io/code/ — 200, mcpx book (site root intact).
- https://developing-today.github.io/code/fleet/ — 200, "fleet - fleet".
- https://developing-today.github.io/code/atlas/ — 200, "atlas - atlas".
- https://developing-today.github.io/code/id/ — 200, "id - id".
- https://developing-today.github.io/code/pm/ — 200, **"Project Management - Project Management"** — the book is live; a deep page (`/pm/reference/workers.html`, "The workers") also fetched and correct.
- Also verified (newer siblings in the same deploy): /code/code/ — 200 "Code Book - Code Book"; /code/lab/ — 200 "Lab Book - Lab Book".

One cross-book staleness observed for the coordinator (not this lane's file): the Code Book's index still describes the PM book's published subpath as "not yet final" — it is now final (`/code/pm/`, live above).

### Skeleton delta

None. The skeleton's chapter list and the drafts reconciled exactly: 58 chapters, every drafted chapter included, in skeleton section order (LLM plans in the ruled order: the-plans, pools-and-routes, models-by-tier, benchmarks-and-evidence, walls-and-resets, costs, priorities). One draft file was consumed by merge, per the brief: `reference/workers-supplement.md` → merged into `reference/workers.md` and deleted.

— Lane PM-ASSEMBLY, programme #13900 (coordinator PM-BOOK), 2026-11-23.
