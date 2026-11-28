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

## 2026-10-10T02-56-13Z Audit: capability checklist

Checked each claim in this doc against the code. Line numbers are in
`pkgs/id/src` (`world.rs`, `world_caps.rs`, `world_compile.rs`,
`world_store.rs`). Status: DONE means implemented and tested; PARTIAL means
implemented with a gap or deviation noted; MISSING means not implemented.

Verification plan:

| Item | Status | Evidence |
| --- | --- | --- |
| Ledger: ceiling removal takes effect on existing capabilities without reissue | DONE | `decide_for` consults the ledger on every use (`world_caps.rs:349`); `set_granted` (`world_caps.rs:379`); test `revoking_a_grant_applies_to_capabilities_already_issued` (`world.rs:3176`), added in this audit |
| Attenuation: child cannot widen its parent | DONE | `WorldScopes::attenuate` requires a subset (`world.rs:116`); `attenuate` (`world.rs:2370`); test `attenuation_only_narrows_and_needs_delegate` (`world.rs:3447`) |
| Attenuation: child cannot outlive its parent | DONE | `delegate` rejects a later expiry and uses above the parent's remaining uses (`world.rs:1005`, `ensure_not_expired` `world.rs:1052`); tests `delegation_may_not_outlive_or_expire_in_the_past` (`world.rs:3235`), `restore_refuses_a_delegate_outliving_its_parent` (`world.rs:3356`) |
| Attenuation: child cannot delegate when `may_delegate` is false | DONE | Delegation requires `DELEGATE` (`world.rs:117`); GUEST excludes it (`world.rs:75`); test `attenuation_only_narrows_and_needs_delegate` (`world.rs:3447`, JOIN|DELEGATE case) |
| Revocation: revoking a parent refuses the subtree | DONE | `revoke_tree` (`world.rs:1136`); tests `revoked_capabilities_cannot_read_or_write` (`world.rs:3437`), `delegated_capabilities_are_narrow_and_die_with_their_parent` (`world.rs:3481`) |
| Revocation: journal replay reproduces the same state | DONE | `Revoked` replay in `WorldCore::restore` (`world.rs:674`); test `a_restored_delegation_keeps_its_parent_link` (`world.rs:3560`); `a_revoked_delegation_subtree_stays_revoked_after_restart` (`world_store.rs:504`), added in this audit |
| Participant attribution: `chat.say` by a `JOIN\|CHAT` participant is allowed | DONE | Added in this audit: `a_participant_with_chat_lets_the_program_say_what_it_asks` (`world.rs:2830`) asserts an allowed use and zero denials. Gate: `execute` uses `actor_permits` from `acting` (`world.rs:1743`) |
| Participant attribution: `chat.say` by a participant without CHAT is refused | PARTIAL | `a_participant_cannot_make_the_program_do_what_its_scopes_forbid` (`world.rs:2798`) uses a JOIN\|INPUT participant. The refusal code is `actor_denied`, not `cap_denied` (`world_caps.rs:290`, `:357`); see deviations. A JOIN-only participant cannot submit input at all, so that exact case is not reachable through the input path |
| Import gate: a program importing an ungranted module fails to compile | DONE | `check_imports` (`world_compile.rs:101`), run before the compiler (`world_compile.rs:182`); tests `an_ungranted_platform_import_is_refused_by_name` (`world_compile.rs:643`), `a_sibling_file_cannot_smuggle_an_ungranted_import` (`world_compile.rs:667`), `an_ungranted_import_is_refused_before_the_compiler_runs` (`world_compile.rs:677`) |
| Import gate: a program importing only granted modules compiles and runs | PARTIAL | Compile and install are tested (`apps_can_import_the_platform_screen_module`, `world_compile.rs:528`; end-to-end check in the 11-28-37Z section). No test asserts that such a program then runs and answers requests |
| Journal: no secret in any entry after mint, attenuate and revoke | DONE | `attenuated_capability_secrets_never_reach_the_journal` (`world_store.rs:556`), added in this audit, scans the journal after all three operations; `journal_never_contains_capability_secrets` (`world_store.rs:478`) covers the manual `Issued` path |
| Existing tests keep passing (join, invite, capability tests in `world.rs` and `sandbox.rs`) | DONE | `cargo test --lib`: 641 passed, 0 failed (640 before this audit, plus the one added here) |

Claims beyond the verification plan:

| Item | Status | Evidence |
| --- | --- | --- |
| Ticks run with no acting participant (ceiling only, no attribution) | DONE | `acting` cleared after each command (`world.rs:1520`); `fire_ticks` (`world.rs:1721`); tests `subscribed_ticks_arrive_only_while_someone_is_present` (`world.rs:3054`), `ticks_follow_the_world_grant_not_a_participant_capability` (`world.rs:3077`), `ticks_speak_for_the_world_not_for_a_present_participant` (`world.rs:3137`), added in this audit |
| Use limits draw down every ancestor | DONE | `spend` (`world.rs:1074`), `ensure_uses_left` (`world.rs:1065`); test `use_limits_draw_down_every_ancestor` (`world.rs:3279`). Counted on committed chat and input only (`commit`, `world.rs:1289`) |
| Account-held scopes have a directory ceiling | DONE | `issue_for_account` (`world.rs:910`); `effective_scopes` (`world.rs` near 1015) |
| Ceiling for program-level (non-account) capabilities | PARTIAL | Program effects are gated by `CapLedger` only (`world_caps.rs:349`). A non-account capability has no scope ceiling. Not implemented: a policy decision, not a bug |
| Direct participant `chat` and `input` gated by the ledger | PARTIAL | `chat` (`world.rs:1170`) and `prepare_input` (`world.rs:1189`) check scope only, not the ledger. Gating them would silence chat under the default `deny` policy. Not implemented: a policy decision |
| Grant-on-use restricted to development | PARTIAL | `--world-cap-policy` defaults to `deny` (`cli.rs` near 393); `grant-on-use` is accepted by `serve.rs` (near 351) with no development-only restriction. Not implemented |
| Wasm host-import gating | MISSING, vacuous | The sandbox rejects every module import (`sandbox.rs` near 77-84; test `rejects_modules_with_host_imports`, `sandbox.rs:1157`), so there is nothing to gate. The native worker has no host function table. Not implemented |

Open questions answered by the implementation:

- Ticks: ceiling only, no per-participant attribution (`fire_ticks`, `world.rs:1721`).
- CLI default: `deny` (`cli.rs` near 393).
- Delegation default: GUEST lacks DELEGATE, so delegation is off unless granted.
- Import gate: per module, checked at compile time (`check_imports`).

Deviations from the doc:

- A refused program effect from a participant returns `actor_denied`, not `cap_denied` (`world_caps.rs:290`). The doc asked for `cap_denied`. Changing the wire code is left for a decision.
- A JOIN-only participant cannot submit input, so the JOIN-only refusal case is covered by a JOIN|INPUT participant instead.

Verification run for this audit: `cargo fmt --check` clean; `cargo test --lib` 641 passed; `cargo clippy --all-targets` (default features) exits 0 with the same 143 pre-existing warnings and none on lines added in this audit. The `--all-features` (web) build was not run here. Its `web/dist` assets were not built, so `rust-embed` fails.
