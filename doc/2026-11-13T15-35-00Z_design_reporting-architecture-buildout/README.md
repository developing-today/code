# Reporting architecture — full design set (design-mill build-out)

Status: BUILD-OUT IN PROGRESS (Summit task 16121, 2026-11-13). Elaborates the proto design
`doc/2026-11-13T15-12-00Z_design_reporting-architecture-event-driven/note.md` (PR #9577, epic #9568).

## Build order (V1 first — retires the most polling first)

1. C3 health hooks (#9571) + C2 GitHub-issues event queue (#9570)
2. C4 prediction logger against existing burn-plan data (#9572)
3. C6 report pages & spaces (#9574) + C7 rollups/design pages/timeline (#9575)
4. C1 end-state chat operating model (#9569) — reroutes themselves execute separately (Drew approved 2026-11-13 09:28 CST)
5. C5 efficiency + provider/model policy (#9573)
6. C8 Atlas ingestion contract, as input into Atlas v1 (#9091), not a parallel search program (#9576)

## Files

- `sources/` — working copies/pointers for this pass (canonical sources stay in the proto directory).
- `inventory-reverification.md` — statement inventory at atomic granularity; zero unmapped is the pass bar.
- `C1-chat-operating-model.md` … `C8-atlas-ingestion.md` — versioned component designs (V1 = what exists today; V2–V4 layer the vision).
- `state-of-the-art.md` — prior-art expansion shared across components.
- `coverage-audit.md` — designed vs implemented vs deployed vs live-verified + unresolved gaps (posted on #9568; epic NOT closed by the design pass).
- `fdrs/` — this program's rulings as FDRs per #9180 (chat map, reset rule, edit windows, hook thresholds).

## Rules

- Designs + issues only: no live queue-semantics, cron, hook, or production dispatch changes (per task brief).
- Research is its own issue class — filed, done, built on; never silently settled inside a design.
- Consume / extend, never duplicate: #1184, #9196, #9060 (+#9070/#9073/#9074/#9075), #9091, #9106, #9178, #9180.
