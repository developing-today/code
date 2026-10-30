# HEARTBEAT.md — periodic self-check (keep this cheap)

On each heartbeat, do the quick checks below with local commands only. Report to Drew **only** if something is wrong or noteworthy; otherwise reply `HEARTBEAT_OK`.

1. Fleet proxies: `curl -s -m 3 http://127.0.0.1:18748/v1/models` and `...:18746/v1/models` return JSON lists. (CommandCode + Design proxies are the model's own lifelines.)
2. `systemctl --user is-active openchamber mcpx openclaw-gateway` — all active.
3. Disk: `df -h /home` — flag if free space drops under 25 GB.
4. Anything in this workspace's `notes/follow-ups.md` whose date has arrived — surface it once.

Do NOT run builds, test suites, web research, or repo sweeps on a heartbeat. Do NOT burn a heavy model on heartbeat turns.
