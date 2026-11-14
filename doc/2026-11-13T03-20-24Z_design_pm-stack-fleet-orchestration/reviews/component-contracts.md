# Independent technical contract review — task 15381 / #9196

**Scope:** technical correctness, reliability, performance, extensibility, and reuse of the
current repository. Reviewed `architecture.md`, components #9199/#9200/#9201/#9203/#9204/
#9206/#9207/#9208/#9209, FDR-PMO-001–008, Track A/B census, command inventory and slice map,
and source 01 plus the complete v1.1 charter (source 02). Source 03/04 were consulted only
where useful for concrete requirement examples. This review does not judge publication,
final coverage, implementation, or deployment readiness.

## Verdict: disagree — not yet a complete, implementation-ready technical contract

The shared architectural direction is technically coherent and reuses the correct authorities
and existing seams. The findings below are narrow but should be corrected before adopting the
design as a complete contract. No other blocking technical defect was established in this pass.

## Findings

### High — protected expedite work lacks an enforceable write-authority gate

**Evidence:** `pm-stack/design-reprioritization-2026-11-12.md:11-15,22,122` says Drew's
expedite set is pinned and untouched, that nothing in the program demotes an expedite item,
and that the set is his alone. Its applied record at `:207` says ten linked tasks were
deliberately left unchanged. In contrast, `components/9199-distributed-priority.md:28-33,58-62`
permits authorized manual edits and manual bypass of hysteresis, while
`components/9200-priority-distribution.md:37,52-55` gives routine authorized humans the
highest field-edit precedence and accepts `brief_ruling` for an owner ruling. The shared
authority interface (`architecture.md:114-120`) requires authorization but does not encode
this protected-set restriction or its principal/field scope.

**Counterexample:** an otherwise authorized triage or manual-priority command targets one of
the protected expedite tasks and changes its urgency, or an automatic clump/aging sweep moves
it. The generic authorization and hysteresis rules allow the operation even though the
current owner ruling forbids this program from touching that task and reserves the set to
Drew alone.

**Fix:** add a versioned, explicit owner-policy gate for the protected set to the shared
priority/field-authority contract. It must identify the protected membership/source and the
exact principal/action allowed to change it; automatic triage, label normalization, clump
smoothing, and routine manual edits must not override it. Consume the existing ruling; do not
seek re-ratification or make this a wait-for-owner-pointer gate.

**Oracle:** model tests attempt each mutation path against protected and ordinary items.
Every unauthorized actor and every automatic policy path must produce a typed rejection/no
change for protected items; the specifically authorized owner action must preserve exact
before/after provenance. Include membership/version changes and stale policy snapshots.

### Medium — cross-language digest framing is not fully normative

**Evidence:** `architecture.md:153-179` chooses RFC 8785 + SHA-256 with a purpose/version
domain separator for command fingerprints and manifests, and requires Rust/TypeScript golden
vectors. It does not define the exact domain-separator bytes/framing, whether/how the separator
is concatenated with canonical bytes, or the wire encoding of the resulting digest. Separately,
`components/9200-priority-distribution.md:116` requires a deterministic `order_hash` over
identities and comparator inputs, but does not define its canonical DTO/byte encoding or hash
domain. RFC 8785 evidence at `research/canonical-json-evidence.md:21-23,34-36` correctly notes
that application digest semantics are a Fleet choice, not supplied by RFC 8785.

**Counterexample:** two conforming implementations use different encodings, e.g. a NUL-delimited
ASCII prefix versus a length-prefixed domain, or hash different but plausible order-hash input
objects. They produce different idempotency keys/receipts or report cross-target order divergence
despite identical semantic inputs; alternatively, domain concatenation without unambiguous
framing can blur protocol domains.

**Fix:** specify a single byte-level framing for each digest purpose (including fixed domain
bytes/version, canonical DTO schema and output encoding). Define the `order_hash` input DTO and
whether it covers ordered IDs only or also all comparator/snapshot inputs. Keep command, manifest,
and order-hash domains distinct; do not extend the legacy creation-outbox hash.

**Oracle:** publish independent Rust/TypeScript golden byte strings and digests for each
purpose; assert the same semantic input matches cross-language, an altered included input differs,
and cross-purpose inputs cannot share a domain. For order hash, verify identical pinned snapshots
match and a changed comparator/snapshot input changes the hash.

## Positive contract checks

- `architecture.md:20-37,262-306` correctly treats one broker as the target, not a false V1
  claim; documents the #9028 PostgREST bypass; limits projector reuse to read/transport
  mechanisms; and assigns implementation to proposed internal `fleet-queue` modules and
  existing owners rather than inventing a service, queue, DB, binary, or GitHub client.
- Claim eligibility consistently requires both task requirements and caller filter; the claim
  boundary revalidates current inputs rather than trusting a projection or sample. Priority
  preserves the #7030 15/25 values and numeric work-ID order (9 before 10).
- The designs do not promise remote exactly-once, magical graph CAS, or a GitHub lock/fence.
  They distinguish intent from verified effect, preserve partial completion, describe late
  activity races, and retain uncertain receipts for recovery.
- Projector delivery/applied/high-water acknowledgments are consistently kept distinct from
  Fleet command acceptance and operation durability. Hydration cursor advancement follows
  durable page ingest, with staged-object/orphan recovery where DB and artifacts cannot share a
  transaction.
- Strict parsing, mutating `work.link` ownership, sole CLI registration glue (CLI-13), exact
  future test targets, and zero-match test-filter guards are materially stronger than
  hand-wavy implementation contracts. The Playwright slice is scoped as a future CI harness,
  not a new service or claimed test run.

## Follow-up review — 2026-11-13

Re-reviewed the actual amended architecture, #9199/#9200/#9208, FDR-PMO-002, RFC evidence,
and `contracts/digest-vectors.json`. This follow-up supersedes the current-status verdict
above; the two original findings remain as an audit trail, not outstanding blockers.

1. **Protected owner-priority policy — resolved.** `architecture.md:277-313` now makes the
   existing Drew-only ruling an explicit versioned policy, uses canonical protected membership
   rather than mutable labels, binds the trusted configured owner principal and exact allowed
   action, checks `owner_policy_ref` atomically with work version, rejects routine/automatic/
   `--force`/confidence/hysteresis bypasses, and defines fail-closed stale/denied outcomes.
   It preserves exact #7030 15/25 semantics and explicitly rejects the obsolete band-90 table
   as authority. The shared Command includes `owner_policy_ref` (`architecture.md:101-107`);
   #9199 triage, #9200 producer precedence, #9208 field authority, and FDR-PMO-002 now point
   to the same gate and oracle cases. This closes the protected-set mutation counterexample
   without requesting re-ratification.
2. **Digest framing and order-hash inputs — resolved.** `architecture.md:176-244` specifies
   exact byte framing (`ASCII(domain) || 0x00 || UTF8(JCS(dto))`), output syntax, a closed
   three-domain registry, exact command-intent and full eligible-population order DTOs, and
   incomplete-snapshot behavior. The manifest digest omits only its own field and artifacts
   hash raw bytes (`:246-251`). The framing vectors are correctly scoped as framing-only, not
   application-schema proof (`contracts/digest-vectors.json:1-29`). I independently recomputed
   all three framed byte strings and SHA-256 values offline; all matched. FDR-PMO-002 now
   declares the shared architecture normative.

**Current verdict: agree — the reviewed technical design scope is complete and internally
consistent.** Remaining Rust/TypeScript RFC/JCS, full application-DTO, and cross-target
conformance tests are explicitly future implementation acceptance, not claimed as run and not
a design-scope blocker. Issue publication/coverage and implementation/deployment/live proof
remain separate gates and are not implied by this verdict.
