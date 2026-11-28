# C009: F12.10 fair cross-world scheduling per-account quotas and slow-client back-pressure — D1-028

[Main reference](../main.md#c009)

## Relevant transcript excerpts

<!-- workflow:excerpt C009 54 58 -->

[Transcript lines 54-58](../transcript.txt#L54-L58)

```text
- [ ] **D1-028 — F12.10 Fair cross-world scheduling, per-account quotas, slow-client back-pressure** (L, design)
  A hub scheduler that shares fuel/CPU fairly across worlds, with per-account quotas (worlds, storage, fuel/s) and back-pressure; exit: a spinning world cannot raise another's p99 by more than 10%.
  - Status: not started. pkgs/id/src/world_hub.rs bounds only sessions and open worlds (:12); `grep -n 'quota\|fair\|fuel_per_sec' pkgs/id/src/world_hub.rs pkgs/id/src/sandbox.rs` finds nothing. F11.7/F11.8 prerequisites are not started.
  - Source: .opencode/plans/phase-12-platform-flexibility.md:49
  - Prerequisites: #553 — D1-007 F11.7 Host metrics, Prometheus endpoint and tracing spans, #539 — D1-008 F11.8 Benchmark harness with committed baselines
```

## Normalized purpose

Intake for the hub scheduler, per-account quotas (worlds, storage, fuel/s), fair fuel/CPU sharing, and slow-client back-pressure with the p99 isolation acceptance. Reconciles open #1067 with F11.7/#553 and F11.8/#539 prerequisites.

## Initial thoughts

- State the seam explicitly: hub/host, not the current session-count limiter or metrics enumeration.
- Keep the p99 isolation threshold as the acceptance anchor from intake onward.

## Inferred suggestions

- Inference: capture current hub limits and metric enumeration at S1 as the baseline the design must supersede.

## Stage results

Pending: research, synthesis, design, planning and implementation have not run.
