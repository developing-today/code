# Cloudflare credential architecture — scoped tokens by purpose + Bitwarden backup

Living document · Status: shipped (2026-10-07) · Register entry C2 · Tracking epic: developing-today/code#8805

## Purpose

- Replace one broad Cloudflare credential with purpose-scoped credentials.
- Back up all on-disk secrets into Bitwarden so no secret lives only on disk.
- Keep the most privileged credential out of Worker secrets and runtime env.

First asked 2026-10-07:
- "put all secrets on the disk in there"
- "generate creds for a different purpose that just have access to cf Ai gateway
  and workers Ai for inference, read, etc"

## What landed (features, and how each works)

- Account-wide admin token:
  - Stored in Bitwarden.
  - Never placed in Worker secrets or environment.
- Day-to-day token:
  - Minted 2026-10-07.
  - Account-scoped.
  - Permissions: Workers Scripts / KV / R2 / D1 / Queues read-write, plus Tail read.
  - Intended for routine Cloudflare work instead of the admin token.
- Dedicated R2 token for nix-cache:
  - Bucket-scoped to the `nix-cache-nars` bucket only.
  - Object read-write on that bucket only.
  - Swapped into the nix-cache Worker and re-verified after the swap.
- Inference-only token:
  - Minted 2026-10-07.
  - Permissions: AI Gateway Read+Run, Workers AI Read+Write.
  - Live-tested — embeddings returned.
  - Serves the AI Gateway `opencode-gateway` (see C4).
- Bitwarden backup:
  - 5 on-disk secrets backed up to Bitwarden.
  - Each backup round-trip sha256-verified.
  - Project: Bitwarden project "developing today".
  - Items named in the register include:
    `cloudflare.api-token-account-wide`, `cloudflare.api-token-day-to-day`,
    `cloudflare.r2-worker-*`, `cloudflare.api-token-ai-inference`,
    `nix-cache.*`, `cachix.*`, and others not itemised in the entry.
- `UPLOAD_SECRET` rotation:
  - Rotated consistently across Worker, local file, and Bitwarden.
- Backup tooling:
  - Idempotent script: `~/tools/bws-backup-secrets.sh`.

## Rough architecture

- Bitwarden (project "developing today") is the backup / source store.
- Cloudflare tokens are split by purpose:
  - admin (stored only, not deployed)
  - day-to-day (broad routine account work, still scoped)
  - R2 bucket-scoped (single Worker, single bucket)
  - inference-only (AI Gateway + Workers AI)
- Workers receive only the token their purpose requires.

## Status

- Design status in register: built-into-running-system.
- Implementation status in register: shipped.
- Tracking: NEVER-ISSUED — searched "bitwarden secrets backup" in
  developing-today/code, no matching issue.
- Triage: (b) DONE-WITHOUT-ISSUE.

## Next steps — immediate / near-term / long-term

- Immediate: none recorded as a build step.
- Near-term: resolve the open plugin question below if it recurs.
- Long-term: not recorded.

## Open follow-ups

- Plugin question left unanswered in the record (2026-10-07 04:45):
  Drew asked "is that cfat key cloudfare workers Ai? I had a plugin but
  it's not accepting that token".
  - Answered at the time: the cfat key is full-admin; the inference key is
    the one to use.
  - Still unknown: which plugin, and what error it showed.
- Broader Bitwarden sharing question (2026-10-06) — Organizations /
  Collections, Secrets Manager machine accounts — was answered, but no
  decision is recorded.

## Future ideas not yet specced

- Not recorded in the register entry.

## Links

- Register entry: C2 in `~/workspace/design-audit-60d/REGISTER.md`
- Related register entries: C1 (nix-cache), C3 (credential receiver), C4 (AI Gateway)
- Tracking epic: developing-today/code#8805
- Design artifact: Bitwarden project "developing today" items listed above;
  script `~/tools/bws-backup-secrets.sh`
