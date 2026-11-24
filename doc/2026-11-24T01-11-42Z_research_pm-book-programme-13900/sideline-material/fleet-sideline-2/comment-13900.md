## FLEET-SIDELINE-2 — complete: fleet book gaps + Drew's additions, one PR, merged

Lane FLEET-SIDELINE-2 (coordinator PM-BOOK), 2026-11-23. Worktree
`~/wt-fleet-gaps-book` on the laptop off `~/git/code`, branch
`book/fleet-gaps-additions`. No platform flag tripped.

**PR:** [#13978](https://github.com/developing-today/code/pull/13978)
— merged, merge commit
[`00c4873dd11bc3233f0f05ed7bf6da7cc21f80c3`](https://github.com/developing-today/code/commit/00c4873dd11bc3233f0f05ed7bf6da7cc21f80c3)
(2026-11-24T01:29:31Z). All chapters in `pkgs/fleet/book/concepts/`,
registered in SUMMARY.md, H1 == entry text (script-checked).

### Census gaps (material: FLEET-GAPS-SURVEY, D30–D35)

- `concepts/fleet-hosted.md` — **The hosted edition: fleet-hosted**:
  five doors, plumbing layer, dual ingestion, sessions/CLI,
  library-only direct-read, fixture seams; status stated as
  implemented/never deployed (D35), README stub table corrected (D31).
- `concepts/queue-transport-alert.md` — **The queue watchdog**:
  Access probe + failure classes, Durable Object state machine,
  three channels + retry discipline, recovery triage task, deploy/
  rollback; stale registry deploy record called out (D32).
- `concepts/dispatch-jobs-store.md` — **The dispatch-test jobs
  store**: the DB as its own subject (D30) — connection by name, the
  14-column `jobs` table, no-CREATE-TABLE provenance (D34), the
  #6598 write order, scan never opening the store, estate position
  (D33). Operational coverage stays in the dispatch chapters.

### Drew's additions (2026-11-23 ~19:01 CST)

- `concepts/temporary-builds.md` — **Temporary builds and custom
  connections** (Actions exec, #13774): runner-dials-out design under
  the public-log filter, the handoff (handle + `OWNER/REPO#RUN_ID` +
  run attempt), lease + max lifespan, the **tiered extension policy**
  as landed by PR #13854 (tier 1 auto+notify, tier 2 deny-window,
  tier 3 manual; tunables named), and `exec.security.cab` as a
  **path-scoped tunnel ingress, not a Worker** (WORKER-EVIDENCE
  verdict). The halted in-vivo proof is stated, not claimed.
- `concepts/compile-coordinator.md` — **The central compile
  coordinator** (#13765) from the fleet side: the four reasons, the
  three designed tables `placement_owner_state` / `placement_claims`
  / `placement_budgets` in fleet-queue, the five verbs + sweep, and
  the singular per-seat fallback contract (`placement_path`).
  **Designed vs built as of 2026-11-23: designed, not built** —
  grep-verified no `placement_coord` module and no DDL on main; the
  build (queue task 21517) is parked, its record naming a dedicated
  slice-1 task as the next step; the one shipped piece is the client
  claim attempt inside Actions exec.
- `concepts/lab-cross-links.md` — **Cross-links: automation,
  Cloudflare pipes and DNS**: link-don't-duplicate. Timing note: the
  Lab Book's part 1 landed mid-lane (PR #13968, merged 19:25 CST);
  the chapter was rewritten before merge to link its real chapters
  (Boot and auto-join; Talos and Cozystack; The cluster programme;
  Switches), states that the pipes have no dedicated chapter in
  either book yet (#8987/#8870/#13912, #5030 are the record), and
  names programme #13905 as the owner of the not-yet-written DNS
  coverage (`infra/dns/`, #902, #8808, #4989 are today's record). No
  invented URLs.
- `operations/program-issue-map.md` — dated addendum, "Additions
  after the census (2026-11-23)": prioritised working order +
  dependency map for the added items, in the book's style. (The
  file's pre-existing census table is not treefmt-clean on main; the
  addendum matches the file's own style and the table was left
  untouched rather than re-padding 203 unrelated rows.)

### fleet bridge subcommand state (asked for by the brief)

From the first sideline's record (PR #13915, merged
`189c055fee2a`): **`fleet bridge` already exists** — send, agents,
log, hello, inbox, post, ack, health, live-verified against the
installed binary, and its book chapters were audited accurate. The
only gap is the #13895 notes surface; the follow-up family
(get/send/receive/watch for class/name notes) is filed as
**#13907 and is still follow-up** — no notes code exists in
`pkgs/message-bridge/worker/src/` and #13907 stands open. This
lane's issue-map addendum carries it at working-order item 3.

### Build verification actually run

- `mdbook build` in the worktree: exit 0, **zero warnings** — run
  three times (initial, after treefmt, after merging current main +
  the Lab Book rewrite). Rendered HTML spot-checked: all six new
  chapters present; the `<window_start>` placeholder and the
  extension alert keys render as text.
- `nix build .#fleet-docs --no-link --print-out-paths`: green twice
  — `/nix/store/3hm1x1dv625yfc021iwifzcn4fdcfgjg-fleet-docs` on the
  first commit, `/nix/store/1jafkk0mggq0p17vp5g6wk14aisasvip-fleet-docs`
  on the final merged-main tree.
- Audits: H1 == SUMMARY entry for all six chapters; every internal
  `.md` link resolves; hedge grep — two deliberate keeps, both the
  sources' own terms ("connection-shaped" is #13774's term of art;
  "Turso-shaped" is the survey's).

### Disagreements

D30–D35 arrived with the survey and are carried in the chapters.
This lane's three are filed grouped as
[#13977](https://github.com/developing-today/code/issues/13977):
D36 (fleet roadmap links to a then-unlanded Lab Book — **resolved
by #13968 landing mid-lane**; correction posted on #13977), D37
(#13774 premise's Worker-gate door vs the landed tunnel-only door),
D38 (task 21517's packet named a wrong crate path).

Raw material deposited in the sandbox at
`~/workspace/pm-book/sideline-material/fleet-sideline-2/` (survey
inputs, all chapter drafts, the disagreements text, this comment)
for the programme's dated record supplement.

— Lane FLEET-SIDELINE-2, 2026-11-23.
