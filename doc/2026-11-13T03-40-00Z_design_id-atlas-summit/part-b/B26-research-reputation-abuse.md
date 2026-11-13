# #9147 research slice B26 — reputation-abuse controls

**Research scope:** `reputation-abuse`. Research-only; do not implement score or protocol changes in this slice. Evidence checked 2026-11-12.

## Failure modes and controls

| Abuse | Default design control | Residual limitation |
|---|---|---|
| Self-promotion / ballot stuffing | no self-rating weight; deduplicate evidence IDs; per-identity caps | Sybil identity cost is unresolved |
| Retaliation / smear | claims are allegations, not verdicts; signed equal-auth right of reply; no unilateral score change | local policy can still be biased |
| Spam / flooding | rate and byte caps; moderation and privacy-redacted commitments | operators choose caps |
| Whitewashing | retain local identity continuity evidence and decay carefully | open federation cannot guarantee identity continuity |
| Collusion | require independently verifiable scoped evidence; local weighting | colluding evidence producers can still deceive; no global oracle |

## Evidence and alternatives

libp2p Gossipsub provides local scoring/threshold precedent, not globally fair or portable truth. RFC 9162 and RFC 9942 support verifiable log/VDS statements at issuance/inclusion, not the truth of a claim or attribution to a named party without key binding. Compare (A) global scalar authority, rejected; (B) local evidence-weighted policy, default; (C) no automated rating until review, valid conservative deployment mode.

## Sources

- [libp2p Gossipsub v1.1](https://github.com/libp2p/specs/blob/master/pubsub/gossipsub/gossipsub-v1.1.md)
- [RFC 9162](https://www.rfc-editor.org/rfc/rfc9162.html)
- [RFC 9942](https://www.rfc-editor.org/rfc/rfc9942.html)

**Conclusion:** no public allegation alone changes a rating. Any future automation must attribute evidence correctly, preserve reply/privacy/appeal, and expose local weighting; no first-party privilege and no collusion-proof claim.
