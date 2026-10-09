# Attenuated capabilities for world access

Related: [`2026-10-07T00-20-00Z_architecture_roc_worlds`](../2026-10-07T00-20-00Z_architecture_roc_worlds/2026-10-07T00-20-00Z_architecture_roc_worlds.md)
(the worlds architecture this design extends, especially "Authorization" under
the capabilities section and the join and invite frames).

## Intent

A server must control access by which platform functions a world program may
use, and must accept untrusted programs only as source it compiles itself from
trusted sources. Today the server has two access mechanisms:

- a per-world grant ledger of named capabilities (`world_caps.rs`:
  `time.now`, `players`, `chat.say`, ...), and
- bearer join capabilities that carry a fixed `WorldScopes` bit set
  (`JOIN`, `CHAT`, `INPUT`).

The grant ledger is an access control list (ACL): authority is a table keyed
by world, checked on every use, and it can only be changed by an admin. The
join capability is an object capability (ocap) in the narrow sense that
possession is authority, but it cannot be narrowed or delegated.

This document records the decision on the direction, the design for v1, and
the long-term path to decentralised capabilities (OCapN, Keyhive), so that
v1 choices do not close that path.

## Decision

1. **Authority is carried by capabilities.** A capability is a bearer record
   that names a world and a set of functions it may cause. Possession of a
   valid capability, and nothing else, authorizes those functions.
2. **Attenuation only narrows.** A holder may mint a child capability with a
   subset of its own functions, an earlier or equal expiry, and an optional
   use limit. Nobody can widen a capability, and the server never mints a
   capability that exceeds its root.
3. **The ACL survives as the world's ceiling.** The world's grant ledger is
   the set of functions the world permits at all. The effective authority of
   any capability is `capability.functions ∩ world.ceiling`, checked at use.
   Removing a function from the ceiling takes effect across every capability
   at once, without reissuing them. The ceiling is administrative policy, not
   a grant.
4. **Identity never grants authority.** Iroh node IDs, WebSocket origins, SSH
   user names and display names do not appear in any authorization decision.
   They may be logged, and they may select a world (SSH user, `world` field),
   but the capability decides. This is already the rule for Iroh.
5. **The program's surface follows the ceiling.** Two gates:
   - *Import gate (compile time).* Source the server compiles may import only
     platform modules and functions in the world's ceiling. A program that
     imports `Screen` in a world that has not allowed it fails to compile, so
     it never runs.
   - *Effect gate (run time).* Each host effect request is executed with the
     authority of the capability that caused it (see "Who a request acts
     for"), intersected with the ceiling. A denial is journaled and visible to
     the program, as `cap_denied` is today.

Trusted sources are: native modules admin-installed (opt-in), and Roc source
the server compiles. Compilation stays admin-only until a separate decision
says otherwise.

## Vocabulary

- **Function.** A stable, versioned name in the catalog. It is either an
  *effect* (`time.now`, `time.tick`, `random.u64`, `players`, `players.list`,
  `chat.say`, `world.info`) or a *platform module* (`Screen`, `Key`, and later
  others). Effects are gated at run time. Modules are gated at compile time.
  Names never change meaning; new names are added.
- **Ceiling.** The set of functions a world permits. Stored as the existing
  grant ledger, extended to cover modules and the join scopes. The `*` grant
  means every function the server knows.
- **Capability.** A bearer record: `{id, world, functions, expires_at?,
  uses_left?, parent?, may_delegate, revoked}` plus a secret. The secret is a
  256-bit OS-RNG value, shown once, and stored only as its SHA-256 digest
  (already the rule for join capabilities). The journal records mint,
  attenuate and revoke by `id` and digest, never the secret.
- **Root.** The admin token mints root capabilities for a world. The admin
  token itself is not a world capability and is never journaled.

## Who a request acts for

This is the main behavioural change, so it is stated explicitly.

- A request a **participant** causes (for example a `chat.say` produced by
  that participant's input) is authorized by **that participant's capability**,
  intersected with the ceiling. A guest with `JOIN|CHAT` cannot cause
  `players` or any other function it was not handed.
- A request the **world program** makes on its own (ticks, the `wants` of a
  model that is not tied to one participant) is authorized by the **world's
  ceiling** only. The program has no authority beyond the ceiling, and no
  authority derived from who happens to be connected.

Today the grant ledger applies to every request regardless of who caused it.
Moving participant-caused effects to the participant's capability is the
confused-deputy fix: a program holding `chat.say` must not speak for a
participant who was never given it.

## v1 design (implement next; nothing below is built yet)

### Minting and attenuation

- `mint(world, functions, expires?, uses?, may_delegate)` by the root: the
  functions must be within the ceiling, or the call is refused.
- `attenuate(parent_secret, functions, expires?, uses?)`: the parent must
  have `may_delegate`; the child's functions must be a subset of the parent's
  functions; `expires` must not exceed the parent's; the child inherits
  `may_delegate = false` unless the parent grants it explicitly, and it may
  not exceed the parent's delegation.
- Guests obtain capabilities through `invite` and `join` as today. Invites
  default to `may_delegate = false`.

### Checking

At each use: look up the record by the digest of the presented secret; refuse
if unknown, revoked, expired, out of uses, or for another world. The effective
functions are `record.functions ∩ ceiling(world)`. Ancestor checks are not
done at use: revoking a capability marks its whole subtree revoked at
revocation time, which the journal records as one event.

### Mapping from today

- `WorldScopes::JOIN|CHAT|INPUT` become the functions `join`, `chat.say` and
  `input`. Existing journal entries replay with their bits mapped to names, so
  no journal migration is needed beyond the mapping.
- `world_caps.rs` keeps the catalog, the bounds, the usage report and the
  `grant_caps` frame. Its ledger becomes the ceiling. `--world-cap-policy`
  keeps `deny` and `grant-on-use`; `grant-on-use` is documented as a
  development policy, and the default for a world that accepts untrusted
  source should be `deny` (see open questions).
- Transports do not change. Iroh, WebSocket and SSH all present a bearer
  capability as their first credential, and all reach the same check.

### Compile-time surface

- The compile service receives the world's ceiling and rejects any `import`
  outside it, with the import named in the error.
- Wasm: only the host imports allowed by the ceiling are linked. A missing
  import fails at instantiation, which is a second, independent gate.
- Native tier: the worker is given only the host functions in the ceiling.
  The worker is already sandboxed (Landlock on Linux); this adds the function
  allowlist on top, it does not replace the sandbox.

## Long-term path to decentralised capabilities

The v1 record is designed so later stages change how a capability is
verified, not what it says or how it is presented.

Constraints that v1 must keep:

- **C1. No identity-keyed authority.** New code must not key an authorization
  decision on a node ID, user name or origin.
- **C2. Canonical, self-describing records.** A capability's content
  (`world`, `functions`, `expires_at`, `uses`, `parent`, `may_delegate`,
  `issuer`) has one canonical encoding. Stage 1 signs that encoding with an
  issuer key; the holder can then verify the chain offline.
- **C3. Monotone attenuation checked in one place.** The subset and expiry
  rules live in one function that both `mint` and `attenuate` call, so a
  signed chain can be verified by the same rule.
- **C4. Revocation by id, cascading.** Revocation is a journal event naming an
  id. A replicated world distributes it as an event rather than as a server
  call.
- **C5. The surface is derived, not hard-coded.** The import list and host
  imports come from the ceiling. A peer that runs the same source under a
  different ceiling gets a different surface, and nothing in the program
  changes.
- **C6. Stable function names.** Already a rule; it is what lets signed
  capabilities from one server mean the same thing on another.

Stages:

- **Stage 0 (v1, now).** Server-held bearer capabilities, digest-stored,
  attenuable inside one server, with the ceiling as policy. Bearer means a
  leaked capability is usable until it expires, is used up or is revoked.
- **Stage 1 (trigger: a capability must be checkable without the issuing
  server being online, or be handed to a party that should not reach the
  server).** Signed capabilities and delegation chains, with issuer keys
  derived from the existing node keys. Still one world, one host. Revocation
  becomes a signed journal event. This is a new verification step under the
  same records.
- **Stage 2 (trigger: a world must replicate between peers without a trusted
  host, or must be encrypted from its host).** Evaluate Keyhive: convergent
  capabilities, revocation without a central server, BeeKEM group key
  agreement, Beelay sync. It is pre-alpha, so adopt it behind the Stage 1
  verifier only after it reaches a stable release, and keep the record format
  independent of it.
- **OCapN (trigger: programs in different servers must hold references to
  each other's objects).** Adds network-transparent references, handoffs and
  distributed GC. It is pre-standard. It is not needed for worlds that are
  hosted on one server and joined by capability.

Keyhive and OCapN are not implemented, and no code depends on either. They are
recorded here so that the decision to defer them is revisited on a trigger,
not on a date.

## Threats and what this does not protect

- **Bearer theft.** Anyone holding a capability has its functions until it
  expires, is used up or is revoked. Mitigations: narrow functions, short
  expiry, use limits, and transports that encrypt the credential (Iroh, TLS
  WebSocket, SSH). The SSH password is a capability and is subject to the same
  rules.
- **Confused deputy.** Addressed by "Who a request acts for". The residual
  risk is a world-level request that the program makes on its own; it is
  bounded by the ceiling.
- **Trusted source only.** The import gate applies to source the server
  compiles. Native modules are admin-installed, so they are trusted by
  decision, not by the gate.
- **Resource limits** are separate from authority. Bounds in `world_caps.rs`
  and the admin-set execution limits continue to apply to every capability.
- **Not a sandbox.** This is authorization. Isolation is still the Wasm
  runtime and the native worker's sandbox.

## Open questions

1. **Ticks and other world-level effects.** Confirm they use the ceiling only.
   Recommended: yes.
2. **`grant-on-use` in production.** Recommended: `deny` as the default for
   any world that accepts source, with `grant-on-use` allowed only in
   development.
3. **Delegation default.** Recommended: guests cannot mint sub-invites. Admins
   can enable `may_delegate` per capability.
4. **Module gating granularity.** Whether `Screen` is one function or each
   export is its own. Recommended: per module first; per export only if a
   program needs to be given part of a module.

## Verification plan (for the implementation slice)

- Ledger: ceiling removal takes effect on existing capabilities without
  reissue.
- Attenuation: a child cannot widen its parent, cannot outlive it, and cannot
  delegate when `may_delegate` is false. Refusals are tested for each rule.
- Revocation: revoking a parent refuses the subtree; replay of the journal
  reproduces the same state.
- Participant attribution: a `chat.say` caused by a `JOIN|CHAT` participant is
  allowed, and one caused by a `JOIN`-only participant is refused, with the
  program receiving `cap_denied`.
- Import gate: a program importing an ungranted module fails to compile, and a
  program importing only granted modules compiles and runs.
- Journal: no secret appears in any journal entry (test scans the journal for
  the secret bytes after mint, attenuate and revoke).
- Existing tests keep passing, including the join, invite and capability
  tests in `world.rs` and `sandbox.rs`.

## Next steps

1. Review this design. Confirm the four open questions, especially the
   "Who a request acts for" change, which alters current behaviour.
2. Implement the v1 in small commits: the record and attenuation rules
   (pure, in `world_caps.rs`), then mapping the join scopes, then the
   participant-attributed check, then the import gate in the compile service.
3. Record each step as a dated section in this document.

No ratatui dependency is added; the Rust side has no widget consumer yet.

---

## 2026-10-09T10-32-35Z Update: v1 rules implemented (scopes, attenuation, attribution)

Implemented in `world.rs` and `world_caps.rs` (first two steps of "Next steps"):

- **Delegate scope.** `DELEGATE` is a scope bit (`delegate` on the wire), not a
  separate flag. A capability with it may mint a child whose scopes are a
  subset of its own and that keeps `JOIN`. This is the `may_delegate` of open
  question 3, expressed as a scope. `GUEST` does not include it, so guests
  cannot mint sub-invites by default.
- **Attenuation over the session.** `attenuate {name, scopes}` is joined-only.
  Success answers with an `invite` frame; a refused delegation answers
  `delegation denied`; an unknown scope name answers `unknown scope`.
- **Parent links are journaled.** `Issued` carries an optional `parent`,
  defaulting to none so existing journals still replay. Revoking a capability
  revokes its subtree (`revoke_tree`), and replaying `Revoked` runs the same
  cascade, so restore reproduces the state.
- **Attribution.** A program request caused by a participant's input is checked
  against that participant's scopes: `chat.say` needs `CHAT`, other functions
  need `JOIN`. A refused actor is recorded as denied and never triggers a grant.
  Requests the host causes (ticks, host events) use the ceiling only, which
  matches open question 1.

Deviations from the verification plan:

- The error code is `actor_denied`, not `cap_denied`. It is decided in
  `CapLedger::decide_for`, which is also where the ceiling check lives.
- A `JOIN`-only participant cannot submit input at all, because input requires
  `INPUT`. The attribution case that matters is therefore an `INPUT` participant
  without `CHAT`, not a `JOIN`-only one.

Not yet done: the import gate in the compile service, expiry and use limits,
and attributed ticks. Tests cover the pure attenuation rule, scope names and
effects, the ledger refusal, cascading revocation, the participant limit,
restore with a parent link, the actor-level refusal, and the session attenuate
frame.

---

## 2026-10-09T11-28-37Z Implementation: import gate and compile memory

Implemented the compile-service half of the gate in `world_compile.rs`,
`world_caps.rs` and `world_session.rs`:

- **Module list from the platform.** `Compiler::new` lists the platform's
  modules (every `.roc` file but `platform`). The catalog gains `Key` and
  `Screen`, matching those modules.
- **Refused before the compiler runs.** `check_imports` reads each `import`
  line of the submitted sources and refuses a platform module the world does
  not grant, naming it and the grant command. It runs on the granted set
  captured at compile time, so granting or revoking takes effect on the next
  compile. The check is by import line, not by the compiler's output, so a
  sibling file cannot pull in an ungranted module.
- **Compile-time, not run-time.** Platform modules are gated when source that
  imports them is compiled. A running program is never checked again.

Tests cover refusal by name, comments and non-platform imports passing,
sibling-file smuggling, and refusal before the compiler is spawned. The
`roc-screen` test grants `Screen` and `Key`, because that example imports both.

Found while verifying end to end: `id world compile` built wasm32 apps with
roc's default memory (1024 pages, 64 MiB). The sandbox's 16 MiB limit traps on
instantiate, so every server-side compile failed at install with
`InvalidModule`. The example build scripts already pass `--wasm-memory`; the
compile path now passes the same 129-page value (`WASM_MEMORY_BYTES`).
Native (x64musl) builds are unaffected.

Verified against a scratch `id serve --world` on the system roc: compiling
`roc-screen` with nothing granted is refused for `Screen`; with `Screen` only
it is refused for `Key`; with both it installs. Revoking `Screen` refuses the
compile again, and re-granting restores it.

Not done here: expiry and use limits on capabilities, attributed ticks, and
the "who a request acts for" check for ticks.
