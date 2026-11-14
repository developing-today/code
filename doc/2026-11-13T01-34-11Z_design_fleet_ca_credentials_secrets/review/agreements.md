# Review agreement receipts

These receipts distinguish the **scope actually reviewed** from whole-program
completion. A proposal-content agreement does not approve the publisher, prove
source coverage, execute a benchmark or authorize production. Changes after a
recorded hash require an appropriate re-review.

## Performance/formats — six packets

- Author: `ses_e5e2d70eddaffeObjheAZwxxsP`.
- Independent reviewer: `ses_e5e2d404a32ffeTrAf63rrwvD7`.
- Durable full review: [`performance.md`](performance.md).
- Author closed metadata/body urgency mismatch, made SecureStore evaluation
  required while keeping adoption conditional, replaced retired consumer IDs,
  and specified the quantile estimator/confidence method and its assumptions.
- Final reviewer statement: **“AGREE — all six packets are publication-ready
  as proposals. The prior P1/P2 and original urgency/SecureStore findings remain
  closed. I found no remaining security or semantic gap in scope.”**
- No benchmark or production result was claimed. Summit independently checked
  fnox's published read-only client/daemon documentation and reviewed the
  proposal corrections; tool/combined-manifest review remains separate.

| Reviewed file | SHA-256 |
|---|---|
| `proposals/performance.json` | `c714fd3c908f4c63b7cff456e2bec601aa0343473f79afd932b60f488cde00d9` |
| `designs/performance-formats.md` | `3584bef7ca20ccd6e771580dcf7f77ad4e96691269df11b8c3c32c4c9926998a` |
| `research/provider-formats.md` | `e4cc446a170f5839bd210e7ea3423d835930fc6921de4a11ad9fe79abef89f31` |
| `review/performance.md` | `3a0766a22753beb90ba64bfe660e9b72aeb12d0a536cdb77100db902e7451d5d` |

## Future infrastructure/MTC — nineteen packets

- Author: `ses_e5e2d70339fffe4IMQPWYaRX3l`.
- Independent reviewer: `ses_e5e2d2d6875ffeSO6QPzuXFp3U`.
- Summit read the reviewer's actual session output, not only the author's
  summary. First review covered all thirteen FUT and six MTC packets and their
  research/design files. It found one medium issue: FUT-08's hard dependencies
  were optional context, not required completed interfaces.
- After the author removed those hard edges, the same reviewer rechecked:
  **“Closed. … FUT-08 has `depends_on: []`; its body says FUT-03/FUT-06 are
  optional context, not prerequisites … Residual issue: none found in the two
  requested files.”** The author explicitly agreed.
- The initial full review found no other issues in scope, including FUT-13's
  ROOT-09/ROOT-10/SEC-02 references and no hard dependencies, lower-is-sooner
  urgency, MTC size/experiment qualifications and the OCI/R2 boundaries.
- Summit separately fetched the actual Cloudflare Azul repository, Cloudflare
  September 2026 report, draft-08 §6.5 and Oracle Free Tier page. This is
  documentation evidence, not a deployment, customer-plan or performance test.

| Reviewed file | SHA-256 |
|---|---|
| `proposals/future-infrastructure.json` | `b3936df553a411f9737462f827163d077e2a464aec0d937dbb05e1e4d24d7321` |
| `proposals/mtc.json` | `0d0e43d9828752ae8bee1a54130cc9cfd3b0e160426853f43f0d489a954b217f` |
| `designs/federation-infrastructure.md` | `b9c19dd733e18dba81c96f3e8b68c0c0e943fe8480835d9754ff182b612ae358` |
| `research/future-infrastructure.md` | `be958ef78f7e6cf8eb84e938e554724f6df5830f4268e92bbb7d0d03c60171cc` |
| `research/mtc.md` | `59ac6877b8fd1ab8f35d4c393fbad272cd3bc6bd7980d91c50485cfc2e1321d1` |

## Per-item baseline versus scope watermark — limited-scope agreement

- Author: `ses_e5e2d7126a8ffew859l00WyC0U`.
- Independent reviewer: `ses_e5e2d346d27ffeifGwyr0MCCAm`.
- Summit read the actual review and re-review messages. The initial reviewer
  found one contradictory checkpoint sentence; the author corrected it.
- Same reviewer: **“Agree. The prior blocker is closed … REC-01, REC-04, and
  REC-07 consistently preserve that distinction, including outbox lag. The
  100-item restart case is sound: 99 verified components remain committed,
  item 100 retains its old base, and the global watermark remains unchanged.
  No remaining concrete blocker found.”**
- Scope: that checkpoint contract and its three packets, **not** independent
  approval of the entire secrets/reconciliation domain. A later integration
  review still covers the full thirteen packets.

| Reviewed file | SHA-256 at Summit readback |
|---|---|
| `proposals/secrets-reconciliation.json` | `e3c944969c9fec04614610da262ccf8d3b7ef43786f698fe508f0b37b6d03ca3` |
| `designs/reconciliation.md` | `999613d7d2aca1bdd3ca098a308a01c43738ee1822fc0a0205a7fec5d8317e27` |

## Remaining gates

Full canonical source/semantic coverage, exact proposal-to-source mapping,
the final dependency graph and publication tooling have separate independent
reviews. None is implied by the agreements above. All implementation issues
remain future work; no approval is a certificate of implementation completion.
