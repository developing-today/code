# Independent review — architecture and V1 census slice

**Snapshot reviewed:** 2026-11-12 21:17 CST. **Scope:** the full contents of
`designs/architecture.md`, `evidence/census.md`, `proposals/architecture.json`
and `proposals/census.json`, checked against the parent architecture and
source precedence, risks/gates, and v0.3 spec §§1, 6, 25, 26, 35, 43 and 48.
This is the requested frozen-slice review, not a review of the whole design
tree or final source-inventory coverage. I made no edits to the four reviewed
artifacts.

## Disposition of the six packets

| Packet | Verdict | Finding |
|---|---|---|
| ARCH-01 — runtime identity/reference and V1 compatibility | **DISAGREE — clarify the API contract before publication as the canonical seam** | The public `ReadPolicy` and `ResolvedSecret` pseudocode does not expose all freshness context promised by ADR-S03. Details below. |
| ARCH-06 — shared signer/artifact authority interfaces | **DISAGREE — align the interface pseudocode before treating the architecture as settled** | The issue packet has grant-bearing output and signer calls, but the architecture's `ArtifactSink` / `PkiEngine` examples do not show how that authority reaches those calls. Details below. |
| ARCH-07 — registry and feature graph | **AGREE** | Correctly owns registry/feature wiring; OPS-05 help generation is soft coordination, not a blocking dependency. No cycle in this slice. |
| ARCH-10 — profile dependency closure and budgets | **AGREE** | Separates the existing connect hard gate from unratified, measured-later budgets; depends on ARCH-07 and PERF-01. |
| TRACE-01 — canonical traceability CI guard | **AGREE** | Reuses the canonical inventory and reviewed mapping; explicitly does not claim completeness while mapping/receipts are missing. Backlog is the right lifecycle. |
| TRACE-02 — V1 CLI/profile golden corpus | **AGREE** | Tests the existing contract without turning the V1 absence of cert/generate into a permanent ban. Its expected `get` value on stdout and redacted diagnostics are correctly distinguished. |

## Findings requiring design clarification

### A-01 — Runtime read interface omits ADR-S03 request/result context

**Severity:** Medium, but required before freezing the public read contract.

In `designs/architecture.md`, `ReadPolicy` has `max_age`, `stale`, allowed
sources and `require_version_proof` (lines 43–49); `ResolvedSecret` has source,
revision, fetched-at and a boolean `stale` (lines 50–57). The prose gives
`DataClass` (line 42) and says stale policy is class-dependent, but no request
field carries purpose, offline allowance or revocation/security class, and the
result has no explicit freshness decision/degraded-use reason. The parent ADR-S03
requires those facts on the request/result. Spec §35.4 requires selected-source
and staleness status; §37.2 makes offline age/class/revocation policy explicit.

`PERF-02` may own freshness *semantics*, but ARCH-01 owns the API that must carry
those semantics. Either add a typed `ResolutionContext` / policy fields and an
explicit `FreshnessDecision` result, or state precisely which fields are
resolved from stored credential policy and how the caller's purpose/offline
intent is represented. Do not put the policy model in a second resolver.

**Acceptance oracle:** A fake clock/provider model varies purpose, class,
revocation, offline state and age for the same logical reference. It must show
fresh-only denial, permitted bounded stale use with an explicit degraded reason,
and deny-overrides-cache behavior for revoked/high-risk classes. The result
receipt identifies source/revision/fetched age without exposing bytes.

### A-02 — Shared signer/output authorization is not represented consistently

**Severity:** High authority-interface ambiguity; ARCH-06's issue packet has
most of the required contract, but the architecture API examples disagree.

`proposals/architecture.json` ARCH-06 passes `SigningAuthorization` through a
single-tuple `SignerSession` / `DurableTupleClaim`, requires atomic durable
`consume_once`, has a concurrent winner and crash/uncertain-result oracle, and
passes `OutputAuthorization` to `ArtifactSink.write`. That is consistent with
the current architecture prose requiring final TBS/SPKI/serial binding, consume
before signing, burned uncertain tuples and no blind retry.

But `designs/architecture.md` declares `Signer::sign(claim, exact_payload)`
while `PkiEngine::issue_leaf(req, signer: Arc<dyn Signer>, ...)` accepts neither
the claim nor a signer session/context (lines 127–168). Its `ArtifactSink::write`
signature accepts only `artifact` and cancellation (lines 170–173), although
the next paragraph requires bound authorization and destination permission for
private output. As written, the public call surface does not show where those
unforgeable authorities enter.

**Required fix:** Make the design pseudocode match ARCH-06: issuance receives a
validated signer session/tuple-claim context (or an equivalent unforgeable
context), and private artifact output receives a bound `OutputAuthorization`
or an unforgeable authorized sink. State the invariant that a raw signer handle
or ordinary `ArtifactSink` cannot sign/write private output. Keep ARCH-06 the
single owner of the shared types; PKI consumes them.

**Acceptance oracle:** Compile-time/API tests show a root/issuer signature and
private artifact write cannot be requested without the appropriate validated
claim/grant. Denied, expired, wrong-tuple and wrong-destination requests make
zero signer/sink calls. Existing atomic consume/race/crash tests remain.

## Cross-cutting checks that passed

- **V1 boundary and test evidence:** The census pins baseline
  `34faffbff26615e313488f6d04705a444342e41e`, separates source behavior from
  deployment/permission claims, and correctly records `fleet secrets get` as
  intentionally returning the requested value on stdout while diagnostics are
  redacted. Its 16 and 306+2+1 test counts are explicitly qualified as bare
  command runs with no resource wrapper and no durable original test log; they
  are not presented as independently archived release receipts or a full
  Fleet/Nix build. Do not promote them beyond that evidence status.
- **Identity/root authority:** The ordinary `SecretRef` uses an opaque stable
  `CredentialId`; `SecretAlias` is scoped and mutable. `SecretBytes` is not
  implicitly cloneable/serializable, and the document does not derive `Clone`
  for it or `ResolvedSecret`. RootPrivate is denied in the ordinary resolver,
  cache, prefetch, benchmark and equality-index paths; escrow and signer
  authorities remain separate.
- **stdout and compatibility:** The census, ARCH-01 packet and TRACE-02 packet
  agree on successful `get` stdout, names-only `list`, and diagnostic redaction.
  TRACE-02's positive oracle expects the sentinel on stdout; its negative oracle
  scans stderr/diagnostics instead of incorrectly requiring the value to be
  hidden from the caller.
- **Source identifiers:** TRACE-01's `COORD8-0001` and
  `SPEC-R-evidencestate`, and TRACE-02's `SPEC-OPS-005` and `SPEC-0001`, are
  present in the frozen canonical inventory. The inventory README explicitly
  states that 113 oracle reviews and 233 atomicity reviews remain and that
  atomic completeness is not yet established. The packets do not claim those
  gaps are already closed.
- **Dependencies and lifecycle:** In this six-packet slice, TRACE-01/02 have no
  prerequisite; ARCH-01 depends on TRACE-02; ARCH-06 consumes OPS-01; ARCH-07
  depends on ARCH-01/06 and domain descriptors, but not OPS-05; ARCH-10 depends
  on ARCH-07 and PERF-01. This graph is acyclic. TRACE-01 is Backlog until its
  reviewed mapping/receipt inputs exist; TRACE-02 is Ready for synthetic V1
  characterization; the architecture packets remain Backlog. None implies
  production authority.
- **Primary-kind normalization:** The composite authored kinds for these six
  packets have explicit entries in `traceability/normalization-policy.json`.
  `prepare_manifest.py` normalizes to the publisher's allowed primary kind,
  records the original composite kind and rationale in the issue body, and has
  tests for exact normalization and rejection of unreviewed composites. Do not
  replace this with a heuristic `split('+')` rule.
- **Urgency and estimates:** The six packets use the current lower-is-sooner
  charter with High/600 or High/450–500 scores; lifecycle states are valid.
  TRACE-02's `S/M` estimate is a relative size range, not a calendar-time
  promise. Preserve numeric urgency/band separately from queue projection.
- **Source truth:** Spec §24.2 directly gives 13 issuer certificates per
  partition, four purpose/partition groups = 52 issuer certificates, plus 13
  root variants = 65 CA certificates. No “32+20” conflict is in the assignment
  or governing sources. My earlier integration-draft I-6 wording has been
  explicitly corrected; no human decision or issue should be created for that
  invented conflict.

## Publication recommendation

Do **not** publish ARCH-01/ARCH-06 as the final authoritative interface
contracts until A-01/A-02 are reconciled in the architecture text and the
proposal bodies continue to match. The six packets otherwise have valid
owners, useful acceptance oracles, allowed normalized primary kinds, valid
urgency/lifecycle fields and an acyclic dependency structure when the combined
manifest includes all referenced domain/census proposals. This is a slice
review only; it does not certify other domains, source-inventory completeness,
global proposal DAG, or final mission completion.

## Review snapshot provenance

Read 2026-11-12 21:17 CST; SHA-256 values below identify the reviewed bytes.
Re-hash before publication if any author has edited a frozen file.

| File | SHA-256 |
|---|---|
| `designs/architecture.md` | `286bbe0613e40a3b9d896d1271a49c58686d7a1081e47d7ad658fc02ece8c09e` |
| `evidence/census.md` | `b07e648dcfed990e1eab7e996b12572937f35b8770cacb1bf885cdc60f9850a0` |
| `proposals/architecture.json` | `f3a87c1e327eb0f679b1d4a91a2c969118ca2bb81443c55ee8957f520f88badf` |
| `proposals/census.json` | `40913c01d5a81f57b36794917a8f3c963178a949068b58f5dbf91e23198369d6` |
| `architecture-review.md` | `8e2f3f3b61999c953b8be9bc94cde8b377da99497a9591acd01311043e12c028` |
| `source-precedence.md` | `22a1aef6e08395ceabe6c2a0fdd78bc6c45cefaa9050263b5e7bbdb663e1fd8d` |
| `risks-and-gates.md` | `1f7f3aa17f8ad663a76faa62088bfe035105bc47d594a44e5881282806988212` |

## Author-fix re-review — 2026-11-12 22:03 CST

The author-fix hashes match the current files: `designs/architecture.md`
`cd4bf0f1ef581618b7a1ce4944624d0d95888d88ad5fa5c471d6ce667bce3645`,
`proposals/architecture.json`
`b686619fee6900b199a5b8d0fe09dab5ca4b7c28777935e433204bc1d11952bd`, and
`evidence/census.md`
`9d932db575f9cc6899cf3b2687d2a7d884c0f25989184ebce9f9a55eb50edf60`.

### Updated verdicts for the six packets

| Packet | Re-review verdict | Closure |
|---|---|---|
| ARCH-01 | **AGREE** | A-01 is closed. `ResolutionContext` carries purpose, offline intent, required assurance and a caller max-age ceiling; a trusted policy source supplies stored class, revocation, scope/native binding, age limit and assurance floor. The caller can only tighten these. `ResolvedSecret` returns age, revocation and an explicit freshness/degraded decision. PERF-02 remains the semantic owner; this is only the shared API shape. |
| ARCH-06 | **AGREE** | A-02 is closed for the shared signer/output contract. The signer call consumes a one-shot tuple claim; `consume_once` is atomic/durable, tests cover concurrent consumers and crash/lost-result states, and artifact output takes an `OutputAuthorization`. The updated issuance path prepares exact SPKI/serial/TBS before authorization and does not accept a raw signer-only call. |
| ARCH-07 | **AGREE** | Registry/feature graph remains one owner; OPS-05 help integration is explicitly soft, with no blockedBy edge. |
| ARCH-10 | **AGREE** | Profile/feature closure and provisional size/startup targets remain measurable proposals, not results or ratified gates. |
| TRACE-01 | **AGREE** | Canonical inventory/traceability CI guard is scoped to existing data and stays Backlog until reviewed mappings/receipts exist. |
| TRACE-02 | **AGREE** | V1 golden corpus expects the successful `get` sentinel on stdout and redacts diagnostics; capability 69 and current profile absence are versioned baseline facts, not permanent prohibitions. |

The former A-01/A-02 disagreements in this file are superseded for these
snapshots. The six packet set is **publication-ready**, provided the
normalization/publisher run includes all cross-file dependencies (census,
architecture and referenced domain proposals) in one combined manifest. The
composite primary-kind normalizer is exact and preserves the authored kind;
urgency bands/lifecycle are valid. TRACE-02's S/M relative estimate is not a
calendar promise.

### One bounded PKI handoff clarification, not an ARCH-06 packet blocker

The updated high-level `PkiEngine::issue_root` signature at
`designs/architecture.md:225–230` explicitly supplies the root key, ceremony
authorization, signing-authorization port, signer and durable authorization
ledger. Unlike `prepare_leaf` at lines 211–213, it does not show a serial-ledger
or prepared-root input. The pinned original spec §5.2 (snapshot lines 235–241)
requires unique per-issuer serials and a collision-free issuance ledger; the
reference root/cross-certificate set has 13 root variants (§24.2, snapshot
lines 1423–1436). ROOT-02 already owns the ledger, so this does not reopen the
generic ARCH-06 authorization contract or hold its packet. Before PKI
implementation, state that root preparation reserves each self/cross
certificate serial through ROOT-02 (or an injected `SerialLedger`) before
freezing TBS and requesting authorization. Oracle: concurrent root serial
reservations never collide; crash/retry never reuses a reserved serial, and
each final authorization tuple contains the reserved serial and exact TBS
digest.

### Source and scope confirmation

The re-review used only the original source snapshot at
`source-snapshots/intake-34faffbff2/sources/`, pinned to
`34faffbff26615e313488f6d04705a444342e41e`; the spec blob SHA-256 is
`a17559474cc82992edf374615c28547dcbbdfe051481bae41cc0bfece018e6bb`.
The source-integrity receipt confirms the five covered `SHA256SUMS` entries
pass and the formatted `sources/` view is not the line/excerpt oracle. Spec
§§1, 6, 25, 26, 35, 43 and 48 support the reviewed multicall, profile, API,
capability, compatibility and evidence contracts.

The exact §24.2 graph remains 13 issuer certificates per partition, four
purpose/partition groups = 52 issuer certificates, plus 13 root variants =
65 total. There is no user/source “32+20” split; the integration-draft I-6
retraction remains the correction of record. The implementation-roadmap and
other-domain packets are outside this frozen-slice verdict and remain for the
parent's later whole-tree review after the mapping/tools freeze.

## Re-review after author A-01/A-02 fixes — 2026-11-12 22:07 CST

The author-frozen hashes named by the parent match the files re-read here:

| File | SHA-256 |
|---|---|
| `designs/architecture.md` | `cd4bf0f1ef581618b7a1ce4944624d0d95888d88ad5fa5c471d6ce667bce3645` |
| `evidence/census.md` | `9d932db575f9cc6899cf3b2687d2a7d884c0f25989184ebce9f9a55eb50edf60` |
| `proposals/architecture.json` | `b686619fee6900b199a5b8d0fe09dab5ca4b7c28777935e433204bc1d11952bd` |
| `proposals/census.json` | `40913c01d5a81f57b36794917a8f3c963178a949068b58f5dbf91e23198369d6` |

I re-read the four artifacts in full and re-checked source requirements only
from
`source-snapshots/intake-34faffbff2/sources/fleet_architecture_spec_v0_3.md`
(pinned source commit `34faffbff26615e313488f6d04705a444342e41e`, original
SHA-256 `a17559474cc82992edf374615c28547dcbbdfe051481bae41cc0bfece018e6bb`).
`source-integrity.md` records that original blobs were copied without alteration
and that the formatted `sources/` view is not the byte/line oracle. This review
uses the snapshot for §§1, 6, 25, 26, 35, 43 and 48; it does not infer a new
source version from upstream formatting.

### Updated six-packet verdicts

| Packet | Final slice verdict | Re-review finding |
|---|---|---|
| ARCH-01 | **AGREE** | A-01 is closed: `ResolutionContext` carries purpose, offline intent, required assurance and a caller maximum-age ceiling; a trusted policy source supplies stored class, revocation, scope binding and policy limits; the caller can only tighten. `ResolvedSecret` carries source age, revocation and an explicit freshness/degraded decision. This makes PERF-02 consumable without duplicating its semantic owner. |
| ARCH-06 | **AGREE** | A-02 is closed in both the shared interface and issue packet: prepared SPKI/serial/TBS/plan tuple → authorization port → atomic durable `consume_once` → signer claim/session → signed result; private output uses `OutputAuthorization`. The packet's concurrency and crash/lost-receipt oracle is explicit. No raw `Arc<dyn Signer>` path bypasses the claim in the updated pseudocode. |
| ARCH-07 | **AGREE** | Registry/feature wiring remains its sole owner. OPS-05 is soft coordination, not a blockedBy edge; this removes the former ordering concern. |
| ARCH-10 | **AGREE** | Profile closure, link exclusion and measured build/startup budgets are scoped; new thresholds remain provisional, not passed or release-blocking before measurement/ratification. |
| TRACE-01 | **AGREE** | It gates the existing canonical inventory/mapping/receipt inputs, does not create a competing source of truth, and remains Backlog while reviewed mappings/receipts are incomplete. |
| TRACE-02 | **AGREE** | It characterizes V1 only. The positive `get` oracle expects the requested value on stdout; diagnostics redact it. Missing `cert`/`pki`/`generate` is a versioned baseline snapshot, not a permanent prohibition. The census's bare test executions remain explicitly non-durable observations, not release evidence. |

The primary-kind normalizer maps each packet's exact composite `kind` to one
coordinator-supported primary kind and preserves the authored composite plus
reason in issue metadata. The six urgency scores/bands and lifecycle values are
valid under the stated intrinsic scale; queue projection remains separate.
TRACE-01/TRACE-02 source IDs match the frozen canonical inventory. For
publication, the normalizer must be given the combined proposal arrays so
ARCH-01's TRACE-02 and other cross-file dependencies resolve; the six packets
are not six independently normalizable manifests.

### Remaining bounded integration note (not a packet blocker)

The shared signer contract is now coherent. In the illustrative high-level
API, `issue_root` names an explicit root key, ceremony authorization, signing
authorization port, signer and durable authorization ledger, but unlike
`prepare_leaf` it does not show a `SerialLedger`/prepared-root input. Root
serial allocation belongs to ROOT-02/PKI domain ownership, so this does not
reopen ARCH-06 or the six packet verdicts. Before implementing the root helper,
the PKI design should explicitly say that root/self/cross-certificate serials
are reserved durably before TBS authorization and signing, via an injected
ledger or a typed prepared-root object. The acceptance oracle is unique
per-issuer serials under concurrency plus crash/retry with no reuse.

### Source-truth correction retained

Spec §24.2 in the original snapshot directly defines 13 issuer certificates
per partition, four purpose/partition groups = 52 issuer certificates, plus 13
root certificate variants = 65 total CA certificates. The assignment and
snapshot sources do not contain a “32+20” conflict. The prior integration-draft
I-6 wording has been explicitly retracted; the remaining ROOT-06/CERT-02
ordering point concerns exact signer-root CRL ownership against that single
source-defined graph, not a human approval or ambiguous split.

**Scope boundary:** this is a final review of the architecture/census slice
only. It does not approve the other domain drafts, global proposal graph,
canonical inventory completeness, issue publication, or the overall design
mission. The parent should run the whole-tree review after inventory/tools and
the other domain packets freeze.
