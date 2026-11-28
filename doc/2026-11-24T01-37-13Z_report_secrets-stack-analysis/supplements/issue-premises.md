# Issue premises this analysis composes with (name-only, no values)

_Summarised 2026-11-24 from the issue bodies of #13948, #13893, #9155, #13658 in developing-today/code._

## #13948 — secrets plane design (the design this analysis must compose with)

- Problem: sandbox secrets broker cold refresh shells out to `bws secret get` once per secret (242 spawns, ~160–220 s), clients hang instead of falling back; two brokers (sandbox + laptop) hold the same set with no sync proof; `fleet secrets` is read-only (verbs `get`, `list` only — verified from the installed binary).
- Chosen design: delta refresh from one `bws secret list` (entries carry `revisionDate`; apply only what moved; a partial list applies nothing — the #5504 lesson); broker-to-broker peer sync gated by a **keyed (HMAC) set hash** (v1 peer payload = full set; full upstream refresh only when the peer is down AND the broker restarted cacheless); client surface gains `refresh`, `get --upstream`, `get --no-cache`; `put` commits to the **primary store first**, then pushes to the peer and writes the mirror (one-store nodes escalate the dual write to the laptop).
- Primary-store recommendation inside #13948: **Bitwarden Secrets primary / 1Password mirror** (evidence: broker already authenticates headless to Bitwarden; only Bitwarden's list returns values + `revisionDate`; 1Password list is metadata-only and its service accounts carry rate limits). Open question 1 leaves ratification to Drew.
- Non-goals recorded there: namespaces/store selection is #10358; the canonical secrets model is #9158; replacing either store is out of scope.
- **Interaction with this analysis:** SOPS governs a disjoint, config-shaped set; nothing in #13948's six slices changes. The seam rule (one system of record per secret; one-way promotion file → store by rotation) is the only addition, and it is recorded on #13948 as a comment.

## #13893 — the cold-refresh problem record

- Measured incident: ~160–220 s cold refresh (242 per-secret `bws` spawns), clients hung >120 s behind a present-but-refreshing broker; proposed batching + a ~2–5 s client fallback bound. #13948 absorbs the batching as its full-refresh mechanism and keeps the client bound in Slice 1; #13893 stays open until Slice 1 lands evidence.

## #9155 — fleet CA, credentials & secrets programme (v0.3 design mill)

- Estate PKI reference: four independent roots (Ed25519, P-256, RSA, ML-DSA-44 baseline) plus the P-384 IdP CA as interim primary; root custody escrowed to exactly three destinations (encrypted laptop storage, the authorised Bitwarden project, the authorised 1Password vault) with tested restoration; SOPS/age/NixOS integration is an in-scope workstream of this programme; safety gate: design/synthetic proof only in this phase, no production CA generation or live vault writes authorised by the programme itself.
- Relevance: §1's recommendation keeps root custody exactly here and chains SPIRE _under_ it; §2's Tier A (CA material) already lives in this custody model, never in git.

## #13658 — the IdP story

- One self-hosted identity provider issuing OIDC + SAML SSO for our own services, signing keys from our own PKI, new services onboarded by configuration. Programme frame over #9077 and children (minimal provider #13409, federation doors #13411/#13412/#13414, linking #13415, first consumer #13416), with pkgs/id binding via #2852 as recorded acceptance debt.
- Relevance: §3's Keycloak verdict is measured against this — Keycloak's overlap is with this provider, not with any vault — and the provider's stated properties (own-PKI signing keys, minimal footprint) are the fit argument for keeping it.
