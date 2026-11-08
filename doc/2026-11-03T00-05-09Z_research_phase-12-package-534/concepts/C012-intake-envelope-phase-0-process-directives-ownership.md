# C012: Intake envelope phase-0 process directives ownership and tracker policy — G bucket

[Main reference](../main.md#c012)

## Relevant transcript excerpts

<!-- workflow:excerpt C012 146 148 -->

[Transcript lines 146-148](../transcript.txt#L146-L148)

```text
[Comment by @dezren39 at 2026-10-27T04:10:20Z; https://github.com/developing-today/code/issues/534#issuecomment-6433533889]
Ordered queue now lives in #1235 (Path to Phase 12): the 14 prerequisite steps in claim order, with queue-packet priorities encoding the same sequence, then this epic's F12 items as tier 2. The validation audits are steps 4–7 (#1075 → #1076 → #1077 → #1078, phase order). Designs under this epic run continuously ahead of each F12 implementation.

```

<!-- workflow:excerpt C012 160 164 -->

[Transcript lines 160-164](../transcript.txt#L160-L164)

```text
[Comment by @dezren39 at 2026-10-27T08:49:36Z; https://github.com/developing-today/code/issues/534#issuecomment-6437548554]
🤖 **Agent note** · CORRECTION to my cross-reference note above (queue task 1258)

The F12.2 line in that note was wrong, based on a corrupted dedupe result: F12.2 forking/templates was **not** missing — it is tracked by dedicated issue **#556** (`id(world): F12.2 world forking and templates`). My duplicate filing #1354 has been closed as a duplicate of #556. For completeness, F12.3 also has dedicated remainder issue #835 and F12.5 has #1062, alongside D1-021/D1-023 here. Nothing on this umbrella needs adding for F12.2.
<!-- agent-note:v1 agent=repo-worker-code state=triaged task=1258 correction=1 -->
```

<!-- workflow:excerpt C012 185 204 -->

[Transcript lines 185-204](../transcript.txt#L185-L204)

```text
[Comment by @dezren39 at 2026-11-02T22:09:11Z; https://github.com/developing-today/code/issues/534#issuecomment-6589710037]
🤖 **Agent note** · triage verdict: CONFIRMED, routed
- **Agent:** queue worker (repo-worker-code), OpenChamber scheduled lane c29174f1-48e0-4b13-b4ff-9b73ab67e9ab
- **Session / ID:** queue task 2123 (triage packet for this issue)
- **Found via:** queue claim
- **Verified on current main 9f730f1c2:** `world.toml` manifest + `apply`/`diff` exist (cli.rs Diff/Apply, commands/world.rs manifest parse) — the body s "not started" for F12.1 is stale, as the 2026-10-26 intake review already flagged. No `ProviderRegistry`/`provider_registry` and no `plaza_abi_version` anywhere under pkgs/id — F12.4/F12.7 still code-absent, matching the merged design s reconciliation table. The issue body remains a stale snapshot; the comments + merged design are authoritative.
- **Routing:** design workflow decided and merged (doc/2026-11-01_design_id534.md, PR #3912 merged 2026-11-02): umbrella ledger + one shared research package, one concept per D1, phased plan 0–6, #534 stays open, no D1 ticked without committed artifact + linked implementation issue. No package exists yet in doc/, so enqueued execution slice task 6507 (repo-health-code-pilot, small_fix): open the package intake + phase-0 inventory/link reconciliation, carrying Drew s two open order questions (F12.5 vs F12.6; F12.17 prerequisite) with the design s option-B recommendations for confirmation. Dependencies gate implementation, not research — the questions are not research blockers.
- **Expectation:** #534 stays open as the umbrella; task 6507 stands up the package. No code changed by this triage; no D1 box ticked.
<!-- agent-note:v1 agent=repo-worker-code state=triaged task=2123 follow_up=6507 at=2026-11-02T22:18:00Z -->

[Comment by @dezren39 at 2026-11-02T23:14:55Z; https://github.com/developing-today/code/issues/534#issuecomment-6590906086]
🤖 **Agent note** · handed off
- **Agent:** queue worker (repo-worker-code), lane code-g
- **Session / ID:** task 6507
- **Found via:** queue claim
- **Task:** Open the #534 Phase 12 research package intake per the merged workflow design (PR #3912 verified MERGED; issue #534 verified OPEN, no hard-stop labels) — assigned to the opencode execution lane.
- **Expectation:** effort/l packet, package-opening slice only; the laptop claimant launches a bound opencode session that drives task 6507 to its terminal state. #534 stays open; no D1 box ticked without its artifact + linked implementation issue.
- **Links:** issue #534; PR #3912 (merged design)
<!-- agent-note:v1 agent=repo-worker-code state=handed-off at=2026-11-02T23:16:00Z -->

```

<!-- workflow:excerpt C012 130 145 -->

[Transcript lines 130-145](../transcript.txt#L130-L145)

```text
[Comment by @dezren39 at 2026-10-27T03:21:42Z; https://github.com/developing-today/code/issues/534#issuecomment-6432929359]
## Reprioritization: Phase 12 is the goal; the path to Phase 12 comes first (Drew, 2026-10-26)

The id queue has been re-ordered. **Adopt this order in all sweep / queue / orchestration work:**

1. **Path to Phase 12** — `now` + `priority/high`: validation tasks #1075 (Phases 0–8 + platform completion), #1076 (Phase 9), #1077 (Phase 10), #1078 (delivered Phase 11), umbrella #796; CI trust #229, #241, #551; journal format #227; cost visibility #553, #539, #542; projection remainder #242; remaining F11 #544, #555, #541; and this epic, #534.
2. **Phase 12** — `next` label: F12.1–F12.18. The full set now has issues: #807, #556, #835, **#1061 (F12.4 registry)**, **#1062 (F12.5 providers)**, **#1063 (F12.6 audit)**, **#1064 (F12.7 ABI)**, **#1065 (F12.8 SDKs)**, **#1066 (F12.9 upgrades)**, **#1067 (F12.10 scheduling)**, #805, #557, #806, **#1068 (F12.14 portals)**, #558, **#1069 (F12.17 trust)**, #560. Fix the F12.4 gate (#838) before the registry lands.
3. **Phase 13 + long-term ideas** — `priority/low`: #785–#790, #840, and the newly tracked ideas #1070 (group encryption), #1071 (TUI), #1072 (GUI), #1073 (Datastar), plus #793, #812. Do not pull these ahead of tiers 1–2.

**Every roadmap item now has an issue** — including the smallest, furthest-out ideas — so nothing lives only in a plan document. The roadmap of record is updated in PR #1079 (`planning/roadmaps/id.md`: Now = path to Phase 12, Next = Phase 12).

**Dashboard mirrors this list:** this org Project (Area `id-train`; Horizon Week = tier 1, Month = tier 2, Quarter = the rest) and the Grist Program roadmap (Area `id`, 7 rows: path-to-Phase-12, four Phase 12 rows, Phase 13, long-term ideas).

**Drop rule:** if an item turns out to be blocked, wrong, or superseded, park it (`status/parked`) or close it with a note saying why — that is always allowed and never needs permission. What is not allowed is letting one item silently stall the items behind it. Work the tiers in order and keep moving.


```

<!-- workflow:excerpt C012 156 158 -->

[Transcript lines 156-158](../transcript.txt#L156-L158)

```text
- **F12.2 Forking and templates** was **missing from the D1 checklist** and tracked nowhere — now filed as dedicated issue #1354 (spec + exit criteria from the plan, dependency F12.1 noted). Add it to the checklist or treat #1354 as its home.

<!-- agent-note:v1 agent=repo-worker-code state=triaged task=1258 ref=534 -->
```

## Normalized purpose

Global bucket for the merged-design workflow rules and tracker policy: exactly one shared package with one concept per D1, issue/comment/code reconciliation first, no duplicate implementation issues, independently mergeable designs, dependencies gate implementation not research, and the no-box-ticked-without-artifact-plus-link acceptance rule. Also carries corrected cross-references and the 2026-10-27 queue tier/ownership ordering.

## Initial thoughts

- Directives bind every stage but create no separate design deliverable.
- The D1 inventory stays exactly the 11 listed items; F12.2 stays with #556 and needs no checklist addition.

## Inferred suggestions

- Inference: later stages should treat the two option-B recommendations as reviewable defaults, not research blockers.

## Stage results

Pending: research, synthesis, design, planning and implementation have not run.
