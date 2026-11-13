# #9143 research slice B13 — OCapN

**Research scope:** `crdt-ocapn`. Research-only; attenuation is an open application-level question, not a verified OCapN feature. Evidence checked 2026-11-12.

## Evaluation criteria

Authority model; reference/handoff semantics; transport separation; attenuation evidence; fit to directional peer classes; interop maturity; signed-history/data-model coverage.

## Evidence and fit

The cited CapTP draft and implementation guide support capability-based object references and third-party handoff; the netlayers draft separates transport. The repository describes living drafts. Cited sources do not establish general attenuable references. OCapN is neither a CRDT nor a signed data-history/federation-discovery format. Any adapter would be complementary to, not a replacement for, the signed ingest gate.

## Sources

- [OCapN CapTP specification draft](https://github.com/ocapn/ocapn/blob/main/draft-specifications/CapTP%20Specification.md)
- [OCapN implementation guide](https://github.com/ocapn/ocapn/blob/main/implementation-guide/Implementation%20Guide.md)
- [OCapN netlayers draft](https://github.com/ocapn/ocapn/blob/main/draft-specifications/Netlayers.md)

## Options and disposition

1. Treat OCapN as the peer-class/signature system: reject; scope is different and draft status remains.
2. Evaluate application-level wrappers for bounded relay authority using references/handoff: open, requires an explicit proof of attenuation behavior.
3. Use explicit signed relay grants without OCapN: valid baseline.

**Conclusion:** cite only references, handoff and transport separation as established by these sources. Do not assert attenuation without additional primary evidence. No implementation in this slice.
