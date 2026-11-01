# F12.6 slice 1: administrative and provider-use audit

## Request and intent

Implement queue task 4885 for issue #1063: add a structured audit record for
administrative actions and provider use, including refused admin commands, and
prove that replaying a scripted admin session reconstructs its final grant set
and ceiling. This slice does not include OpenTelemetry/OTLP export, a query or
UI surface, or auditing ordinary program traffic beyond provider use.

## Initial design

- Emit versioned, structured records through a reserved tracing target. Each
  record carries a timestamp, world identity, authority/actor identity, action,
  safe essential parameters, and outcome. Never include admin tokens,
  credentials, or secret values.
- Emit exactly one record for each admin command processed, whether accepted or
  refused. Emit provider-use records at the existing `CapUsage` accounting
  point, including grants, denials, and first-use grants.
- Capture the audit target in tests. Replay the records from a scripted admin
  session and compare the reconstructed grants and ceiling with the live final
  state; separately assert a refused command emits exactly one refusal record.

## Sink decision

Use the reserved tracing target rather than introducing a new world-journal
entry. The Phase 12 plan notes that new journal entry types are forward-only
for older binaries and should wait for F11.12 journal-format management. The
tracing target provides a single structured emission surface without changing
the journal format or admin behavior. Durable retention remains the configured
tracing subscriber's responsibility; OTLP export is a later slice.

## Change log

- 2026-11-01: Initial design for queue task 4885, before implementation.
