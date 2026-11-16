# Fleet architecture documentation package v0.3

**Date:** 2026-11-06. **Status:** research and implementation specification, not an implemented deployment. No production keys, live vault credentials, DNS records or repository code were altered to create this package.

## Start here

1. **`fleet_architecture_spec_v0_3.md`** — governing engineering design (50 numbered sections, §§0–49 and Appendix A). Integrates all three rounds of user requirements, the four-root trust discussion, the critical daily 1Password/Bitwarden synchronization model, and the new independent research.
2. **`SECRETS_BACKEND_RESEARCH_2026-11-06.md`** — provider matrix, source links, technical and economic limits, security / availability tradeoffs, and separate future research issues.
3. **`fleet_agent_handoff_prompt_v0_3.md`** — copy/paste assignment for the implementing agent. Attach the two documents above; do not use old v0.2 handoff text.

## Core decisions

- Continue authorized 1Password + Bitwarden Secrets Manager support; **Bitwarden Password Manager is a different source** requiring later access and approval.
- Use SOPS/age for NixOS/Flux/Kubernetes declarative deployment secrets and design a scoped offline/continuity strategy; benchmark real BWS/OP latency, consider fnox for access/caching; reserve OpenBao for an isolated service pilot and Cloudflare D1/R2 vault design for future dedicated security review.
- Preserve user-requested `zz.delete` history and in-place native password updates. No false deletes on inaccessible scopes; no secret plaintext in audit indices.
- For the **reference PKI run**, four independent roots, nine asymmetric root cross-certificates, A/B independent partitions for Base and Internal Web, direct eligible intermediate cross-signatures: **20 CA key pairs / 65 CA certificates** excluding leaves. The default `roots-only` mode remains available for future purpose-specific issuers.
- Distinguish compatibility clients (up to four root anchors) from PQ-only clients (ML-DSA trust enforcement). Certificate policy does not automatically enforce PQ-only rules.
- The reference dates are `2050-07-07T12:00:00Z` for roots and `2032-07-07T12:00:00Z` for Base + Internal Web. Ordinary defaults remain 20-year root, 5-year issuer, generally 90-day leaf unless a profile overrides.

## Research caveats

Provider prices, availability, Kubernetes/cryptography capabilities, API limits, and tooling versions are time sensitive; validate against the product's official documentation and test in the authorized environment before implementation. Sources are included as hyperlinks in the research dossier. The existing Fleet implementation was inspected at a point in time, not audited or modified.

## Hand-off checklist

Read the spec and the research document completely, pin the current `developing-today/code` commit, perform a repository census, produce an ADR/requirements-to-tests mapping, implement safely with test data first, and use the resulting Fleet commands for end-to-end test artifact creation. Obtain required explicit approval for irreversible credential, root-key or production infrastructure changes; do not claim completion based solely on plans or compilation.
