# Human review — `fleet kv` design (#10359)

- Status: agent-preliminary; no human confirmation claimed. Work may proceed on the stated defaults; only a human moves one to confirmed or reversed.
- Scope: this sheet exists so a human can ratify or reverse the toss-ups without re-reading the full design. The full record is `2026-11-23T12-45-00Z_design_fleet_kv.md` in this folder.

## H1 — Sweep keying: owner+context vs session-expiry

- Default (D006): merge under `(owner_user_id, context_id, session_id)` provenance via durable outbox + CAS; sessions never evaporate silently.
- Alternative: session-scoped partitions that expire (simpler lifecycle, but a dead session's work product can vanish before anyone sweeps it).
- Why it needs a human: product call about whose work survives disconnect and for how long.
- Reversal cost: medium (re-key sweep + outbox provenance).

## H2 — Blob threshold default + flag shape

- Default (D009): per-namespace policy, 256 KiB agent-preliminary, per-push override flag; subordinate to the shared per-partition policy (component E owns the partition threshold; the 16 KiB conservative start in #9084 governs where both apply — the KV knob may only narrow, never widen).
- Alternative: lower default (16–64 KiB, closer to media plane) or higher (1 MiB, fewer references); flag as `--inline-max` vs `--blob-threshold`.
- Why it needs a human: default prices every large value (DB rows vs reference machinery); flag name is CLI UX.
- Reversal cost: low (policy value + flag alias).

## H3 — `--global` surface exactness (O1)

- Default: flag (`fleet kv get --global`), matching the issue sketch; exactness settles in Phase 1 build.
- Alternative: `fleet kv global <verb>` subcommand.
- Agent recommendation: flag (smaller surface, composes with `--namespace`).
- Reversal cost: low.

## H4 — Capability TTL + revocation expectations (O4)

- Default (D011): short-lived capabilities (minutes); revocation = reference removal + TTL-bounded lag (component-F posture).
- Alternative: longer TTLs (fewer mints, slower revocation) or instant-revocation machinery.
- Why it needs a human: security/product call; tier docs must not promise instant byte-level revocation.
- Reversal cost: high (auth architecture).

## H5 — Central tenancy at scale (O5, D002)

- Default: one fleet-kv Turso primary in v1; per-group DBs are provisioning scopes, not sync topologies.
- Alternative: per-session/per-group Turso DBs as the unit of sync (matches the issue's "own small database" intuition, multiplies primaries without settling per-key arbitration).
- Why it needs a human: cost + tenancy shape; needs Phase 2 load evidence.
- Reversal cost: high (splits the primary).

## Explicitly not asking

- D003/D004/D005/D007/D010/D012/D013: direct readings of the issue or settled ID/plane contracts; reversing one means reopening that contract, not this design.
- F12.5 `kv`/`blob` provider small caps (#1062) are a different surface (in-world sandbox ceilings) and are not ratified or changed here.
- Workload figures (five agents, key counts, write rates) are planning fixtures, not measured or free-tier guarantees.
