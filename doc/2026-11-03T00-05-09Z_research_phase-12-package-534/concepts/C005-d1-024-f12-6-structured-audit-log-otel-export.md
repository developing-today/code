# C005: F12.6 durable structured audit log with OpenTelemetry export — D1-024

[Main reference](../main.md#c005)

## Relevant transcript excerpts

<!-- workflow:excerpt C005 34 38 -->

[Transcript lines 34-38](../transcript.txt#L34-L38)

```text
- [ ] **D1-024 — F12.6 Structured audit log with OpenTelemetry export** (L, design)
  A structured audit log of admin actions and provider use, exported via OpenTelemetry, building on F11.7; a test replays the log to the final grants.
  - Status: not started. `grep -rn 'opentelemetry\|OpenTelemetry\|audit' pkgs/id/src/world*.rs` finds nothing. Prerequisite F11.7 has not started (D1-007).
  - Source: .opencode/plans/phase-12-platform-flexibility.md:39
  - Prerequisites: #553 — D1-007 F11.7 Host metrics, Prometheus endpoint and tracing spans
```

<!-- workflow:excerpt C005 178 180 -->

[Transcript lines 178-180](../transcript.txt#L178-L180)

```text
**Phased plan:** (0) reconcile inventory and issue links; (1) F12.1/F12.3 deltas plus F12.4/F12.7 foundations, with F12.6 design in parallel; (2) F12.5/F12.14 after registry and audit; (3) F12.8 research and F12.9 after the ABI; (4) F12.10; (5) F12.17; (6) verify artifacts/links before ticking boxes.

**Questions for Drew:**
```

## Normalized purpose

Intake for the durable, replayable F12.6 audit log with the OpenTelemetry export as an adapter. Reconciles open #1063 and the F11.7/#553 dependency while keeping audit design parallelizable.

## Initial thoughts

- Separate durable audit facts (grants, attributions, provider use) from snapshots/metrics at intake so S2 does not conflate them.
- Record the option-B recommendation: durable audit precedes external-effect providers (F12.5).

## Inferred suggestions

- Inference: draft the event ownership/schema question as the first S1 research question.

## Stage results

Pending: research, synthesis, design, planning and implementation have not run.
