# Fleet CA, credentials and secrets — design mill

**Program:** [#9155](https://github.com/developing-today/code/issues/9155).
**Scope:** design, public-source research, synthetic evidence and granular
implementation issues. **Not a production rollout.**

The source package calls itself Fleet v0.3. The **design layers** below use
V1–V4 for a different purpose: V1 describes existing behavior, never new work
silently presented as shipped. Design, implemented code, executed synthetic
tests and authorized production behavior remain separate evidence states.

## Reading order

1. [`note.md`](note.md): original, preserved intake and source provenance.
2. [`source-precedence.md`](source-precedence.md): binding resolutions.
3. [`architecture-review.md`](architecture-review.md): Summit decisions and
   authority boundaries; [`review/architecture-challenge.md`](review/architecture-challenge.md)
   records the independent challenge and response.
4. [`evidence/census.md`](evidence/census.md): V1 code/test census, including
   corrections to the intake. In particular, `fleet secrets get` intentionally
   emits its value to stdout; logging/errors are redacted. #6767's code
   mitigation merged in PR #8491, while deployment/canary evidence is still a
   separate obligation, not authorization in this run.
5. [`designs/`](designs/): additive component designs; [`research/`](research/)
   contains primary-source findings and their limits.
6. [`inventory/README.md`](inventory/README.md): source statements, stable
   identifiers, precedence and coverage method. Physical line coverage alone
   does not establish atomic semantic coverage or implementation completion.
7. [`proposals/`](proposals/): standalone issue packets; proposal IDs are local
   planning identifiers until bound to actual numeric issue receipts.
8. [`tools/`](tools/) and [`experiments/`](experiments/): explicit, bounded
   operator tooling and synthetic models. No tool may treat the presence of
   a design or plan as production authorization.

## Components and version layers

| Existing domain starter | Component design | V2/V3 direction; V4 extension |
|---|---|---|
| #9156 | Inventory and evidence | Source → canonical requirement → design/test/issue trace; ongoing drift audits |
| #9157 | [Architecture](designs/architecture.md) | Typed library seams and feature/profile isolation; future adapters without a second CLI |
| #9158 | [Secrets](designs/secrets.md) | Stable logical identity, lossless fields, capability/grant separation; scoped federation |
| #9159 | [Reconciliation](designs/reconciliation.md) | Pure three-way planner, history-before-update saga, verified baseline and recovery; explicit multi-provider policies |
| #9160 | [Performance and formats](designs/performance-formats.md) | Reproducible benchmarks, request freshness, conditional fnox, SOPS/age integration; optional format/service investigations |
| #9162 | [Generators](designs/generators.md) | Password/native SSH/iroh APIs and safe output/adoption; further credential families stay native |
| #9163 | [Roots and custody](designs/pki-roots.md) | Exact reference graph, explicit signer transactions and closed three-destination escrow policy; offline/hardware migration |
| #9164 | [Issuance](designs/pki-issuance.md) | Issuers/leaves, revocation/publication, explicit trust kits and independent client evidence; external issuance adapters |
| #9165 | [Operations and evidence](designs/operations-evidence.md) | Strict automation, docs/help/schema generation, synthetic release evidence; separately approved rollout |
| #9166 | [Federation and infrastructure](designs/federation-infrastructure.md) | Nonblocking falsifiable labs; independent failure domains, workload identity, dynamic leases and delivery operators |
| #9167 | [MTC research](research/mtc.md) | Optional controlled-stack experiment, never a baseline dependency or replacement for ML-DSA-44 |

## Non-negotiable reference and custody policy

- Four independent root key identities, nine asymmetric root cross-certificates,
  Base plus Internal Web, independent A/B partitions: **20 CA key pairs and
  65 CA certificate variants before leaves**. Reference root expiry:
  `2050-07-07T12:00:00Z`; both issuer sets: `2032-07-07T12:00:00Z`.
- **ML-DSA-44 baseline.** RSA-2048 is policy-gated compatibility, not the
  default long-lived root. No silent algorithm substitution or inference of
  client support from an available cross-certificate.
- Root signing requires explicit transaction-bound authorization. Public
  discovery, possession of key bytes and access to escrow are not that grant.
- Initial root escrow is exactly encrypted laptop storage, authorized BWS
  project and authorized 1Password vault, with independent restoration tests.
  R2 is research-only; new providers never expand this set automatically.
- BWS, Bitwarden Password Manager (`bw`) and 1Password (`op`) are different
  products with different grants. Root material is outside ordinary
  resolver/cache/prefetch/benchmark/equality-index paths.

## Status and continuation provenance

The first run was stopped for an incorrect worktree location. The orchestrator
preserved its entire partial tree in pushed commit `bdafeecae6`, moved the
worktree to `~/git/fleet/15345-fleet-ca-buildout`, and reopened #9155 after an
erroneous automation close. The user resumed the original Summit session;
a duplicate continuation yielded without writing. The issue comments contain
the incident, ownership and safe-stop artifact table.

**This continuation is in progress.** Preserved draft files and proposal JSON
must not be mistaken for reviewed acceptance or published issues. The final
coverage audit, issue receipts/index and staged roadmap will identify precisely
what is reviewed, tested, published, unsupported or approval-blocked.

Task 15345's `summit-direct` claim/lease belongs to the supervisor. This
session records progress on the issue and branch, not by guessing queue
worker identities. Exact production identities remain a gated follow-up;
fixtures/mocks are the confirmed default and do not block this design.
