# FLEET.md — the intelligence fleet and the four repos

## Who is who

- **Drew** — the human. Owns every machine, account, and final decision.
- **Muse ("Archer")** — Drew's personal agent/chief of staff (Meta Muse, Power plan — an assistant subscription, _not_ a model pool; never describe sessions as drawing on it). Coordinates across Drew's accounts and devices, owns long-term memory and follow-through. First in line for anything cross-cutting.
- **Aster ("Dot")** — peer AI in the Developing Today Slack (workspace: Developing Today; channel #alignment is the Muse↔Aster room). The arrangement: the two converse freely, either may bring work or questions unprompted. Use Aster for second opinions and research bounces. Log substantive Aster requests. Never spend Aster Work/Codex tasks when an OpenCode session can do it — chat is unmetered, tasks are not.
- **OpenChamber** — Drew's orchestration GUI over OpenCode sessions on this laptop. "OpenChamber first" for complicated work: sessions, its tooling, its orchestration.
- **OpenClaw (you)** — the local, always-on node: chat channels, monitoring, quick questions, and delegating coding to OpenCode via ACP.

## The four repos (kept healthy by continuous sweeps)

1. `developing-today/code` — the monorepo: laptop NixOS flake, provider proxies (`bin/*-proxy.mjs`), mcpx, the id project, PM stack. **All new code goes here.**
2. `developing-today-forks/opencode` — OpenCode fork (model routing / route pools work).
3. `developing-today-forks/openchamber` — OpenChamber.
4. `dezren39/nix` — nix config/packages (incl. mcpx packaging).

Sweep discipline: fix one issue per pass when you can; open PRs are first-class work items; triage and prioritize the backlog otherwise. Labels: `user/owned`, `user/review` (Muse triages first), `user/escalated`; lifecycle labels `work/*`, `effort/*`, `queue/blocked`, `meta/*` with the agent-note:v1 standard.

Agent notes — where they live: before filing a note, apply the one-step placement test from the agent-note placement design (#6022, §1): **"Would this note still be true, and still be looked for, if the repo it is about were deleted tomorrow?"** Yes → zettel (knowledge about the world); no → the repo it describes (knowledge about the artifact); neither yet → workspace/memory (knowledge about the current attempt). The placement decision table is §2 of the canonical design doc, `doc/2026-11-10T20-00-00Z_design_agent_note_placement.md` in developing-today/code — that doc is the table's single home; point to it, never copy the table into a second doc that can drift.

Standing repo rules: merge commits only · never force-push main · **never touch the `vertexinc` org** (info about it comes via "Karen") · finished work gets merged, not parked · ROADMAP files keep Now/Next only (GitHub is the record of done work).

## Other standing context

- **mcpx** — Drew's MCP gateway: one daemon fronts the fleet's MCP servers (codedb, fff, chrome-devtools, context7, desktop-commander, …). This gateway also has direct MCP wiring to the most useful of those (see openclaw.json `mcp.servers`).
- **Notes system** — Emanote Zettelkasten: `dezren39/zettel` + `developing-today/zettel`. Drew brain-dumps; agents explode into small interlinked notes and push.
- **Basement lab cluster** (goal, in progress) — Talos + Cozystack + TALM, Postgres (CNPG), RustFS, ~12 Threadripper machines. **Hard rule: no HashiCorp tooling.**
- **Fleet data** — Turso is the fleet's only database platform (Supabase is retired). Not OpenClaw's concern unless asked.
