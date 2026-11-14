# Source precedence and authority

Owner: Summit, task 15345, starter #9155. Design baseline: `34faffbff2`.
The immutable intake inputs are in
`source-snapshots/intake-34faffbff2/sources/`; `note.md` is the intake record,
not a substitute for those inputs. Upstream auto-formatting changed the
ordinary `sources/` view; both variants are preserved, as explained in
[`source-integrity.md`](source-integrity.md). The five entries in the supplied
SHA256SUMS were independently checked against the original snapshot: all pass. That manifest does
not cover the other source files or independently prove the original ZIP
hash; the ZIP's intake receipt is in `note.md`.

## Resolution rules

1. The current user assignment controls this **design** mission and its
   non-negotiable safety gates.
2. The dated addendum refines particular outcomes of the integrated v0.3
   specification; it does not discard unrelated requirements.
3. The specification, including its §33 conflict table, §47 crosswalk,
   §31 register, and Appendix A, is the remaining implementation baseline.
4. The decision review supplies proposals to validate, not current-state
   assertions. Older source-input-three and v0.2 alternatives remain in the
   inventory with their supersession recorded.
5. The coordinator handoff controls design deliverables and issue quality.
   The companion PM charter supplies shared conventions only. It does not
   enlarge this CA program into a PM redesign.

## Binding resolutions

| Question/conflict | Resolution | Source |
|---|---|---|
| Does the previous implementing-agent prompt authorize implementation now? | No. Retain its future acceptance requirements; this run designs, researches, files issues and uses synthetic proof only. | User; handoff lines 37–43; coordinator §§0,10 |
| Is ML-DSA replaceable by MTC? | No. ML-DSA-44 remains the baseline. MTC is a separately tracked, optional experiment with no baseline dependency. | User; handoff lines 53–64 |
| Four roots or one shared key? | Four independent key identities: Ed25519, P-256, RSA-4096, ML-DSA-44. Cross-certificates reuse a subject public key, not another algorithm's private key. | Spec §§0.2,3,24,33 |
| Full cross graph or asymmetric graph? | Six classical-to-other-classical plus three classical-to-ML-DSA root cross-certificates. No default ML-DSA-to-classical signatures. | Spec §§3.2,33.2 |
| Roots-only or direct issuer variants? | Reference profile uses both: Base and Internal Web, each four algorithms × two independent A/B partitions. Ordinary future issuer profiles may choose roots-only. | Spec §§24.2,33.2 |
| Reference inventory | 4 root keys + 16 issuer keys = 20 CA key pairs. 13 root certificates + 4 partitions/sets × 13 issuer variants = 65 CA certificate variants, excluding leaves. | Spec §§3,24 |
| Exact dates or relative defaults? | Reference roots: 2050-07-07T12:00:00Z. Both reference issuer sets: 2032-07-07T12:00:00Z. General policy remains 20 calendar years / 5 calendar years / normally 90 days, with the explicit reference Base test-leaf exception. | Spec §§24,33.2; Appendix A |
| RSA-2048 | Explicit, policy-gated compatibility only; never a silent long-lived-root downgrade. | User; spec §§24.1,33.2 |
| Auto-locate parent vs auto-use private root? | Discovery may resolve public inventory or identify candidates. Signing always requires an explicitly authorized signer source. Ambient vault discovery is never signing authority. | Spec PKI-046/047 as narrowed by PKI2-008; user |
| No root replication vs initial escrow? | Addendum supersedes the earlier blanket prohibition: design exactly three approved escrow destinations, not generic reconciliation. This design run performs none of the escrow operations. | Addendum §5; user |
| Root custody destination set | Encrypted laptop storage, authorized BWS project, authorized 1Password vault. R2 is research-only. New adapters do not expand this set. | Addendum §5 |
| BWS/bw/op | Three separate products, grants and adapters. Existing BWS authorization never grants bw personal-vault access. | Addendum §§1–2; spec §§19,38 |
| Daily reconciliation vs safety | Future daily reconciliation covers only the authorized shared compartment. Inaccessible/incomplete inventories do not prove deletion. Initial production writes, including zz.delete histories, require approval. | Spec §§20,36; coordinator §10 |
| Public Web name | Private trust, not public WebPKI. Public ACME/DCV/CAA/CT is a separate source and workflow. | Spec §§24.8,33.2,44 |
| AIA/OCSP | Multiple AIA entries are legal but not a compatibility guarantee. Distribute tested chains; advertise OCSP only when an actually supported service is served. | Spec §§4,24.7,33.2 |
| OpenBao reliability | A synthetic lab, not an immediate dependency. Three containers on one machine are one failure domain. Production reliability must include operations, recovery and independent sites. | Addendum §7 |

## Evidence vocabulary

**Source-verified** means the cited source or code says it; **executed** means a
recorded command actually ran; **measured** requires samples and methodology.
**Proposed** budgets and APIs are not measurements. **Unsupported** is an
explicit capability boundary, not a passed test. **Approval-blocked** retains
the desired outcome and its safe preparatory work; it does not authorize it.
Later design versions never retroactively change the V1 capability census.

The inventory and issue receipts must preserve these distinctions. A native
GitHub relationship, a queue task, an issue body and a merged implementation
are four different records; none proves the others exist.
