# C006: F12.5 first new providers kv blob http.fetch notify — D1-023

[Main reference](../main.md#c006)

## Relevant transcript excerpts

<!-- workflow:excerpt C006 39 43 -->

[Transcript lines 39-43](../transcript.txt#L39-L43)

```text
- [ ] **D1-023 — F12.5 First new providers: kv, blob, http.fetch, notify** (XL, design)
  A bounded durable per-world KV store, content-addressed blobs over iroh-blobs, allowlisted http.fetch (off by default, no off-list redirects), and rate-limited notify. Each needs a ceiling, an audit entry, and bound, refusal and replay tests.
  - Status: not started. None of kv/blob/http.fetch/notify appears in CATALOG (pkgs/id/src/world_caps.rs:55-73) or in the dispatch match (world.rs:2526-2547).
  - Source: .opencode/plans/phase-12-platform-flexibility.md:38; .opencode/plans/phase-12-platform-flexibility.md:85
  - Prerequisites: D1-022 F12.4 Capability provider registry (manifested components) (in this list), D1-024 F12.6 Structured audit log with OpenTelemetry export (in this list)
```

<!-- workflow:excerpt C006 180 181 -->

[Transcript lines 180-181](../transcript.txt#L180-L181)

```text
**Questions for Drew:**
1. **F12.5 vs F12.6 order:** options are (A) plan sequence F12.5 → F12.6 or (B) D1-023's explicit F12.6 prerequisite before F12.5. **Recommend B**: make durable audit precede external-effect providers and reconcile the plan.
```

## Normalized purpose

Intake for bounded durable KV, content-addressed blobs, allowlisted http.fetch, and rate-limited notify. Reconciles open #1062 and carries the unresolved F12.5-vs-F12.6 ordering question with the design option-B recommendation.

## Initial thoughts

- Capture per-provider schema/ceiling/refusal/replay/audit as required design dimensions from the start.
- Do not let research block on the ordering question: research may proceed while implementation waits for Drew confirmation.

## Inferred suggestions

- Inference: treat the off-by-default and no-off-list-redirect rules as acceptance constraints on the later design.

## Stage results

Pending: research, synthesis, design, planning and implementation have not run.
