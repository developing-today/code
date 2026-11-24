# Cloudflare research — Workers, costs, containers and prototypes

The Cloudflare research set from `hatch-personal/workspace/cloudflare/`: the 2026-10-20 research series (workers-paid, free-tiers, paid-only, declarative-infra, multiple-plans-tos, containers-do-costs, do-practices) at the top level plus the byte-identical `docs/` copies, the `docs/ideas.md` parked-ideas note (2026-10-26), the Cloudflare project README, the magic-environment design move-note, and the two prototype READMEs (`cf-container-hello`, `lab-ops-hub`).

## Provenance

Migrated 2026-11-24 from hatch-personal (`workspace/cloudflare/`) under the DOCS-MIGRATION programme (#13877). Source Markdown files are copied byte-for-byte, preserving relative paths. This is a dated research record — it states what was present at migration time and is not rewritten later. Tracking issue: #13877.

## Contents

- `README.md` — Cloudflare project home (Workers Paid $5 + R2 base; research set and Lab Ops Hub pointers).
- `design-magic-environment.md` — move-note (2026-10-25): v1 design reviewed and rewritten as v2 at the Atlas project home.
- `research-containers-do-costs.md` — containers vs Durable Objects cost research.
- `research-declarative-infra.md` — declarative infrastructure research.
- `research-do-practices.md` — Durable Objects practices research.
- `research-free-tiers.md` — free-tier research.
- `research-multiple-plans-tos.md` — multiple-plans / ToS research.
- `research-paid-only.md` — paid-only features research.
- `research-workers-paid.md` — Workers Paid plan research.
- `docs/ideas.md` — parked future ideas (2026-10-26, none approved or scheduled).
- `docs/research-containers-do-costs.md` — byte-identical copy of the top-level file.
- `docs/research-declarative-infra.md` — byte-identical copy of the top-level file.
- `docs/research-do-practices.md` — byte-identical copy of the top-level file.
- `docs/research-free-tiers.md` — byte-identical copy of the top-level file.
- `docs/research-multiple-plans-tos.md` — byte-identical copy of the top-level file.
- `docs/research-paid-only.md` — byte-identical copy of the top-level file.
- `docs/research-workers-paid.md` — byte-identical copy of the top-level file.
- `cf-container-hello/README.md` — known-good container reference (official hello-world image, `/`, `/lite`, `/probe`, `/worker-marker` routes).
- `lab-ops-hub/README.md` — Lab Ops Hub full-stack demo (Workers + D1/R2/KV/Queues/DO/Containers/AI/Cron/Assets, $0-overage design).

The migrated source set contains 19 Markdown files. Including this front page, the record has 20 files.

Code remains in place: the `cf-container-hello` and `lab-ops-hub` code, configs and assets (src/, image/, container/, public/, test/, wrangler.jsonc, package.json, tsconfig.json, Dockerfiles, schema.sql, Nix files) stay in `hatch-personal/workspace/cloudflare/` — only the two README.md files migrate here as research-prototype documentation.
