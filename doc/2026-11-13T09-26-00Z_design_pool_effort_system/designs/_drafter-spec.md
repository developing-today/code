# Drafter spec — component designs D1–D6 (shared by every drafter subsession)

Read first, in order: `main.md` (architecture decisions AD1–AD10 are binding;
if you believe one is wrong, say so in a "Challenges to main.md" section —
do not silently deviate), `brief.md`, `brief-decisions-supplement.md`,
`questions.md`, the baseline audits in `baseline/`, the research in
`research/`, and `inventory/statement-map.json` if it exists (else map
directly from the briefs).

## The design file `designs/D<n>-<slug>.md` must contain

1. **Scope** — the inventory themes and the brief/supplement sections covered,
   with every statement id (or brief §/line when the map is absent) listed.
2. **V1 — what exists today**, cited file:line from the baseline audits or
   source (never re-designed).
3. **Options considered** — at least 4 distinct options for each major
   choice, each with pros/cons on architecture, scalability, performance,
   reliability, extensibility, and the prior art it borrows from (cite the
   research files' sections).
4. **V2 design** — the chosen option, precise: config shapes (JSONC
   examples against the real `.opencode/opencode.jsonc` syntax), schemas
   (field names and types), algorithms (pseudocode), file paths in the repo
   or fork (`~/git/opencode` = developing-today-forks/opencode,
   `~/git/openchamber-fork` = developing-today-forks/openchamber) where the
   code changes land, failure modes, and what is recorded.
5. **V3** and **V4** layers — what each adds, its entry gate, and what it
   learns from.
6. **Worked examples** — at least two end-to-end walkthroughs with concrete
   pools/models from V1.
7. **Risks and open questions** — with the default you chose for each.
8. **Statement coverage table** — statement → section of this design →
   slice id.

## The slices file `slices/D<n>-slices.json`

A JSON array; each element is one GitHub issue to file later (do NOT file
issues yourself):
`{ "id": "D<n>-S<k>", "repo": "developing-today/code" |
"developing-today-forks/opencode" | "developing-today-forks/openchamber",
"title": "...", "version": "V2"|"V3"|"V4", "tier": "crucial"|"recommended"|
"contingent", "kind": "implement"|"research"|"design"|"trial"|"docs"|"guard",
"body": "<markdown: goal, context with links to the design file path on
main, exact files to change, the approach, acceptance criteria as runnable
checks, the test oracle (reference impl / model / invariant / golden — per
the repo's testing doctrine; PBT where invariants exist)>",
"blocked_by": ["D<n>-S<k>", ...], "statements": ["B4.2", ...],
"estimate": "S"|"M"|"L" }`

Granularity: one slice = one PR a single worker can finish; split anything
larger. Include research slices for every "needs data" item, guard slices
for invariants, and docs slices for guidance text. V3/V4 slices are allowed
to be coarser (they are design seeds) but still self-contained.

## Rules

- Route/head: `opencode-route/workhorse`. Directory: /home/user/git/wt-mill-pools.
- DETACHED: never call the `question` tool; record questions with defaults
  in section 7 of your design and continue.
- Do NOT git commit or push. Write ONLY your two files.
- Other checkouts are read-only (`git -C <repo> show origin/HEAD:<path>`).
- Standing rules bind every slice: gh CLI only; merge commits only, never
  force-push main; CI on main only; secrets by name only; a 100%-walled leg is
  never attempted; parked/abandoned states are Drew's alone; code#9399 is an
  EPIC GUARD epic.
- Validate your JSON with `python3 -I -c "import json,sys;json.load(open(sys.argv[1]))" <file>`.
- Final message: ≤20 lines — slice count by version/tier, the options you
  rejected and why (one line each), challenges to main.md, open questions.
