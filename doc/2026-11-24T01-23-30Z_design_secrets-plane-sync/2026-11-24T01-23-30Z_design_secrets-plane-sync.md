# Secrets plane — delta refresh, broker peer sync, dual-store puts

Design record for [#13948](https://github.com/developing-today/code/issues/13948) (direction: Drew, 2026-11-23). Extends [#13893](https://github.com/developing-today/code/issues/13893) (cold-refresh batching + client fallback bound) and [#13894](https://github.com/developing-today/code/issues/13894) (broker performance under load); where the batching-only proposal and this design overlap, this design supersedes the batching-only framing. Companion future-work to-dos: [#13950](https://github.com/developing-today/code/issues/13950) (embed both stores in the fleet binary) and [#13952](https://github.com/developing-today/code/issues/13952) (`fleet init` bootstrap command). Related: [#10358](https://github.com/developing-today/code/issues/10358) (secrets scoping), [#9158](https://github.com/developing-today/code/issues/9158) (canonical secrets model).

## The problem

The sandbox secrets broker refreshes by shelling out to `bws secret get` once per secret — ~242 spawns, ~160–220s cold (measured 2026-11-23). During the refresh the broker is present-but-not-serving and clients hang instead of falling back (>120s observed), so every sandbox restart wedges every secrets consumer for minutes. The estate's two brokers (sandbox + laptop) hold the same set but cannot learn from each other, and `fleet secrets` is read-only, so writes propagate only when a refresh happens to notice them.

## Verified premises (2026-11-24, filing lane)

- `fleet secrets` today has exactly two verbs, `get` and `list`, and is read-only (installed binary's help). `refresh` and `put` are new surface.
- `bws secret list <project>` returns the whole set in one call, including values and a per-secret `revisionDate` (bws 2.1.0, verified live: 244 secrets; fields `creationDate, id, key, note, organizationId, projectId, revisionDate, value` — count and field names only inspected). Delta-by-`revisionDate` is supported by the CLI today.
- bws write verbs exist (`create / edit / delete`).
- 1Password CLI (`op` 2.41.0) is installed on both machines; `op item create/edit` and `op service-account ratelimit` exist (help verified). NOT live-verified: `op item list` JSON field shape — documented to carry `created_at`/`updated_at` with metadata-only list results; the laptop's `op` has no account configured in a plain shell. The design's Slice 5 includes a capability spike to verify before building on it.

## The design (summary; full text on #13948)

1. **Delta refresh.** Each broker keeps a watermark (last refresh start, minus a skew overlap). A refresh is one `bws secret list`; only entries with a newer `revisionDate` are applied, deletions come from the id-set diff, and a partial list applies nothing (the #5504 lesson).
2. **Broker peer sync.** On startup and each tick a broker asks its trusted peer for `{set_hash, last_upstream_refresh_at, count}`. Equal hashes mean synced. If the peer refreshed more recently, pull the peer's full set (v1 payload = everything, Drew's explicit simplification; selective pull-back deferred until requirements diverge). Peer down → upstream refresh. A sandbox broker needs a full upstream refresh only if the laptop is down AND it restarted with no cache.
3. **Set hash.** Sorted, keyed (HMAC) hashes over `(name, value)` pairs — the sync proof, exchanged only between authenticated brokers, truncated in logs, so a leaked hash doesn't enable offline guessing.
4. **Client surface.** `get --upstream`, `get --no-cache`, `secrets refresh` (reports which leg served it), and `secrets put`. The #13893 client bound (~2–5s, then fall through) rides along as Slice 1.
5. **Put semantics.** The receiving node writes the primary store first — that write is the commit point — then pushes the value directly to the peer broker and writes the mirror store; the peer never re-fetches. A node holding only one store escalates the put upstream to the laptop, which always has both.
6. **Dual-store, one primary.** Puts go to BOTH Bitwarden Secrets and 1Password, kept in sync by construction plus a periodic reconcile that repairs drift from the primary. **Recommendation: Bitwarden Secrets primary, 1Password mirror** — against Drew's initial lean, on evidence (the broker stack already runs headless on Bitwarden machine tokens; Bitwarden's list is the only one-call delta source; 1Password list is metadata-only and its service accounts are rate-limited; 1Password keeps its human custody/escrow strength as mirror and recovery path). Ratification is an open question on #13948, decider Drew. A Bitwarden-only node bootstraps store access via the canonical Bitwarden path (its access token is itself a vaulted secret, fetched by name).

## Build slices (dependency-ordered; acceptance criteria on #13948)

S1 client bound + one-call refresh (closes #13893) · S2 delta refresh + watermark · S3 keyed set hash + peer status · S4 peer pull sync · S5 client surface (`refresh`, get flags, `put`; includes the 1Password capability spike) · S6 reconcile + drift repair. Rollout is laptop-first, peer verbs behind a config flag, `put` dark until reconcile is live. Verification includes the two timed cold starts Drew's framing implies — peer up (serving in seconds, zero bws calls) and peer down (serving via upstream, one list call) — plus hash-equality proof, put propagation from each node class, a drift-repair drill, and a log audit proving a canary value appears nowhere.

## Security shape

No new trust root: peer sync rides the existing fleet enrollment and broker tokens. Values move only on the existing tailnet broker channel (WireGuard-encrypted) and broker↔store TLS. Logs carry names, counts, timings, truncated hashes — never values. Put requires enrollment-grade auth and is audit-logged by name + actor + timestamp.

*Secrets are referenced by name only throughout this record; no values appear here or on the linked issues.*
