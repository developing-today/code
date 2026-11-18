# Design: boundary enforcement inputs — declaration residence, evidence sources, verifier (#8851)

- **Date:** 2026-11-17
- **Status:** design decision record; the loading plumbing it specifies is
  tracked separately (see §7). Nothing in the live signup/login, world or
  bridge/federation call sites is wired by this record — that is #7787
  parts 2-4 (queue task 15074).
- **Issue:** [#8851](https://github.com/developing-today/code/issues/8851),
  "id: boundary enforcement inputs — declaration residence, evidence sources,
  verifier (#7787 parts 2-4 prerequisite)".
- **Parent:** [#5683](https://github.com/developing-today/code/issues/5683).
  Consumes the framework design `doc/2026-11-08_design_id5683.md` (§2.2, §2.3,
  §3, §7, §8) and the step-6 adapter `crates/id-core/src/boundary_surfaces.rs`
  (PR #8850, merged `4267e5213b`).
- **Basis:** queue task 20223 (lane code-i, 2026-11-17) on `origin/main`
  `b4b6a1b16b`, worktree `~/git/fleet/queue-20223-8851-boundary`. Every code
  location below was read at that baseline.

## Problem this record answers

`boundary_surfaces::check_surface` (`pkgs/id/crates/id-core/src/boundary_surfaces.rs:331`)
takes ten inputs: an `EnforcementPoint`, an `&EffectiveBoundary`, the boundary
version, a `&ScopeRef`, an `&ActorId`, the subject, a residency `&Scope`,
`&[SubjectEvidence]` claims, `now`, and a `&dyn EvidenceVerifier`. Nothing
outside `id-core` can produce nine of those ten. Verified at the basis commit:

```
git grep -n 'BoundaryDeclaration\|EffectiveBoundary\|check_surface' -- pkgs/id
  -> only crates/id-core/src/{boundary,boundary_surfaces,intranet,residency,
     residency_inventory}.rs
```

Wiring the surfaces without deciding these inputs would invent the semantics
inline in four live call sites. This record decides them, and names where each
piece lives.

## 1. Declaration residence: the operator config, per scope, content-addressed

**Decision.** Declarations live in the **operator's existing server config
file**, in a new `[[boundaries]]` array, and are loaded by the same loader that
already owns operator policy: `ServerConfig::load`
(`pkgs/id/crates/id-cli/src/server_config/mod.rs:178`).

Rationale, from what is already there:

- The config is already the operator's single reviewable artifact for
  who-may-do-what: `users`, `groups`, `create`, `policy`, `budgets`
  (`mod.rs:72-84`). Boundary declarations are the same kind of statement —
  operator policy about their own server, world or accounts — so a second
  residence would give the operator two files to keep consistent.
- The config is already persisted and replay-safe: `start_server_config_persisted`
  (`pkgs/id/crates/id-cli/src/commands/serve/startup.rs:69`) provisions from it
  into the journaled `directory.jsonl` beside the groups log, the same layout a
  world directory uses. Declarations need no new durability story.
- A declaration is **content-addressed and immutable** (`BoundaryDeclaration::id`
  is BLAKE3 over the canonical body, `boundary.rs:216-236`). Residence therefore
  stores *which* id is effective per scope, not a mutable blob: replacing a
  boundary means naming a different id, and `ensure_supersedes`
  (`boundary.rs:342`) checks monotone version per scope.

**Scope resolution.** Each entry names the scope it governs, using the existing
`ScopeRef` (`Server(name) | World(name) | Account(name)`, `boundary.rs:71-79`).
The config keys are the operator's own names for those scopes, so no new
namespace is introduced:

| Scope | Config identity | Effective at |
| --- | --- | --- |
| server | the config's own server name | every surface point the declaration lists |
| world | the world id the config provisions / serves | that world's membership and blob points |
| account | the account key (`UserSpec::key`, `mod.rs:89-91`) | that account's signup/login points |

The **effective boundary for one check** is `boundary::effective(server, world,
account)`
(`boundary.rs:468`) over whichever of the three scopes are in play at that call
site — intersection only, as §7 of the framework design requires, so a narrower
world or account declaration can never be widened by a broader server one. A
call site with no world in play passes `None` for it rather than inventing an
empty declaration.

**What the config does not become.** It does not carry evidence or verifier
keys beyond the trust roots it already needs:
`Policy.trust_roots` (`residency.rs:90`) is the operator's issuer list, and it
is the same set `is_trusted_peer`/`peer_asserted_claim`
(`boundary_surfaces.rs:254,267`) consume. One list, one meaning: an issuer the
operator trusts for placement evidence is a peer they trust for peer-asserted
claims. Splitting those two would let a boundary be satisfied by an issuer the
peer-trust matrix would already refuse.

## 2. Evidence sources: one issuer per hop, all signed, all operator-trusted

**Decision.** Evidence is **self-reported by the component that performs the
hop**, signed by that component's key, and accepted only when its issuer is in
the operator's `trust_roots`. There is no ambient, unsigned evidence path. This
follows the #9141 acceptance rule already landed as a conformance oracle
(`doc/2026-11-17T05-15-00Z_design_id9141_peer_trust_classes.md` §1; matrix
`pkgs/id/crates/id-sync/src/federation/slices/b31_peer_classes.rs`).

The four sources the issue names map onto components that already exist:

| Evidence covers | Issuer (signs) | Where the hop happens | Plane |
| --- | --- | --- | --- |
| (a) account store | the serving process that wrote the account record | `Directory` writes: `sign_up` / `sign_up_keyed` / `authenticate` (`pkgs/id/crates/id-directory/src/directory.rs:613,655`) | storage |
| (b) serving connection | the listener that accepted the connection | serve startup / connection accept (`pkgs/id/crates/id-cli/src/commands/serve.rs`) | transport |
| (c) world blob stores | the world store that persisted the blob | `world_store.rs` (`open`/`load`, `:202,662`) | storage |
| (c) world turn hosts | the world session host that ran the turn | `world_session/service.rs` | processing |
| (d) federation peers/relays | the peer or relay itself, about its own endpoint | federation ingress/egress (`id-sync` envelope, `envelope.rs:125`) | transport |

**Peer-asserted claims are graded, never trusted by default.** A peer's claim
about a subject's location enters the same `&[SubjectEvidence]` list but is
constructed through `peer_asserted_claim` (`boundary_surfaces.rs:267`), which
caps it at `SelfDeclared` unless the peer is in `trust_roots`. Collection is
therefore mechanical and ordered:

1. local hops the process performed itself (a, b, c) — `ProviderVerified` when
   the local issuer is a trust root;
2. peer-asserted claims for hops the process did not perform (d) — capped by
   `peer_asserted_claim`.

A missing trust configuration leaves `trust_roots` empty, which under the
adapter's rules means **no declaration activates** and no untrusted claim
satisfies a bar — the safe direction, and already tested
(`boundary_surfaces.rs:778` `peer_claim_is_self_grade_until_the_peer_is_a_trust_root`).

**Why not a separate evidence service.** A fifth process that attests for others
would have to be trusted *and* able to observe the hops, i.e. it would become
every component's proxy. Self-signed per-hop evidence keeps the attester and the
actor identical, which is the property the design's "a signature proves
authorship, not physical location" (`residency.rs:15`) and the assurance model
(§3.2) both assume.

## 3. Verifier: one app-side `EvidenceVerifier`, backed by the existing signed-artifact keys

**Decision.** The app implements `EvidenceVerifier`
(`residency.rs:159-162`: `signature_valid`, `revoked`) **once**, in the `id`
package, and every surface receives `&dyn EvidenceVerifier` from that one
implementation.

- `signature_valid` verifies the assertion with the issuer's key using the same
  ed25519 path the sync layer already uses —
  `ed25519_dalek::VerifyingKey::verify_strict`
  (`pkgs/id/crates/id-sync/src/artifact.rs:531`, `envelope.rs:134`). No second
  crypto path is introduced.
- `revoked` consults the revocation state the key lifecycle already maintains:
  slice B31's per-peer `PeerPolicy::policy_version`
  (`b31_peer_classes.rs`) for peer issuers, and the key-revocation record for
  local issuers. Revocation is checked per call, not cached across a session,
  because §8 requires a recheck when evidence is revoked mid-session.
- The verifier **never** decides policy. It answers only "did this issuer sign
  this assertion" and "is it revoked". Assurance grading, trust-root
  membership and the failure behaviour stay in the adapter, so a verifier bug
  cannot widen a boundary on its own — it can only fail closed.

## 4. What a surface call site receives

From the above, one loader produces exactly what `check_surface` needs:

```rust
// shape only; the concrete type lands with the plumbing
pub struct BoundaryInputs {
    pub effective: EffectiveBoundary, // boundary::effective(server, world?, account?)
    pub version: u64,                 // monotone, from the winning declaration
    pub scope_ref: ScopeRef,          // the scope the check is charged to
    pub policy: Policy,               // residency Policy incl. trust_roots
    pub verifier: Arc<dyn EvidenceVerifier>,
}
```

and one claims-builder produces `Vec<SubjectEvidence>` per check from local hops
plus `peer_asserted_claim`. A call site with no enabled declaration for its scope
gets an `EffectiveBoundary` whose capability is `None`, and `check_surface`
already returns `Allow` for that shape (`boundary_surfaces.rs:344-348`) — so
"not configured" stays a no-op, and no surface needs a branch of its own.

## 5. Versioning, supersession and failure

- Declarations are immutable; the config names ids. Editing a declaration's
  body yields a new id, which is what `ensure_supersedes` compares
  (`boundary.rs:342`).
- `version` is monotone per scope starting at 1 and is the value passed to
  `check_surface`, so the decision log records the exact boundary that decided.
- Failure behaviour is the declaration's own (`FailureBehaviour`: deny, warn,
  step-up, quarantine — `boundary.rs:124`); the loader does not override it.
- A declaration that is invalid (enabled with no enforcement point, or a
  non-superseding version) is a **load error at startup**, never a silently
  ignored entry: `BoundaryDeclaration::new` already rejects the first case with
  `DeclarationError::EnabledWithoutEnforcement` (`boundary.rs:238-256`).

## 6. Rejected alternatives

- **Declarations in the world record / blob store** (a world carries its own
  boundary): rejected as the *primary* residence because a boundary the world
  itself supplies cannot bind a hostile world. World-scoped declarations are
  expressed as `ScopeRef::World` entries in the operator config; the operator
  remains the author. A world may *propose* a boundary in future work, but
  accepting it is an operator action, not a load-time one.
- **A new `boundaries.toml` beside the config**: rejected — a second operator
  artifact to keep consistent with `ServerConfig`, with no added expressiveness.
- **Unsigned local evidence for loopback hops**: rejected by the #9141 rule
  (no unsigned local shortcut); loopback is a location, not an identity.
- **Verifier inside id-core**: rejected — the verifier needs key material and
  revocation state that belong to the app/sync layer; keeping it app-side keeps
  `id-core` free of key handling, as its module docs intend (`boundary.rs:6-7`).

## 7. What lands next (plumbing, and what stays out)

In scope for the `#8851` plumbing task, in this order:

1. `[[boundaries]]` parsing in `ServerConfig` (config schema + validation via
   `BoundaryDeclaration::new`/`ensure_supersedes`).
2. The loader: scope → `EffectiveBoundary` + version + `ScopeRef` + `Policy`.
3. The single app-side `EvidenceVerifier` (§3).
4. The claims-builder (§2), including `peer_asserted_claim` for peer hops.
5. A test that loads a declaration and constructs an effective boundary per
   scope, plus one that an absent declaration yields the no-op `Allow` shape.

Explicitly **out of scope** (they are #7787 parts 2-4, queue task 15074): calling
`check_surface` from `Directory::sign_up`/`sign_up_keyed`/`authenticate`, from
world membership/blob paths, or from the federation ingress/egress points; and
any rendering of the outcome in the UI. This record exists so those call sites
receive inputs rather than invent them.

## 8. Open points

- **Key custody for per-hop issuers.** Each issuer signs with a key; where those
  keys live (process-local generated, or operator-provisioned per component) is
  a key-lifecycle question that belongs with #10186/B31 follow-ups, not this
  record. Until it is settled, the plumbing can accept an issuer identity
  string per component and leave signing to the caller — the design does not
  depend on which.
- **Relay-attested hops.** A relay attesting for a path it carried is
  `Self`-grade by the same rule as a peer; whether a relay may ever be a trust
  root is a policy question for the operator, and this record does not grant it.
