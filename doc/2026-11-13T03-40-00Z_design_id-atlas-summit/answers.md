# Summit decisions, defaults and owner questions

These are additive decisions for #9106, not changes to the intake's binding
signature, extensibility or sibling-ownership constraints. Component workers are
incorporating them; final review must verify the actual documents agree.

## Technical decisions made in the mill

| Question | Decision / alternative retained | Consequence if changed |
|---|---|---|
| Initial content hash | Preserve Iroh raw exact-byte BLAKE3-256; self-describing profile dispatch, no per-object translation table. Legacy artifact SHA-256 and event/world commitments remain separate. | New algorithms add profiles and compatibility vectors; never reinterpret old IDs. |
| Sidecar versioning | Immutable signed snapshots plus a signed generation/predecessor CAS head. Snapshot digest, not date alone, distinguishes filenames; timestamps remain descriptive. Single complete mutable file and enumerate-only versions remain documented alternatives. | Changing physical layout is an adapter migration; content identity and complete authorized metadata semantics stay stable. |
| Signature preimage | Versioned deterministic body, detached signature, then envelope hash. Body excludes its own signature/ID. Existing JSON artifact signing remains legacy bytes. | Any new preimage requires a new profile and dual-reader migration, never a silent compatibility break. |
| Namespace URI | Internal, unregistered `id://<resolver-hint>/<namespace>/v1/...`; required authority, explicit local context. Array slots distinguish null `~` and bytes `b.<encoded-octets>`, including empty bytes; empty array has no slots. | Change before public stabilization only with explicit parser version and migration corpus; no implication of IANA registration. |
| Array shorthand | Server baseline with namespace-specific override, unset unless configured; slot 2 is only an example. | Configuration changes resolution locally; persisted addresses remain explicit arrays. |
| Numeric domains | Arbitrary-precision integers, decimal coefficient/exponent and binary-float significand/exponent. Ordered numeric registry excludes NaN/infinity; canonical positive zero. Type first for mixed-kind ordering, no implicit coercion. | Another comparison profile is explicit/versioned; arbitrary-precision domain is not replaced by float64. |
| Graph API | General typed-byte terms and references, bounded triple-pattern API first; RDF/SPARQL/SQL-PGQ remain adapters/research, not mandatory base semantics. | A language must preserve the base query/reference oracle and authorization; it does not get a second store. |
| Append moderation | Open means submission is permitted. Non-group submitters still enter pending review; only authorized group admission can append directly. | A future auto-review policy must be an explicit authorized review decision, not an unsigned/direct external list mutation. |
| Version relation | Canonical `(v1, succeeds, v2)` follows the directive; reverse ancestry index is derived. Comments remain attached to their exact old versions and carry forward with origin markers under current access/moderation filters. | A reverse API name must map explicitly, never reverse stored semantics unnoticed. |
| Federation budgets | Immutable origin query plus signed per-hop grants. Local aggregate reservation ledger splits child budgets; retries cannot mint fresh work. Every honest node enforces local ceilings; signatures do not force Byzantine nodes to obey a global budget. | A stronger distributed enforcement claim needs a separate protocol and proof; it is not inferred from max-hops. |
| Reputation | Signed attestations/evidence, locally weighted. Public claims are not verdicts; public redacted commitments need not disclose private evidence. | Portability does not create a global score authority; first-party servers get no privileged vote or exemption. |
| Compromise response | Minimal emergency disable/revoke/quarantine in V2; advanced incident/reputation automation in V4. | V3 federation cannot ship without the V2 safety baseline. |
| Receipt evidence | Typed, scoped signed assertions at a time. Receipt alone does not establish perpetual retention, availability or later query eligibility. | Any contradiction evaluator must prove comparable scope/time/policy/authority and applicable obligations. |
| GC concurrency | Participating-writer generation/tombstone fencing, not a non-atomic fresh-reference check. #9084 supplies the reference/lifecycle authority. | No collection enablement until the ownership seam and race model pass; this does not settle copy versus reference. |
| Cache freshness | Locally available source/policy watermarks plus bounded staleness; real-time/cache-disable bypass. Authorization revocation is not allowed the ordinary stale-content window. | A cache requiring a remote authoritative read on every hit fails the intended hop-skipping benefit. |
| Embedders | Snowflake Arctic is the current model family. #9092's second-embedder evaluation is still undecided; consume its versioned result. | No Cortex dependency or guessed second model is introduced here. |

## Owner question recorded without blocking

**Q1: original Part B candidate provenance.** The source asserts 30 candidates but
does not enumerate them. The bounded recovery log is
[candidate-provenance.md](inventory-live/candidate-provenance.md).

- Option A: pause decomposition until the original intake list is supplied.
- Option B (**recommended/default**): publish a clearly labeled, statement-complete
  replacement of at least 30 Part B slices, plus crosscut interop, preserving the
  seven named category anchors; reconcile the original list if it appears.
- Recorded on [#9106 comment 6843258938](https://github.com/developing-today/code/issues/9106#issuecomment-6843258938).
- Default B is in use. This does **not** mean the original list was recovered.
  If an original item adds scope, it receives an additional slice, never silently
  disappears or overwrites the replacement's provenance.

## Evidence-dependent work, not requests for a human oracle

- Cloud latency/economics and real multi-backend encoding comparisons need
  controlled experiments. Synthetic checks establish only their named invariants.
  The mill does not invent winners or request a product decision without evidence.
- An Atlas production `content_hash` producer contract must be traced to its actual
  pipeline and pinned. A consumer column is not an algorithm specification.
- CRDT adoption remains conditional. Research can conclude “do not adopt yet” and
  still complete its evidence task; conflict-report-first remains available.
- Scope, identity/SSO, tenancy and content-copy decisions remain with their sibling
  owners. Their open work is not a license for this program to settle it for them.
