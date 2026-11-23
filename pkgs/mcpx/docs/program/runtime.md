# Laptop runtime and verification venues

## The active service on November 23

At 17:58–18:00 UTC on November 23, the AMD user unit `mcpx.service` was enabled, active and running. Its MainPID owned the listening Unix socket. A read-only `GET /v1/status` against that socket returned the same PID and version 0.1.0. Only the version, PID and pool name/live/tool count were projected. This procedure starts no daemon and executes no upstream tool. The unit's running executable and the user profile's CLI resolved to different Nix store outputs, both labelled 0.1.0. The corresponding source commits are unknown.

The repository declaration at `1f9bc15391ca` has 16 server names. The running unit reported the 15 names below. Atlas appears in the declaration and was absent from this unit's status. This observation narrows #8625's readiness gate. The follow-up below identifies which configuration the unit selected. Private config contents were not collected. [Declaration](https://github.com/developing-today/code/blob/1f9bc15391ca1c2efa639463a6369392456c27e5/home/common/mcpx-settings.nix) · [Status handler](https://github.com/developing-today/code/blob/1f9bc15391ca1c2efa639463a6369392456c27e5/pkgs/mcpx/internal/daemon/server.go#L994) · [Atlas acceptance](https://github.com/developing-today/code/issues/8625)

| Status measure | Observed result |
|---|---|
| Names in the active unit | antigravity, chrome-devtools, cloudrun, codebase-memory, codedb, codex-connectors, context7, deepwiki, desktop-commander, fff, fff-nix, fff-worktree, gopls-mcp-server, mcp-server-neon, sequential-thinking |
| Live instances | All 15 pools reported zero at sampling time. Idle pool state alone cannot establish an outage. |
| Tool metadata | 14 pools reported a positive tool count. mcp-server-neon reported zero. No call or readiness probe established upstream health. |
| Next Atlas acceptance step | Identify the daemon actually selected by each harness and compare its status with the declared server list before attempting any authorized per-client fixture. |

## Which configuration reached the service

After the 18:06 UTC service restart, the active unit still reported 15 configured command-backed servers. The Atlas route returned HTTP 404. The service and the default configuration resolved to the same Nix store configuration. Its symlink timestamp was 02:25:41 UTC. The source commit adding Atlas was recorded at 03:06:29 UTC. This ordering and the observed server list explain why restarting the frontend alone did not load Atlas: the selected configuration generation predates the source change. The evidence applies to the observed unit. A different daemon or harness may select a different configuration. [Declaration commit](https://github.com/developing-today/code/commit/0e0d85e6f238224e963845dffc1157ee64c7375a) · [Atlas acceptance](https://github.com/developing-today/code/issues/8625)

Once an authorized configuration activation installs the intended generation, verify the unit reads that generation and exposes the Atlas namespace. Then collect the four independent client-fixture receipts, including identity, denial, audit and source attribution. A running unit or visible namespace satisfies only the first part.

## CI and release receipts

The newest completed `mcpx-test.yml` run returned by the main-branch workflow query was a failed manual dispatch at `4518255b08f8`. Its predecessor was cancelled. The November 22 schedule also failed. A venue claim needs the specific job, commit and asserted behavior. [November 23 failed dispatch](https://github.com/developing-today/code/actions/runs/45227606720) · [Cancelled dispatch](https://github.com/developing-today/code/actions/runs/45227033087) · [November 22 scheduled run](https://github.com/developing-today/code/actions/runs/45038359692)

The release API lists three v0.1.0 archives, Linux x86-64, Linux AArch64 and Darwin AArch64, plus SHA256SUMS. It records publication at 19:14:13 UTC on October 30. This receipt covers the GitHub release channel. GHCR publication remains a distinct acceptance check. [Release](https://github.com/developing-today/code/releases/tag/mcpx%2Fv0.1.0) · [Container publication](https://github.com/developing-today/code/issues/9900)
