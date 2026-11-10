# Phase 1 — as-built (Kaneo execution board)

Status: **complete**, 2026-10-24. PR `developing-today/code#752`
(merged, `d71231c3`); full completion record in that PR's comment
`issuecomment-6383895207`.

## What runs

- Kaneo 2.47.0 + PostgreSQL 16 in Docker Compose on the Framework
  laptop (`~/pm-stack`), Kaneo bound to `127.0.0.1:5173` only.
- Public address `https://kaneo.security.cab` via a Cloudflare
  Tunnel (connector = `pm-tunnel` systemd user unit) — the GitHub
  App's webhook endpoint lives there.
- Deployment defined in the code repo: `pm-stack/` (compose,
  `deploy.sh` rendering env from SOPS, runbook).

## GitHub App

**developing.today** (slug `developing-today`, App ID 5454422),
owned by the `developing-today` org. Installed on
`developing-today/code` **only** (installation 174792676).
Permissions: Issues read/write; Pull requests, Contents, Metadata
read. Credentials filed in SOPS + Bitwarden + 1Password as
`kaneo.github-app-*`, hash-verified in each store.

## Verified end-to-end

- Project `code` connected; **105 issues imported** (0 skipped),
  19 pre-existing comments carried over.
- Four sync checks passed (issue #754 ↔ task CODE-106): issue →
  task via webhook; Kaneo comment → GitHub (authored by
  `developing-today[bot]`); GitHub comment → Kaneo; task Done →
  issue closed as completed.
- Registration closed (`DISABLE_REGISTRATION=true`). One incident:
  a re-render briefly reopened registration the same day (the old
  preserve-if-set render logic); fixed live, then fixed in the
  repo in Phase 2 — `deploy.sh` now renders closed by default with
  an explicit one-render opt-out for bootstrapping. From the open
  window, an isolated workspace named `test` (owner
  `serene-monkfish`) exists on the instance; verified it has no
  membership in Developing Today. Left in place unless the owner
  says otherwise.
