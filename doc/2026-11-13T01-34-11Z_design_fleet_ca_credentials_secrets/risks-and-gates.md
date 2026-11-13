# Risks, boundaries and decision gates

This register records consequences, not claims of measured security or
availability. The domain designs supply concrete APIs/tests; the issue index
binds their independently reviewable work. Risk severity is not scheduling
urgency, and neither is production authority.

| ID | Risk / affected property | Required mitigation and decidable evidence | Residual boundary / reversal trigger |
|---|---|---|---|
| R01 | An inaccessible/partial inventory becomes a deletion | Snapshot completeness binds account, grants, filters, scope, pagination and cursor generation. Generated outages, permission shrinkage and truncated pages never produce delete or advance a complete checkpoint. | A successful list call cannot prove hidden scope absence. Stop/re-census when the scope/grant fingerprint changes. |
| R02 | Concurrent external edits lost on unguarded hosted writes | Per-operation atomic-precondition/all-writer-serialization/unguarded capability; guarded default refuses unsafe overwrites. §20.5 history and in-place native-ID updates have explicit tests. | A separately approved weaker policy reports residual lost-update risk; no claim that preflight history contains an unseen edit. Revisit when real provider CAS semantics are proven. |
| R03 | Crash/timeout after remote mutation causes duplicate history or divergent baseline | Durable intent, immutable history identity, native-ID/readback reconciliation and per-item verified baseline advancement. Model-check submission-before/after-commit failures and restart sequences. | Independent vaults are a saga, not joint ACID. Preserve uncertainty; do not retry a write just because the transport failed. |
| R04 | Rotation/revocation defeated by stale cache or fallback | Scoped policy-aware single-flight, explicit maximum age/offline permission, immutable fetched-at semantics, invalidation and deny records. Fake-clock/model tests cover refresh failures, revocation and scope aliasing. | Last-read time is not proof of upstream freshness. Fresh-only classes fail closed when status cannot be verified. Root material uses a separate signer/escrow plane. |
| R05 | New adapter silently expands root copies or signing authority | Dedicated closed escrow destination set; transaction-bound signing grants with finite tuples, expiry and durable replay protection. Negative tests attempt ambient discovery, substitution and destination expansion. | Merely holding root bytes cannot be made cryptographically incapable of misuse by a compromised process. Minimize exposure and migrate active signing offline/hardware; a design is not a hardware security boundary. |
| R06 | Backups exist but cannot restore after laptop/provider/site loss | Recovery dependency graph includes recipients, MFA/auth/bootstrap, WAN and operator absence. Restore each named escrow copy with the other destinations unavailable, and restore complete/per-root archives. | Multiple providers can share an operator, laptop or recovery identity. Describe shared compromise and failure domains rather than counting copies as independent HA. |
| R07 | Cross-signing/classical trust falsely presented as PQ assurance | Separate trust-anchor sets, leaf key/protocol eligibility and all-path signature policy. Reject classical anchors/signatures/endpoint authentication in strict PQ tests. | A graph proof is not cryptographic verification, client path building or PQ key exchange. Record each client/library/version independently. |
| R08 | Library/ASN.1/backend appears to support ML-DSA but emits wrong identifiers or modes | Pin FIPS 204/RFC 9881 encodings; independent vectors, certificate/CRL parse/verify and malformed-input negatives; no pre-standard Dilithium substitution. | Mark unavailable validators or clients unsupported, not passed. Re-open backend selection when source support/security advisories change. |
| R09 | CRL/publication state rollback, broken AIA, expired revocation objects | Durable issuer-key/serial/CRL-number state; immutable public paths and staged publication; verify hash/MIME/issuer/AKI/time on every selected mirror before activation. | Revocation of a trust anchor requires trust distribution/removal policy; an ordinary CRL is not universal root distrust. Mirror/CDP availability and client revocation behavior are separate release gates. |
| R10 | Field mapping drops attachments, unknown fields or native identity | Versioned lossless canonical records and sidecars; native IDs tracked separately from logical IDs and aliases; round-trip corpus and boundary/property tests. | A provider without a field cannot silently claim native feature equivalence. Retain fidelity evidence and expose unsupported operations. |
| R11 | Public SQL/journal/equality metadata leaks secrets or low-entropy guesses | Non-secret receipts separate encrypted recovery payloads. Scoped keyed equality tokens only under reviewed policy; log/telemetry canary tests; no raw password hashes or root equality index. | Even keyed tokens reveal equality/access patterns. Metadata minimization and key-compromise analysis remain mandatory. |
| R12 | New dependencies bloat tiny binaries or add latency/operating burden | Preserve connect's existing size gate; isolate dependency closures, feature matrices and in-memory paths. Record actual cold/warm startup, RSS, latency distributions and maintenance costs before adoption. | Proposed budgets are not results. fnox/SecureStore/OpenBao are conditional choices, not new baseline dependencies. |
| R13 | Same-host lab is mislabeled HA or federation weakens authority boundaries | Explicit host/site/account/operator/quorum/seal/audit failure graph and restoration tests; dynamic leases never copied as static credentials. | No production reliability claim without measured recovery and accountable operators. Optional infrastructure can fail its experiment without blocking core Fleet. |
| R14 | Issue publisher duplicates uncertain writes or publishes unreviewed scope | Strict complete manifest validation before effects, stable markers, serial bounded sanctioned `fleet gh`, durable intent/receipts, native parent/dependency readback and same-pass queue coverage. Check #9155 is open before writes. | A marker, issue, relation, queue task and Project field are different receipts. Rate limiting and token rotation do not create unlimited quota or prove execution did not happen. |
| R15 | Automation closes design epic or starts a second owner | Recheck live issue/session ownership at publication boundaries; one original Summit; native child issues remain open until their own evidence passes. | Prior erroneous close remains an external supervision defect. Stop publication if the epic closes unexpectedly; record it, do not fight unknown automation. |

## Approval boundaries (not granted by this document)

The following are separate future decisions with named principals/scopes,
bounded operation plans and reviewed rollback/recovery evidence:

1. Isolated live-provider benchmark grants (including separately authorized
   `bw` access). The current run uses fixtures/mocks only.
2. First production inventory/reconciliation/history writes and any explicitly
   weaker no-CAS operating policy. Routine follow-on scheduling requires its
   own validated scope and schedule-registry update.
3. Production root generation, signing, export/escrow or custody migration.
   A signing grant is per transaction; escrow approval is not signing approval.
4. Real credential or remote SSH activation/rotation, DNS or trust-store
   modifications, publication/installation and service deployment.
5. Any additional root escrow destination, including R2, and any new live
   OpenBao/Cloudflare secret service or infrastructure trust federation.

Synthetic code and tests are not blocked on those approvals when they are
provably isolated from production authority. An issue implementing a guarded
workflow can be executable while a distinct production-activation issue stays
blocked. No approval is implied by queue eligibility or an issue label.
