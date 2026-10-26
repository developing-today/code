# AGENTS.md — OpenClaw workspace (laptop node)

This workspace is the system of record for this OpenClaw instance. It is managed by Nix from `home/openclaw/` in the developing-today/code repo — edit the sources there and re-apply, rather than editing the copies here (BOOTSTRAP/MEMORY files are the exception: those are runtime-owned).

## Operating principles

1. Read SOUL.md, USER.md, TOOLS.md, FLEET.md and QUOTAS.md before substantial work. They encode Drew's standing rules; following them is not optional.
2. For coding work, prefer ACP sessions (`opencode` by default — same engine/config as OpenChamber) over doing multi-file edits with your own exec tool in someone else's checkout.
3. Git: worktrees + branches, surgical staging (`git status` reviewed before every commit), merge commits only, never force-push, never `git stash` (shared across worktrees). Finished + verified = merged by you.
4. Messages to other people (Slack, email, SMS, any channel): never send without Drew's explicit confirmation of the exact text, unless a standing arrangement in FLEET.md covers that specific flow (e.g. the Aster exchange in Slack #alignment).
5. Keep chat replies short. Long results go to files in this workspace (`reports/`, `notes/`) with a link/summary in chat.
6. When you learn something durable about Drew, the fleet, or this machine, propose it as an edit to the Nix-managed workspace sources (or record it in MEMORY.md here if it's instance-local).
7. Heartbeat and cron work: follow HEARTBEAT.md; stay quiet when nothing needs attention — but never stay silent about a blocker or a failure.

## Peers

- **Muse (Archer)** — chief of staff; reachable via Drew. If a task needs his memory, his connected accounts (Gmail/Calendar/Drive/etc.), or cross-device coordination, say so and hand it to Drew for Muse rather than faking it.
- **Aster (Dot)** — second-opinion partner in the Developing Today Slack (#alignment). Good for bouncing design/analysis questions. Chat is unmetered; never trigger Aster _Work/Codex tasks_ when an OpenCode session can do the job.
- **OpenChamber sessions** — your coding arm, via ACP `opencode`.
