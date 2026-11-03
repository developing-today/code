# C002: F12.4 capability provider registry with manifested components — D1-022

[Main reference](../main.md#c002)

## Relevant transcript excerpts

<!-- workflow:excerpt C002 19 23 -->

[Transcript lines 19-23](../transcript.txt#L19-L23)

```text
- [ ] **D1-022 — F12.4 Capability provider registry (manifested components)** (L, design)
  Providers become components with a manifest (name, version, argument schema, bounds, required scope, default ceiling), read by the ledger, compile gate and CLI, so adding a capability needs no edit to world.rs. One of the two highest-return tasks.
  - Status: not started. pkgs/id/src/world_caps.rs:55 `pub const CATALOG: &[&str]` is a hard-coded list, and execution is a string `match` in pkgs/id/src/world.rs:2526-2547. No registry type (`grep -rn 'ProviderRegistry\|provider_registry' pkgs/id/src` finds nothing).
  - Source: .opencode/plans/phase-12-platform-flexibility.md:37; .opencode/plans/phase-12-platform-flexibility.md:16; .opencode/plans/phase-12-platform-flexibility.md:80
  - Prerequisites: #540 — D1-010 F11.10 Split world.rs, world_session.rs, cli.rs (moves only), #554 — D1-013 F11.13 Error taxonomy: stable codes plus reasons on the wire
```

<!-- workflow:excerpt C002 130 140 -->

[Transcript lines 130-140](../transcript.txt#L130-L140)

```text
[Comment by @dezren39 at 2026-10-27T03:21:42Z; https://github.com/developing-today/code/issues/534#issuecomment-6432929359]
## Reprioritization: Phase 12 is the goal; the path to Phase 12 comes first (Drew, 2026-10-26)

The id queue has been re-ordered. **Adopt this order in all sweep / queue / orchestration work:**

1. **Path to Phase 12** — `now` + `priority/high`: validation tasks #1075 (Phases 0–8 + platform completion), #1076 (Phase 9), #1077 (Phase 10), #1078 (delivered Phase 11), umbrella #796; CI trust #229, #241, #551; journal format #227; cost visibility #553, #539, #542; projection remainder #242; remaining F11 #544, #555, #541; and this epic, #534.
2. **Phase 12** — `next` label: F12.1–F12.18. The full set now has issues: #807, #556, #835, **#1061 (F12.4 registry)**, **#1062 (F12.5 providers)**, **#1063 (F12.6 audit)**, **#1064 (F12.7 ABI)**, **#1065 (F12.8 SDKs)**, **#1066 (F12.9 upgrades)**, **#1067 (F12.10 scheduling)**, #805, #557, #806, **#1068 (F12.14 portals)**, #558, **#1069 (F12.17 trust)**, #560. Fix the F12.4 gate (#838) before the registry lands.
3. **Phase 13 + long-term ideas** — `priority/low`: #785–#790, #840, and the newly tracked ideas #1070 (group encryption), #1071 (TUI), #1072 (GUI), #1073 (Datastar), plus #793, #812. Do not pull these ahead of tiers 1–2.

**Every roadmap item now has an issue** — including the smallest, furthest-out ideas — so nothing lives only in a plan document. The roadmap of record is updated in PR #1079 (`planning/roadmaps/id.md`: Now = path to Phase 12, Next = Phase 12).

```

## Normalized purpose

Intake for the F12.4 provider registry: provider manifests, the catalog/dispatch replacement seam, and gate #838. Reconciles the open state of #1061 with the Phase 12 plan ordering.

## Initial thoughts

- Record the static CATALOG vs registry seam so S2 can design the manifest format and ledger/CLI consumption.
- Carry the #838 gate from the merged design so implementation sequencing is not lost at intake.

## Inferred suggestions

- Inference: baseline the current catalog and dispatch locations on origin/main during S1 so the research questions cite exact files.

## Stage results

Pending: research, synthesis, design, planning and implementation have not run.
