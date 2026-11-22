# Cross-domain integration audit — DRAFT snapshot

**Read-only audit date:** 2026-11-12, America/Chicago. **Worktree:**
`/home/user/git/fleet/15345-fleet-ca-buildout`, branch
`design/fleet-ca-v03-buildout`, HEAD `bdafeecae6` at audit start.

**Not a final review.** Domain owners were actively editing these documents
during the read. The findings apply only to the SHA-256 snapshots below, are
provisional, and must be rechecked against frozen files before being used as
final review comments or issue-publishing input. I did not edit domain/source
files or create/update issues. Absence of a proposal packet below means absent
in this snapshot, not a decision to drop its source requirement.

## Parent closure re-review

- **A-1: closed.** The parent now says unguarded operations stop by default,
  distinguishes an explicitly approved weaker policy, names its residual
  lost-update risk, and requires the in-place §20.5 saga to remain a routine
  reconciliation deliverable. This is consistent with spec §§20.3, 20.5 and
  35.1: history-first, native-ID in-place update remains required, and lack of
  CAS is not misrepresented as serialization.
- **A-2: closed at the design-contract level.** Parent ADR-S01 binds grants to
  a finite enumerated signature-tuple set, calls for per-tuple durable
  consumption, and requires uncertain submissions to be reconciled before
  retry. The current PKI drafts make consumption-before-signing and burning
  uncertain tuples explicit. A single-use default is compatible with 65
  variants: each signature can use its own tuple grant, or the complete
  multi-signer signature set can be enumerated as bounded use. It must never
  authorize later/unlisted signatures. Proposal acceptance still needs a
  concurrent-consumer/crash oracle; see I-5.
- **A-3: closed.** The parent, README and current root-custody draft retain the
  closed laptop/BWS/1Password set, independent restores, incomplete-custody
  status and R2 research-only treatment, consistent with addendum §5.

No source contradiction was found in those three parent closures. They do not
make the domain designs or issue packets final.

## Actionable draft findings

### I-1 — Root recovery dependency is cyclic in the issue packet

**Severity:** High; blocks a valid issue dependency DAG, not synthetic work.

In `proposals/pki.json`, ROOT-09 depends on ROOT-10 while ROOT-10 depends on
ROOT-09. `designs/pki-roots.md` has ROOT-09 depending on ROOT-02/05 and ROOT-10
depending on ROOT-09, so the narrative and machine packet disagree. An issue
publisher cannot create the packet's cycle as native `blockedBy` edges.

**Desired fix:** Separate any recovery-policy/design prerequisite needed before
an escrow adapter from the post-adapter independent restore rehearsal; make
the latter depend on ROOT-09 only. Align the markdown and JSON dependency rows.

**Acceptance oracle:** Parse the canonical proposal graph and assert it is
acyclic; for each listed dependency assert prerequisite completion precedes
the dependent step. No native issue graph contains a cycle.

### I-2 — Stale proposal IDs remain in the architecture routing text

**Severity:** High; risks creating duplicate or dangling issue ownership.

At this snapshot, `proposals/secrets-reconciliation.json` contains SEC-02–06
and REC-01–08, not SEC-07/SEC-08/REC-09. The current `secrets.md` and
`reconciliation.md` say PERF-02 is the canonical freshness owner and explicitly
retire SEC-08/REC-09. Nevertheless `designs/architecture.md` still routes
freshness to PERF-02 **and SEC-08**, lists REC-01–REC-09, and routes root
custody to SEC-07; `designs/generators.md` also names SEC-07. No SEC-07 packet
is present; root custody is instead owned by ROOT-09/10 and OPS-10.

**Desired fix:** Replace stale references with PERF-02, REC-01–08 and the
current root-custody owners, or explicitly retain an ID as a named migration
alias with no publishable body. Keep one canonical owner for each freshness
and root-custody deliverable.

**Acceptance oracle:** Extract all proposal IDs and all cross-document
references; every reference resolves to a current packet or an explicitly
labelled historical alias, and every normative freshness/root-custody
requirement has exactly one implementation owner plus named consumers.

### I-3 — Future-seed register points to a missing packet file and has wrong routing

**Severity:** High traceability gap; blocks claiming §15/§27 idea disposition
complete, not baseline implementation.

`research/future-seeds.md` says its SEED-01–SEED-21 dispositions map to
`proposals/idea-seeds.json`, but no such file exists in the proposal directory
at this snapshot. The register maps §27.4 OCapN and §27.3 Fleet `id`
integration to FUT-12, whose current packet is specifically the SOPS/age,
fnox and Fleet-cache offline-continuity comparison (`proposals/future-infrastructure.json`).
That is not an OCapN or Fleet-id deliverable. It also maps §27.1 systemd sync
rollout to OPS-10 (approval/custody gates) without naming REC-08's schedule
contract.

**Desired fix:** Publish one self-contained packet per genuinely new SEED-xx,
or route each source item to an existing packet whose title, deliverable and
acceptance actually cover it. Correct OCapN/id and daily schedule mappings.
Do not create duplicates for items already owned by FUT/GEN/OPS/REC/CERT/ROOT.

**Acceptance oracle:** For every normative §15/§27 source row, a machine audit
finds a specific existing proposal ID or a concrete SEED-xx packet with
requirement/source IDs, one deliverable, owner, dependencies, boundary and
decidable acceptance. No row is “covered” solely by a domain-starter number;
no seed ID is dangling or reused for an unrelated concept.

### I-4 — Proposal urgency values and labels are not normalized to the current charter

**Severity:** Medium; must be corrected before issue publication/routing.

The current charter is explicit in `designs/architecture.md`: intrinsic
urgency is lower-is-sooner; High is 400–799, Standard 1000, Low 1400,
Someday 1800; this is distinct from Project/queue priority. In the captured
`proposals/generators-ops.json`, GEN-03/06/07/10 and OPS-05/06/07/09/11 have
urgency 300, outside every charter band. That packet also lacks an
`urgency_band` field. `proposals/architecture.json` and
`proposals/secrets-reconciliation.json` also omit the band; `performance.json`
uses a `priority` label rather than the charter band. These schemas cannot
reliably distinguish intrinsic urgency from queue/project priority or show a
valid migration from the older numeric scale.

**Desired fix:** Normalize every proposal to an allowed intrinsic urgency and
matching band, with a recorded old-to-new mapping where values were migrated.
Keep native Project/queue 20/40/70 values separate; do not copy the intrinsic
number into those fields. Re-estimate rather than blindly map a legacy `300`.

**Acceptance oracle:** A schema validator rejects out-of-band numbers,
missing/mismatched bands, and fields conflating intrinsic urgency with
Project/queue priority; validate all proposal JSON before publisher effects.

### I-5 — Root signing authorization is not yet a closed issue-level test contract

**Severity:** High security acceptance gap before signer implementation; the
architecture decision itself is closed.

The current root drafts specify finite signature tuples, durable consume before
signing, and burning uncertain tuples. The ROOT-05 packet in
`proposals/pki.json` still tests a fake invocation counter and replay rejection
but does not require two concurrent consumers to race for the same tuple, a
crash between durable consume and signer invocation, or a lost response with
no receipt. ROOT-03 and ROOT-04 generate self/cross-signed roots but do not
depend on ROOT-05/ARCH-06 or explicitly require the shared synthetic
authorization seam. This leaves room for graph fixtures to sign directly and
never exercise the required policy.

**Desired fix:** Use the shared ARCH-06 authorization type/session; define an
atomic single-winner consume point before signer invocation. Make an uncertain
tuple unusable until an authoritative receipt/audit reconciliation proves the
outcome; absent proof must remain blocked, never be treated as unused. Add
ROOT-05/ARCH-06 prerequisites to ROOT-03/04 or state that their synthetic
signer uses the identical authorized seam.

**Acceptance oracle:** Race two consumers for one tuple: at most one signer
invocation. Inject crashes before claim, after durable claim/before call,
after call/before receipt, and after receipt. Replay or retry cannot yield a
second signature; an unavailable status query leaves the tuple uncertain and
blocked. Test that ROOT-03/04 fixtures cannot produce a signature without the
shared authorization path.

### I-6 — Root-signer CRL ledger must depend on the frozen certificate graph

**Severity:** Medium integration ordering gap.

ROOT-06 in `designs/pki-roots.md` and `proposals/pki.json` now owns the
root-signer CRL ledger; its stated oracle compares signer-root ownership with
the ROOT-04/CERT-02 certificate-edge ledger. Its dependency list names ROOT-04
and ROOT-05 but not CERT-02. CERT-02 is the explicit 65-certificate graph
freezer. Without the dependency, the CRL owner can implement before the exact
issuer-of-record edge set is frozen. Spec §24.2 itself is explicit: 13 issuer
certificates per partition, four purpose/partition groups for 52 issuer
certificates, plus 13 root variants, for 65 total CA certificates. There is no
source-level split ambiguity.

**Desired fix:** Add CERT-02 as a blocking dependency of the root CRL ledger
(or make a canonical, versioned graph schema prerequisite that CERT-02 owns).
Keep CERT-09 scoped to issuer-signed leaf CRLs, as the current documents say.

**Acceptance oracle:** The ROOT-06 fixture consumes the exact frozen
CERT-02 manifest; generated root CRL ownership is set-equal to certificates
whose signer is that root. A changed graph version invalidates/reviews the
CRL ledger instead of silently reusing it.

### I-7 — Exact root-reference expiry is not asserted on every root certificate variant

**Severity:** Medium source acceptance ambiguity; can be resolved in PKI design
before implementation.

The root design states the reference root expiry `2050-07-07T12:00:00Z`, and
ROOT-03 tests that date for four self-signed roots. ROOT-04's nine root
cross-certificates and CERT-02's full root graph do not explicitly assert the
same reference `notAfter` or state their separate validity cap. The governing
spec treats the exact reference root expiry separately from the general
20-calendar-year default (§§24.1, 33.2).

**Desired fix:** State whether the exact timestamp applies to all 13 root
certificate variants; if cross-certificates differ, specify their bound and
rationale. Put the rule in the canonical graph manifest/test, not only the
self-signed-root packet.

**Acceptance oracle:** A test enumerates all 13 root certificates and asserts
the explicitly chosen validity rule against each signed manifest row; defaults
remain tested separately.

## Future-seed routing notes

The seed register correctly labels its scope nonbinding and does not change
ML-DSA-44, the 20-key/65-cert graph, closed escrow, or optional/nonblocking
MTC. However, it must not be used as completed coverage until I-3 is resolved.
In particular, source §27.4 OCapN and §27.3 Fleet-id items cannot be counted as
covered by FUT-12's offline-continuity packet. Novel seeds should remain
strictly limited to uncovered §15/§27 source items; existing packet owners
should be cited rather than duplicated.

## Snapshot provenance (provisional)

SHA-256 captured 2026-11-12 20:35 CST. Findings above are only asserted
against these exact snapshots; active writers may already have changed them.
Re-hash and re-read before final review.

| File | SHA-256 |
|---|---|
| `architecture-review.md` | `48cd2e00a3f0caaaba9f9945217eed4b27366a3c31d9a04daa102b1b2afbf0b0` |
| `README.md` | `b4cab554c229f3071c13fc1ac739a5ec2d86091005a5f1dd345109348bc3aaac` |
| `risks-and-gates.md` | `962b4e7fd11038bbd214824c8f02ba7c5884c541966d57a1f0c7a36a927655ea` |
| `designs/architecture.md` | `0c408c613718ee7360134b831d9a888e017f97bf03204c1aad6fb63f30d95a51` |
| `designs/federation-infrastructure.md` | `9c79078ad0abebef2547d4b9d6f663d18a6c80b9cc07598f0f06efdc5d7161b3` |
| `designs/generators.md` | `c0dcd1e249132e4e0fe538e5d6707f665bd41bd33c8b6e6da45d4110e22661c4` |
| `designs/operations-evidence.md` | `25711d43aa8b9b761252b64a5fbcb1154e40ad3cb94bd4e94e8d7183b4ee54c5` |
| `designs/performance-formats.md` | `de62b9ed7936cd356bd665b05509ffb56557a3ab64be4f56a15aa6dd77a42874` |
| `designs/pki-issuance.md` | `3608814d3db4d1fe21aadae970fe99917a415375cd50a30f695c4fff2e237d0c` |
| `designs/pki-roots.md` | `e9deb6e71f34622531cdd2651228acf9ab8b2f78c7a01ff62e45cfd481ccad2b` |
| `designs/reconciliation.md` | `210ea27f68a67221ebbca3a28c24e9342b759f0c50f4131c1cc5fa59b0654feb` |
| `designs/secrets.md` | `ddad1e29226c6c4445b8deb34ee114ec7ac8534e8419ad3c9223178d56cf6bd8` |
| `proposals/architecture.json` | `5ef6ba4266fab90bd0760f48ff08dbe46ab82f524a09d9e002b38fb1b217f05d` |
| `proposals/future-infrastructure.json` | `9ffb83536fb0da4b7cd852219ec30ca2cdb0e8b79fcdb4a1ca87a30a89432520` |
| `proposals/generators-ops.json` | `a0431cf445ceeec6685b1c24dbcecdf20b1dbef32fdc74a1be28b104912ccf5b` |
| `proposals/mtc.json` | `980b6f393a431943064fc16ba15496bafdfbf0ea4d660510711993af4c850abe` |
| `proposals/performance.json` | `5b9f7e63693d83b789cc948cdeabc6a7b94427dedcf6529f323ce32386a439a6` |
| `proposals/pki.json` | `f499dbf15a4190da91166ff0a56f2fac0bc6ead69bc3c661d2908921e0c7a9bf` |
| `proposals/secrets-reconciliation.json` | `295f8ca705c4d80d164a03b324c0388932e7fb3585f7ce0f214d40f43e7597e3` |
| `research/future-seeds.md` | `71344ccaeb4857f83606f73d071fd1e0b15b0d699292a238fcea0fbde1d5a18c` |

`proposals/idea-seeds.json` was absent from the proposal-directory listing at
this snapshot; `future-seeds.md` links to it. This absence is itself
provisional until the freeze/re-hash pass.

## Source-truth correction — 2026-11-12 21:03 CST

I retract the prior I-6 sentence that referred to a “differently interpreted
32+20 split.” That statement was an invented draft assertion, not a conflict
in the user's assignment or governing sources. A search of the supplied source
files and `note.md` found no such `32+20` / “32 issuer, 20 direct cross-cert”
wording. Spec §24.2 lines 1425–1436 gives the exact signer matrix and directly
derives 13 certificates per partition, 52 issuer certificates and 65 CA
certificates total. Do not ask the user to approve, reconcile or disposition a
32+20 split; it is not a requirement. The remaining I-6 dependency finding is
solely that ROOT-06 must consume the frozen, source-defined edge manifest for
correct signer-root CRL ownership.

## Disposition

This audit is a draft integration handoff, not final approval of the design
tree, inventory coverage, issue DAG or proposal packets. The main actionable
items are I-1 (cycle), I-2 (dangling IDs), I-3 (unbound seed coverage), I-4
(urgency migration), I-5 (issue-level signer oracle), I-6 (graph-to-CRL
ordering), and I-7 (root certificate validity). No GitHub, queue, provider or
production operation was performed.

## CDP / subsequent-CRL follow-up — 2026-11-12 20:39 CST

The supervising parent flagged that the earlier CDP draft could freeze
certificates at an initial empty CRL. I checked the controlling source and the
current draft/packet snapshots again:

- **Source requirement confirmed:** spec §4.1 (line 179) requires each
  embedded AIA/CDP URL to remain immutable *or* be maintained as a durable
  alias for the certificate lifetime, and forbids repointing an old AIA URL
  to an unrelated generation. §4.3 (lines 193–197) requires initial empty
  signed CRLs **plus subsequent renewal/updates**, with CRLs keyed to the
  actual signing CA and number/time.
- **Current draft correction verified:** `designs/pki-issuance.md` now
  distinguishes immutable AIA certificate variants and numbered CRL archives
  from the durable per-signing-key/generation current-CDP URL (lines 23,
  152, 160, 172–181). It requires successive higher CRLs at that original
  CDP, forbids cross-generation reassignment, bounds cache freshness by
  `nextUpdate`, and disclaims universal client refresh/enforcement.
- **Issue acceptance correction verified:** current CERT-09 tests empty CRL →
  revoke → higher CRL → fetch the certificate's unchanged original CDP and
  assert the revoked serial (proposal lines 123–126). CERT-10 requires the
  higher CRL on all configured mirrors and fresh/valid bytes before stable
  route advancement (lines 129–132). CERT-11 repeats the CRL-specific
  original-URL oracle and guard-aware pointer promotion (lines 135–138).

**Disposition:** The specific “frozen initial empty CRL” blocker is closed in
the current draft snapshot. No claim is made that existing clients universally
refresh or enforce revocation; the design correctly makes that client-specific
and records it as a limitation. The stable CDP route is a mutable, guarded
head for one immutable signer/generation identity, not an immutable
certificate/CRL object and not a cross-issuer alias. Continue to test lower
CRL-number replay, stale mirror, mismatched signer/generation, excessive cache
age, and promotion failure.

**Recheck provenance (2026-11-12 20:39 CST):**
`designs/pki-issuance.md` SHA-256
`01c96f0c458d18ad4fda50851b8d99de38d5cc9e596cefad405b64765834f185`;
`proposals/pki.json` SHA-256
`199f192aaf4ea185e1b94733307cb5c96fb07bedfa6a90af4cff2db7f8a0f86d`;
source spec SHA-256
`a17559474cc82992edf374615c28547dcbbdfe051481bae41cc0bfece018e6bb`.
These are moving drafts; re-hash before final review.

The ROOT-09/ROOT-10 dependency cycle in I-1 was still present in
`proposals/pki.json` SHA-256
`199f192aaf4ea185e1b94733307cb5c96fb07bedfa6a90af4cff2db7f8a0f86d` when
read at 20:37 CST: ROOT-09 listed ROOT-10 as a prerequisite and ROOT-10 listed
ROOT-09. The CDP edits did not close that separate issue-packet cycle.
