# C013: F12.13 journal replication to signed read replicas — D1-031

[Main reference](../main.md)

Post-intake addition (2026-11-10): this concept was not part of the
#534 intake eleven (C001–C012). It is the D1-031 research-package
artifact owed to transport tracker #536, whose D1-031 item is
designated "(XL, research package)" and had no concept in this
package. It is recorded in [changes.md](../changes.md); the
workflow state in main.md is untouched.

## Relevant source excerpts

Tracker item, issue #536 (transport designs and research):

```text
- [ ] **D1-031 — F12.13 Journal replication to signed read replicas** (XL, research package)
  Read replicas follow a single-writer world journal over a signed stream, serve spectators and survive writer downtime; tampered streams are refused; the F11.12 format rules apply.
  - Status: not started. The `Mirror` and `Download` world commands (cli.rs:1388 enum) copy modules and records, but there is no journal-follow stream: `grep -rn 'replica' pkgs/id/src/world*.rs` finds nothing.
  - Source: .opencode/plans/phase-12-platform-flexibility.md:55
  - Prerequisites: #227 — D1-012 F11.12 Journal format version header, check and backup commands, #534 (tracking issue) — D1-024 F12.6 Structured audit log with OpenTelemetry export
```

Decision-ready design: doc/2026-11-01_design_id806.md ("Design:
F12.13 journal replication (read replicas over a signed stream)",
2026-11-01, for implementation issue #806).

## Normalized purpose

Intake for journal replication as the read-scaling story of the
world journal: read-only replicas follow a single writer's journal
over a signed stream, verify and persist it, restore through the
existing world restore path, and serve spectator-only sessions —
including while the writer is down. The replica never becomes a
second writer. Replicate the authoritative journal, not the
current `Records` projection.

## Initial thoughts

- Publish only durable entries: the writer appends and fsyncs
  before acknowledging a mutation, so the stream publishes after
  fsync and treats compaction as a reset boundary, not as
  ordinary append-only tailing (design doc §2).
- Integrity is the contract's centre: tampered streams are
  refused at the receiver/verifier, and the F11.12 journal format
  rules (version header, bump-on-first-new-variant-write) apply
  unchanged to replicated journals.
- Outer framing aligns with the F12.6 audit-log framing contract
  (D1-024, concept C005 in this package), per the design's stated
  gate.
- The pre-existing `Mirror`/`Download` world commands copy modules
  and records; they are not a journal-follow stream and stay out
  of scope.

## Inferred suggestions

- Inference: writer-record parity (the replica's restored records
  match the writer's) is the acceptance anchor for the stream,
  ahead of any latency target — a replica that cannot prove parity
  is a cache, not a replica.
- Inference: spectator service during writer downtime should be
  specified as read-only-forever, never fail-over-to-writer; the
  single-writer invariant is what makes the signed stream simple
  enough to verify.

## Stage results

- Design: decision-ready design committed as
  doc/2026-11-01_design_id806.md (2026-11-01).
- Implementation (issue #806, in its own scope — recorded here as
  context, not as this package's deliverable): phase 1 PR #4447
  (replication wire contract + crypto primitives), phase 2 PR
  #4589 (serve signed world journal replication), phase 3 PR
  #4634 (durable replication receiver + verifier), phase 4 PR
  #4708 (read-only replica world service and spectators) — all
  four verified MERGED. Issue #806 remains open for its remaining
  scope.
- Research package: this concept is the committed D1-031
  research-package artifact for tracker #536 (2026-11-10, task
  14072); with it, #536's D1-031 criterion (research package
  committed + implementation issue #806 linked) is met.
