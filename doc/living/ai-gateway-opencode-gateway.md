# AI Gateway "opencode-gateway"

Living document · Status: shipped (2026-10-07) · Register entry C4 · Tracking epic: developing-today/code#8805

## Purpose

- Provide a named Cloudflare AI Gateway for the fleet's inference traffic.
- Name recorded in the register: `opencode-gateway`.
- Created by Drew himself in the Cloudflare dashboard on 2026-10-07.

Context from the register entry:

- Drew created the gateway, then asked Muse: "what's the ai gateway name BTW?"
- He stood it up for the fleet's inference traffic and confirmed the setup
  with Muse the same morning.

## What landed (features, and how each works)

- Gateway object:
  - AI Gateway named `opencode-gateway`.
  - Created 2026-10-07 04:25 CDT by Drew in the Cloudflare dashboard.
- Credential pairing:
  - The inference-scoped token from C2 serves it.
  - Token name in Bitwarden: `cloudflare.api-token-ai-inference`.
  - Permissions on that token: AI Gateway Read+Run, Workers AI Read+Write.
  - Verified the same morning (2026-10-07) that the inference token works
    with the gateway as-is.
  - The token was live-tested separately in C2 — embeddings returned.

## Rough architecture

- Cloudflare dashboard object: AI Gateway `opencode-gateway`.
- Caller credential: the purpose-scoped inference token (C2), not the
  account-wide admin token.
- No design artifact was found in the register — this is a dashboard
  object, not a code artifact.

## Status

- Design status in register: built-into-running-system.
- Implementation status in register: shipped — created by Drew 2026-10-07.
- Tracking: NEVER-ISSUED — searched "AI Gateway opencode", no matching issue.
- Triage: (b) DONE-WITHOUT-ISSUE — dashboard object, exists by Drew's own act.

## Standing rule context (from C5)

- AI Gateway is NOT a general inference path for the fleet.
- The standing rule is recorded in register entry C5
  (Grounded search plane + narrow AI Gateway exception, 2026-11-06):
  - Drew granted a NARROW exception only: AI Gateway allowed ONLY as
    transport for the grounded search plane's Web Search API traffic.
  - No inference, no other traffic under that exception.
  - Expansion needs a new ruling.
- That exception does not change this gateway's recorded purpose; it limits
  how AI Gateway may be used elsewhere in the fleet.
- Related: Ask-the-ledger (A1.14) was built retrieval-only — the register
  explicitly notes "AI Gateway never used" there.

## Next steps — immediate / near-term / long-term

- Immediate: none recorded.
- Near-term: none recorded.
- Long-term: none recorded.

## Open follow-ups

- None recorded in-slice for C4 itself.
- For the C5 exception: the grounded-search build is blocked on a billing
  choice (load ~$10 credits vs BYOK vs hold) — that follow-up belongs to C5,
  not to this gateway. See register entry C5.

## Future ideas not yet specced

- Not recorded in the register entry.

## Links

- Register entries: C4 and C5 in `~/workspace/design-audit-60d/REGISTER.md`
- Related register entries: C2 (credential architecture — the inference token),
  A1.14 (Ask-the-ledger, retrieval-only, AI Gateway never used)
- Tracking epic: developing-today/code#8805
- Design artifact: none found (dashboard object)
