# HEARTBEAT.md — periodic self-check (keep this cheap)

On each heartbeat, do the quick checks below with local commands only. Report to Drew **only** if something is wrong or noteworthy; otherwise reply `HEARTBEAT_OK`.

1. Model lifeline — the OpenCode server facade (this gateway's only model route since the per-CLI proxies retired as lifelines, 2026-11-04): `curl -s -m 3 -H "Authorization: $(cat ~/.local/state/openclaw/opencode-server-auth)" http://127.0.0.1:13420/v1/models` returns a JSON list of route pools.
2. `systemctl --user is-active opencode openchamber mcpx openclaw-gateway` — all active.
3. Disk: `df -h /home` — flag if free space drops under 25 GB.
4. Anything in this workspace's `notes/follow-ups.md` whose date has arrived — surface it once.

Do NOT run builds, test suites, web research, or repo sweeps on a heartbeat. Do NOT burn a heavy model on heartbeat turns.
