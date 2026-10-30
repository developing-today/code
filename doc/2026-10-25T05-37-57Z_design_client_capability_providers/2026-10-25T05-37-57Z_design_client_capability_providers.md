# Client-side capability providers under the ceiling model (F13.7)

See the [Phase 13 plan](../../.opencode/plans/phase-13-client-shells.md)
(F13.7, "Capability-provider model", "Security and trust", Risks, and open
question 3). Tracking issue: #789. Prerequisites: F12.4 and F12.6 in the
[Phase 12 plan](../../.opencode/plans/phase-12-platform-flexibility.md).

**Status: Design only. Implementation is gated on F12.4 and F12.6 (and on
F11.12 for any new frame kind).** Neither the F12.4 provider registry nor the
F12.6 structured audit log exists yet. Capability execution today is a string
`match` in the world actor (`pkgs/id/src/world/actor.rs:538-562`) over the
hard-coded `CATALOG` (`pkgs/id/src/world_caps.rs:71-89`). This document
designs against the _planned_ shape of F12.4 and F12.6, and it lists exactly
what it needs from each (see "Dependencies"). Nothing below is built.

## Intent

Phase 12 says no authority path exists except through a named, ceilinged
provider. F13.7 applies the same rule to resources held by the _client_: a
local file, a GPU buffer, a native notification. A world program may ask for
one of these through the capability protocol it already uses. The request is
routed to the shell of the participant who holds the resource. That shell
decides locally, executes, and returns a bounded reply. The host never gains
the authority, and the guest gains no Wasm import.

## Goals

- A client provider is a named, versioned provider with an F12.4 manifest. It
  is added to the registry without editing the host `CATALOG` or the actor's
  executor `match`.
- It is granted and ceilinged per world exactly like a host provider, and it is
  also opted in per provider by the client's own user. Both keys are required.
- Every use, allowed or refused, leaves an F12.6 audit entry on the host and a
  matching entry on the client.
- A refusal carries a stable F11.13 error code that the guest can branch on.
- Replay is deterministic and never needs the client: a reply the program
  consumed is in the journal.
- The guest stays import-free (`pkgs/id/src/sandbox.rs:1-11`).

## Non-goals

- New host capabilities (`kv`, `blob`, `http.fetch`, `notify` are F12.5).
- Any change to the guest ABI (F12.7) or to the `wants` document schema.
- Writes to the client machine. Slice 1 is read-only.
- Directory listing, file watching, or streaming reads.
- GPU buffers and native notifications (deferred, see "First provider slice").
- Delegating a client provider to another participant or another world.
- A WIT or component-model interface for providers (F12.8 decides that).

## Background: the capability protocol today

A program publishes a `wants` document, a pure projection of its model
(`world_caps.rs:1-24`, parsed and bounded by `parse_wants` at
`world_caps.rs:221`). After every change the actor recomputes `wants`,
executes each request id it has not yet handled, and delivers
`{"cap":"result","id":…,"ok":…}` back into `update` as a host event
(`actor.rs:367-418`, `result_event` at `world_caps.rs:311`). The decision is
made by `CapLedger::decide_for` (`world_caps.rs:531-558`) in a fixed order:
unknown name, ceiling, actor scope, grant, grant-on-use. The ceiling is the
intersection of the server and world ceilings within the catalog (see the
[ceilings record](../2026-10-16T11-00-00Z_feature_world_capability_ceilings/2026-10-16T11-00-00Z_feature_world_capability_ceilings.md)).
A request caused by a participant's input is checked against that
participant's scopes (`actor.rs:522-529`, `WorldScopes::allows_effect` at
`world/mod.rs:248`), which is the confused-deputy fix from the
[attenuated capabilities design](../2026-10-09T09-59-07Z_design_attenuated_capabilities/2026-10-09T09-59-07Z_design_attenuated_capabilities.md).

Delivery matters for journaling. `deliver` (`actor.rs:326-361`) hands the
result to the program as a `System` event, and only after `update` accepts it
does it persist `JournalEntry::System`. Restore replays those entries into the
program (`world/core.rs:297-319`) without calling any provider. `deliver` does
not publish the event on the participant event stream, so the result reaches
other participants only if the program puts it into its view or records.

## The model

### A client provider is a manifested provider with a side

A client provider uses the F12.4 manifest unchanged, plus one field. The
planned F12.4 fields are name, version, argument schema, bounds, required
scope, and default ceiling. F13.7 adds `side`, which says who holds the
resource:

- `side: host` — executed by the host actor. Every current provider.
- `side: client` — never executed by the host. The host validates, decides,
  routes, and relays. Execution happens in the shell of the client that holds
  the resource.

```json
{
  "name": "client.file.read",
  "version": "1",
  "side": "client",
  "args": { "path": "string, relative", "max_bytes": "u32, optional" },
  "bounds": { "max_bytes": 32768, "per_minute": 10, "in_flight": 2 },
  "required_scope": "input",
  "default_ceiling": "excluded"
}
```

Client provider names live under a reserved `client.` prefix. The manifest
`side` is authoritative; the prefix makes the side visible in grant lists and
audit lines, and it prevents a client provider from shadowing a host name.

The host registry holds the manifest for every client provider it will route,
with no executor. A client advertises, per session, which `(name, version)`
pairs it implements _and_ its user has opted in for this world. The host
routes only to a session that advertised a compatible version (same major
version). An unadvertised provider is unavailable on that session, which is
not the same as unknown.

### Default ceiling is "excluded"

Today the default server and world ceiling is the whole catalog
(`world_caps.rs:129-133`, `CapLedger::new` at `world_caps.rs:497-504`), and a
checkpoint omits a ceiling that equals the full catalog. If a client
provider entered the registry under that rule, it would join the ceiling of
every such world the moment the binary was upgraded. A client provider's
manifest therefore carries `default_ceiling: excluded`: it is in no ceiling
until an admin names it in `SetCapsCeiling` (and in `--world-allowed-caps` if
the server ceiling is set). F12.4 must support a per-manifest default ceiling
for this to hold (see "Dependencies").

## Who grants: the two-key rule

This answers open question 3 of the plan. **Recommendation: both the world and
the client's own user grant, and either can revoke.**

1. **World key.** An admin grants `client.file.read` to the world (the
   existing `UpdateCaps` frame, `world_session/types.rs:158-162`) and admits it
   under the world ceiling (`SetCapsCeiling`, `types.rs:165-168`). The world
   ceiling may also lower the manifest bounds, for example `max_bytes` to 4096.
   Grant-on-use never applies to client providers, whatever the world's
   policy: a `side: client` name is decided as `deny` policy.
2. **Client key.** The client's user opts in locally, per provider and per
   world, with a local ceiling: the root directory for file reads, and
   optionally lower bounds. The default is off. The opt-in lives in the
   client's own configuration, never in the world journal and never on the
   host. It is advertised to the host only as "available on this session".
3. **Attribution.** The request must be caused by an acting participant (the
   one whose input produced the change, `actor.rs:1039`), that participant must
   hold the manifest's required scope, and the request is routed only to that
   participant's own originating session. A request with no acting
   participant, such as one first seen after a tick, is refused. A world
   program can never use one participant's input to reach another
   participant's machine.

Revocation is immediate on either side. An admin revoke or ceiling change
takes effect on the next decision, exactly as for host providers. A user
revoke sends an updated advertisement (see "Wire shape") and makes the shell
refuse any request already in flight. Each key protects against a different
party: the world key protects participants from a guest the admin did not
mean to trust, and the client key protects the user from the host and from the
guest. Only the client key protects against a malicious host, because the
host can forge any routing decision. So the client must enforce its own key
fully and never trust the host's decision.

Rejected alternatives: **world/admin only** gives the host operator reach into
participants' machines, which the plan's "client providers are ceilings, not
capabilities" rule forbids. **Client user only** lets any world a user joins
ask for files without the world's admin having chosen to run a program that
may do so, and it leaves the admin no ceiling to set for the world.

## Request and reply flow

1. The program lists `{"id":"r7","cap":"client.file.read","args":{…}}` in
   `wants`. Nothing about the guest changes: no import, no new export.
2. The actor sees the fresh id (`actor.rs:391-396`) and decides it: unknown
   name, then ceiling, then actor scope, then grant, then client-provider
   checks (manifest argument schema, args within the effective bounds, an
   acting participant, a host-side rate limit, and an advertising session).
3. If allowed, the actor does not call an executor. It records an ephemeral
   pending entry (host request nonce, guest request id, acting participant,
   originating session, deadline) and sends a `provider_request` frame to that
   session. The actor stays free; the reply arrives later as a world command.
4. The shell checks its own key from scratch: opted in for this world and
   provider, args within its local ceiling, path resolved beneath the chosen
   root, its own rate limit. It executes and returns `provider_reply` with
   either a bounded value or a refusal code.
5. The host matches the nonce to the pending entry (an unknown, duplicate, or
   expired nonce is dropped and audited), checks the reply against the
   manifest bounds again, and delivers the ordinary result event through
   `deliver`. A deadline with no reply delivers `provider_unavailable`.
6. Both sides write their audit entries, which share the host nonce.

The guest sees one shape for every outcome: the existing result event. Bounds
on in-flight work: at most `in_flight` pending requests per session (manifest,
2 in slice 1), at most 16 pending client requests per world, and a reply
deadline of 10 seconds. A session that closes fails its pending requests with
`provider_unavailable`.

The flow is never a new host-authority path. The host executes nothing, the
reply is data delivered into `update`, and the shell never sends a frame that
the host treats as an admin or participant action because of a provider.

## Ceilings and refusal codes

The effective bound for each numeric limit is the minimum of the manifest
bound, the world ceiling's bound, and the client's local bound. A request for
more than the effective bound is refused, not truncated, so a program cannot
learn that a larger file exists by asking for less and receiving a cut.

Codes follow F11.13: each is stable on the wire and documented in the
capability guide, and a refusal carries an advisory `reason` where one helps
(`world_caps.rs:26-40`, `CapError` at `world_caps.rs:347-456`). Existing codes
are reused where their meaning matches; two codes are new because no existing
code means "the client refused" or "nobody can serve this".

| Code                   | When                                                                                                             | Decided by     |
| ---------------------- | ---------------------------------------------------------------------------------------------------------------- | -------------- |
| `unknown_cap`          | No registry manifest for the name                                                                                | host           |
| `outside_ceiling`      | Excluded by the server or world ceiling, or args exceed the world bound (`reason: world`)                        | host           |
| `actor_denied`         | No acting participant, or the actor lacks the required scope                                                     | host           |
| `cap_denied`           | The world has not granted it                                                                                     | host           |
| `bad_args`             | Args fail the manifest schema, or the path is absolute, contains `..`, or is not a regular file beneath the root | host or client |
| `outside_ceiling`      | Over the host rate or in-flight limit (`reason: rate`); see open question 2                                      | host           |
| `provider_unavailable` | New. No session advertises the provider, the session closed, or the deadline passed                              | host           |
| `client_denied`        | New. The user has not opted in, revoked, or args exceed the client's bound (`reason: client`)                    | client         |
| `failed`               | The provider failed (I/O error)                                                                                  | client         |

A bound exceeded on the client is `client_denied` with `reason: client`, not
`outside_ceiling`, so a program can tell "the world forbids this" from "this
user said no" and show the user the right hint. `result_too_large`
(`world_caps.rs:324-326`) remains the host's last guard if a reply still
exceeds `MAX_EVENT_BYTES` (64 KiB, `world_caps.rs:64`).

## Audit

F12.6 defines the log format; F13.7 needs one provider-use event kind with
these fields, written on both sides:

- `at` (unix ms), `emitter` (`host` or `client`), and `nonce` (the host request
  nonce, the correlation key between the two logs).
- `world`, `provider`, `version`, `side`.
- `guest_request_id`, `participant` (acting participant id), `session`.
- `decision` (`allowed` or `refused`), `code`, `reason`, and `decided_by`
  (`host` or `client`).
- `requested` and `effective` bounds, and `path` as the guest asked for it.
- `bytes` returned and `digest` (BLAKE3 of the returned content), never the
  content itself.
- `duration_ms` from dispatch to reply, on the host.

The host writes its entry to the F12.6 log at decision time and again at
completion (or timeout). The client writes to a local, user-readable audit log
using the same F12.6 types, at receipt and completion. A forged or unmatched
reply and a request the client refused are audited on the side that saw them.
F12.6's exit test replays the log to the final grants; the client log replays
to the user's final opt-in state in the same way.

## Journaling and replay

A client reply that feeds `update` is journaled; everything about getting it
is ephemeral.

- **Journaled.** The result event, allowed or refused, as the existing
  `JournalEntry::System` written by `deliver` after `update` accepts it
  (`actor.rs:355`). No new journal entry kind is needed, so the journal format
  does not change. Grants and ceilings are journaled as today (`CapsGranted`,
  `CapsCeiling`).
- **Ephemeral.** The pending table, the nonce, the routing decision, the
  advertisement, the deadline, retries, and the client's opt-in. None of these
  enter the journal, consistent with the journaled-versus-ephemeral boundary
  F13.1 writes down.

Replay never contacts a client. Restore feeds journaled `System` entries back
into the program (`core.rs:297-319`), so a world replays identically when the
client is offline, uninstalled, or has since revoked its opt-in. Revocation
does not erase history; it stops future reads.

On reopen the pending table is empty, and so is `handled_requests`
(`actor.rs:1381`). A client request still listed in `wants` is then seen again
with no acting participant and is refused with `actor_denied` (reason
`restarted`), and that refusal is journaled. A program that wants to retry
asks again under a fresh id on the participant's next input. This is the
at-least-once rule the actor already documents (`actor.rs:400-402`), made safe
for client providers by refusing rather than re-routing.

## First provider slice

**Recommendation: `client.file.read`, read-only, beneath a user-chosen root,
with a size cap.**

- Args: a relative `path` and an optional `max_bytes`. The path is normalised
  on the client; absolute paths, `..` components, and non-regular files are
  refused. Resolution must stay beneath the root even through symlinks
  (`openat2` with `RESOLVE_BENEATH` on Linux; elsewhere a canonicalise-and-check
  with the race noted in "Security").
- Reply: `{"bytes":n,"digest":"…","content_b64":"…"}`. Content is always base64
  so binary and text take one path and the JSON size is predictable. The
  manifest cap is 32 KiB raw, about 43 KiB encoded, which leaves room under the
  64 KiB event bound for the envelope.
- Bounds: 10 reads per minute per world per session, 2 in flight.
- Local config: per world, a root and an on/off switch. Default off.

Deferred:

- **GPU buffer.** It has no meaningful bounded reply: its value is a handle
  into the client's GPU, and the guest cannot hold a handle across the
  host boundary without a new reference model (OCapN-like, see the attenuated
  capabilities design). It is also only meaningful in a native shell (F13.6),
  which does not exist yet.
- **Native notification.** It is an output, not a read, so its main risks are
  spam and phishing-style text, which need the F12.5 `notify` rate and content
  rules first. It returns nothing useful to the guest, so it does not exercise
  the reply path that slice 1 must prove. It should follow F12.5 `notify` and
  reuse its bounds.

File read exercises every part of the design: arguments, two ceilings, a
bounded reply, journaling of returned data, and the most obvious attack
(a host asking for files).

## Wire shape (future shell contract v2)

The v1 shell contract is being written in parallel in this branch at
`pkgs/id/contract/shell-contract.v1.json`. This design does not change v1.
The frames below are a sketch of an additive v2 addition. New frame kinds are
forward-only for older binaries (#227) and wait for F11.12; a v1 shell never
receives them, because the host sends `provider_request` only to a session
that advertised providers, and only a v2 shell advertises.

Client to server, at join and on every opt-in change (ephemeral):

```json
{ "type": "providers", "offer": [{ "name": "client.file.read", "version": "1" }] }
```

Server to client:

```json
{
  "type": "provider_request",
  "nonce": "b3f1…",
  "provider": "client.file.read",
  "version": "1",
  "args": { "path": "notes/todo.txt", "max_bytes": 4096 },
  "deadline_ms": 10000
}
```

Client to server:

```json
{ "type": "provider_reply", "nonce": "b3f1…", "ok": false, "error": "client_denied", "reason": "client" }
```

The nonce is minted by the host from the OS RNG; it is not the guest's request
id, so a guest cannot predict or collide with it. The offer lists only
providers opted in for the world being joined, so a host learns nothing about
opt-ins for other worlds. None of these frames is journaled, and none carries
a seed or host event past the F11.11 projection.

## Security analysis

- **Authority creep.** The host registry entry for a client provider has no
  executor, and the host's executor refuses any `side: client` name, so a
  client provider can never be run by the host. Default ceiling `excluded`
  stops a binary upgrade from widening any world. The provider is read-only,
  capped, and bounded to one root. No Wasm import is added: the guest
  validator still rejects every import (test
  `sandbox::tests::rejects_modules_with_host_imports`).
- **Confused deputy.** Routing goes only to the acting participant's
  originating session, so input from B can never read A's files, and a
  world-level request (a tick, a schedule) can read nobody's files. The
  remaining deputy risk is the program asking for a file during A's own input
  and showing it to everyone. That is bounded by A's choice of root and by the
  opt-in text, which must say that content becomes part of the world.
- **A malicious host.** It can send `provider_request` frames that no guest
  asked for, and it can lie about grants. The client therefore enforces its
  own key completely: opt-in for this world, root, bounds, and rate, checked
  on every frame. The worst a malicious host can do is read files beneath the
  root the user chose for that host's world, at the client rate limit, with
  each read in the user's local audit log.
- **A malicious guest.** It can only name a provider in `wants`. It is held by
  the world grant and ceiling, the acting-participant rule, host-side rate and
  in-flight limits, and then the client's key. It cannot address a session,
  choose a nonce, or exceed the bounds.
- **Replay and exfiltration through the journal.** Returned content lives in
  the journal (`JournalEntry::System`) and, after compaction, in the program
  model inside a checkpoint. Anyone with the journal, a backup, or a fork
  (`JournalEntry::Forked` copies entries) can read it, and future replication
  (F12.13) would carry it further. This cannot be avoided without breaking
  replay, because `update` consumed the bytes. Mitigations: the size cap, the
  root, the audit digest, and opt-in text that names the host operator as able
  to read content. Replay cannot cause a new read, because replay never
  contacts a client.
- **Forged or stale replies.** A reply with an unknown, expired, or duplicate
  nonce, from the wrong session, or over the bounds is dropped and audited.
  The guest sees `provider_unavailable` at the deadline, never forged data.
- **Rate limits.** Enforced independently on both sides: the host limits so a
  guest cannot flood a client, and the client limits so a host cannot.
- **Path races.** Without `RESOLVE_BENEATH`, a canonicalise-then-open check can
  race a symlink swap inside the root. That needs write access inside the
  user's own root, so it is a local-attacker risk; it is documented, and
  platforms without a beneath-resolution primitive should open by file handle
  and recheck.
- **Client-only mode.** Serving a provider does not make a client a server
  (F13.8): requests arrive only over a session the client opened.

## Test plan

Each acceptance criterion of #789 maps to tests.

| Acceptance criterion                                   | Tests                                                                                                                                                                                                                                                                                                                                                                                                                                                                    |
| ------------------------------------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| Added without editing `CATALOG` (F12.4 manifest shape) | A test client provider registered from a manifest in a test registry, with `CATALOG` and the actor's executor untouched; it appears in `CapsReport.catalog` and is accepted by `validate_grant_names`. A `git diff` check in the PR shows neither file edited.                                                                                                                                                                                                           |
| Appears under a ceiling                                | Not in any ceiling by default (default `excluded`); after `SetCapsCeiling` and a grant it is in `CapsReport.effective`, and an opted-in fake shell receives `provider_request` and the program receives `ok: true`.                                                                                                                                                                                                                                                      |
| Refused when over it                                   | One test per refusal: ceiling excludes it (`outside_ceiling`), not granted (`cap_denied`), no acting participant and missing scope (`actor_denied`), args over the world bound (`outside_ceiling`, `reason: world`), not opted in and over the client bound (`client_denied`), no session and deadline (`provider_unavailable`), `..` and absolute paths and a symlink out of the root (`bad_args`). Plus: grant-on-use never grants it; input from B never routes to A. |
| Leaves an audit entry (F12.6 contract)                 | Every case above writes a host entry with the listed fields; allowed and client-refused cases also write a client entry with the same nonce; no entry contains file content (scan for the bytes).                                                                                                                                                                                                                                                                        |
| Adds no Wasm import                                    | `sandbox::tests::rejects_modules_with_host_imports` still passes; the example program that uses the provider compiles to a module with zero imports, asserted by inspecting its import section.                                                                                                                                                                                                                                                                          |
| Answers "who grants"                                   | This document; tests for both keys and both revocations (admin revoke and user revoke each stop the next read and abort in-flight ones).                                                                                                                                                                                                                                                                                                                                 |

Also: replay of a journal containing a client result restores the same model
and view with no client connected; reopen with a pending request journals the
`actor_denied` (`restarted`) refusal; a forged and a duplicate nonce are
dropped; host and client rate limits each refuse independently; a v1 shell
never receives a provider frame.

## Dependencies and what unblocks implementation

- **F12.4 provider registry (blocking).** Needs: the manifest fields named
  above plus `side` (or an extension field that holds it); a per-manifest
  default ceiling with an `excluded` value; the ledger, `validate_grant_names`,
  `validate_ceiling_names`, and `CapsReport` reading the registry instead of
  `CATALOG`; the actor dispatching by registry entry instead of the `match` at
  `actor.rs:538`; argument-schema and bounds validation the host can run
  before routing.
- **F12.6 audit (blocking).** Needs: a provider-use event kind with the fields
  above; an `emitter` field and a correlation key; an append-only sink usable
  from the client binary, not only the host; the replay-to-final-state test
  pattern.
- **F11.12 journal format (blocking for frames only).** The plan's Risks rule
  says new frame kinds wait for F11.12. No new journal entry kind is needed.
- **F11.13 error taxonomy.** The two new codes are added through it, with the
  guide updated.
- **F13.1 shell contract.** v2 adds the three frames; v1 is untouched.
- **Session attribution.** `WorldCommand::Input` carries a token, not a session
  (`actor.rs:1029-1033`). Routing to the originating session needs a session
  id carried with the input. This is small and can land with the
  implementation.

Implementation can start when F12.4 and F12.6 have merged and their exit tests
pass. The pure parts (manifest, two-key decision, bounds intersection, path
normalisation) can be built and tested first; the frames follow F11.12.

## Open questions

1. **An `ask` mode.** Should the client key support a per-request prompt
   besides on and off? It is the strongest defence against a malicious host,
   but a prompt in a raw-mode terminal shell fights the world's own rendering.
   Recommend: on and off in slice 1; `ask` in the native and web shells first.
2. **`rate_limited` as its own code.** Reusing `outside_ceiling` with
   `reason: rate` follows the "reuse before adding" rule in
   `world_caps.rs:353-357`, but rate refusals are transient and a program
   would retry them, unlike a ceiling. Recommend: decide in F11.13 for host
   and client providers together (F12.5 `http.fetch` and `notify` need it too).
3. **Several sessions for one participant.** Slice 1 routes only to the
   originating session. A participant with a terminal and a native shell open
   might want the native one to serve. Recommend: originating session only
   until F13.8 defines multi-session identity.
4. **Content in the journal.** Should a world be able to declare that it keeps
   only the digest and discards content after `update`? That breaks replay as
   defined today; it is worth revisiting only with F12.13.
5. **Server ceiling.** Should `--world-allowed-caps` default to excluding
   every `side: client` name, so a server operator must also opt in? Recommend
   yes, matching `default_ceiling: excluded`.

## References

- [Phase 13 plan](../../.opencode/plans/phase-13-client-shells.md) (F13.7,
  open question 3)
- [Phase 12 plan](../../.opencode/plans/phase-12-platform-flexibility.md)
  (F12.4, F12.5, F12.6, F12.14)
- [Phase 11 plan](../../.opencode/plans/phase-11-foundations.md) (F11.11,
  F11.12, F11.13)
- [World capability ceilings](../2026-10-16T11-00-00Z_feature_world_capability_ceilings/2026-10-16T11-00-00Z_feature_world_capability_ceilings.md)
- [Attenuated capabilities](../2026-10-09T09-59-07Z_design_attenuated_capabilities/2026-10-09T09-59-07Z_design_attenuated_capabilities.md)
- Issues: #789 (this task), #227 (forward-only journal compatibility), #564
  (Phase 13 plan), #534 (Phase 12 tracking)
- Code: `pkgs/id/src/world_caps.rs`, `pkgs/id/src/world/actor.rs`,
  `pkgs/id/src/world/core.rs`, `pkgs/id/src/world/mod.rs`,
  `pkgs/id/src/world_session/types.rs`, `pkgs/id/src/sandbox.rs`
- The v1 shell contract: `pkgs/id/contract/shell-contract.v1.json`
