# Laptop port registry — host `amd` (authoritative)

One row per port claimed, reserved, or observed on the laptop.
This file is the source of truth for "who owns port N on amd":
before any service claims a new port, check here first, and add
its row in the same change that claims it.

Audit: 2026-11-05 (pm-stack overhaul, Phase A). Method: every
row's *Source of truth* is a committed repo file (path:line at
audit time) or the live unit/config named in the row; the
*Status* column was verified against a live `ss -tlnp` /
`ss -ulnp` sweep and `docker ps` on amd at ~02:00 CST
2026-11-05. Bind scopes: **loopback** = 127.0.0.1 only,
**tailnet** = 100.99.113.103 (the host's tailnet address)
and/or the MagicDNS name via `tailscale serve`.

## The Pad correction (read this first)

Pad is on **7777** and always was in committed config
(`pm-stack/pad/compose.yml`, `PAD_PORT 7777`). Reports on
2026-11-05 that "Pad is down" or "Pad is on 3131" were
**stale and wrong**. The 3131 story traces to one place: the
comment in the hand-installed `~/.config/fleet/connect-udp.toml`
on amd, which claimed 3131 was "already assigned to the pm pad
service (PM_AGG_PAD_URL http://127.0.0.1:3131)". No committed
config ever pointed the aggregator at 3131 for Pad: the
aggregator default is `PM_AGG_PAD_URL=http://127.0.0.1:7777`
(`pm-stack/aggregator/src/config.ts:206`), the deployed
aggregator env has no `PM_AGG_PAD_URL` override, and at audit
time the aggregator's own `/api/health` reported
`pad: state up, ok true, HTTP 200` — the aggregator consumes
Pad on 7777 successfully. The laptop-local comment has been
corrected in place (comment only, no behaviour change).

**3131 is retired/unassigned.** Nothing listens on it, and no
service may claim it without updating this registry — its only
history is the stale story above and the pre-2026-11-05
fleet-config default (see the CONNECT-UDP rows).

## pm-stack

| Port | Service | Bind | Source of truth | Status |
|---|---|---|---|---|
| 4870 | pm-aggregator (dashboard HTTP) | loopback + tailnet | `pm-stack/aggregator/src/config.ts` (`PM_AGG_PORT` default 4870); `nixos/services/pm-stack/default.nix` | live |
| 4871 | pm-aggregator freshness IPC (`/publish`) | loopback | `pm-stack/aggregator/src/config.ts:148` (`PM_AGG_FRESHNESS_PORT` default 4871); `nixos/services/pm-stack/default.nix:163` | live |
| 4879 | pm-dispatch (intake) | loopback + tailnet | `pm-stack/dispatch/src/config.ts:73` (`DISPATCH_PORT` default 4879); `pkgs/fleet/crates/fleet-config/src/lib.rs` `PORT_DISPATCH` | live |
| 5173 | Kaneo (Docker, `pm-stack-kaneo-1`) | loopback | `pm-stack/compose.yml:35` | live |
| 5432 | Kaneo Postgres (Docker) | container-internal only (no host bind) | `pm-stack/compose.yml`; `pm-stack/aggregator/src/topology.ts` | live (internal) |
| 6056 | ready-for-agent (UI + GraphQL) | loopback + tailnet via socket proxy | `pm-stack/aggregator/src/config.ts:209` (`PM_AGG_RFA_URL` default); `nixos/services/pm-apps/default.nix:148-166` | live |
| 7777 | Pad (Docker, `pm-pad`, v0.18.1) | loopback + tailnet | `pm-stack/pad/compose.yml:26-31` (`PAD_PORT 7777`); `nixos/services/pm-apps/default.nix:103-108` | live |
| 7801 | pm-projector (+ link registry) | loopback | `pm-stack/projector/src/config.ts:1` (`DEFAULT_PORT 7801`); `nixos/services/pm-stack/default.nix:100` | live |
| 8484 | Grist (Docker, `pm-grist`) | loopback + tailnet | `pm-stack/grist/compose.yml:30-31`; `nixos/services/pm-apps/default.nix:92-93` | live |

## Fleet (brokers, laptop-agent, CONNECT-UDP)

| Port | Service | Bind | Source of truth | Status |
|---|---|---|---|---|
| 3130 | Muse runtime egress proxy | — (not this host) | `pkgs/fleet/crates/fleet-config/src/lib.rs` `DEFAULT_EGRESS_PROXY_PORT` | external — the runtime's proxy, on the `HTTPS_PROXY` host; never claim locally |
| 3131 | — | — | retired: was `DEFAULT_CONNECT_UDP_LISTEN` before 2026-11-05 | retired / unassigned (see Pad correction) |
| 3132 | fleet-connect-udp (CONNECT-UDP endpoint, RFC 9298, #6109) — dark, no consumer | loopback (TCP listener; serves UDP-over-CONNECT-UDP) | code default: `pkgs/fleet/crates/fleet-config/src/lib.rs` `DEFAULT_CONNECT_UDP_LISTEN`; client default: `pkgs/fleet/crates/fleet-proxy/src/bin/fleet-udp-forward.rs` `DEFAULT_PROXY`; deployed unit: hand-installed `fleet-connect-udp.service` + `~/.config/fleet/connect-udp.toml` (`connect_udp_listen = "127.0.0.1:3132"`) | live |
| 4799 | laptop-agent daemon | tailnet | `pkgs/fleet/crates/fleet-config/src/lib.rs` `PORT_LAPTOP_AGENT`; hand-installed `laptop-agent.service` | live |
| 4800 | fleet secrets-broker | tailnet | `pkgs/fleet/crates/fleet-config/src/lib.rs` `PORT_SECRETS_BROKER`; `pkgs/fleet/crates/fleet-queue/src/secrets_broker.rs:191` (`SECRETS_BROKER_BIND` default) | live |
| 4801 | fleet queue-broker | tailnet | `pkgs/fleet/crates/fleet-config/src/lib.rs` `PORT_QUEUE_BROKER`; `pkgs/fleet/crates/fleet-queue/src/queue_broker.rs:591` (`QUEUE_BROKER_BIND` default) | live |
| 13420 | OpenCode server (standing) | loopback (+ tailnet HTTPS 4096 via `tailscale serve` → 13420) | `home/openclaw/default.nix:33` (`opencodeServerPort`); home-manager `opencode.service` | live |

## Browser debug ports (loopback only)

| Port | Service | Bind | Source of truth | Status |
|---|---|---|---|---|
| 19222 | Helium CDP | loopback | `nixos/environment/default.nix:533` (`--remote-debugging-port=19222`); `helium-cdp.service` | live |
| 19223 | Brave CDP | loopback | `home/common/default.nix:177` | reserved — not listening at audit |
| 19224 | Chromium CDP | loopback | `nixos/environment/default.nix:289` | reserved — not listening at audit |
| 19225 | Firefox WebDriver BiDi (`--remote-debugging-port`) | loopback | no in-repo declaration found at audit | live at audit (a transient headless test instance, profile `/tmp/ff-wrap-test`) |
| 19226 | Zen WebDriver BiDi | loopback | no in-repo declaration found at audit | reserved (per the 2026-11-05 overhaul brief) — not listening at audit |

## Provider proxies + OpenClaw + Atlas (loopback family)

Port lets live in `home/common/default.nix` (top-of-file let
block); units are home-manager user services/sockets in the
same file unless noted. Tailnet reach for 18743–18746 and
18760 is `tailscale serve` — imperative tailscaled state, not
declared in this repo (`home/common/default.nix` notes the
pattern at its atlas-search block).

| Port | Service | Bind | Source of truth | Status |
|---|---|---|---|---|
| 18742 | clef-proxy | loopback | retired in `home/common/default.nix` (2026-11-04) | retired — free |
| 18743 | agy-proxy (Antigravity) | loopback | retired 2026-10-30 (`.github/scripts/check-proxy-port-fallbacks.sh`) | retired — **stale `tailscale serve` forward still present**, pointing at a dead loopback port |
| 18744 | codex-proxy | loopback (socket-activated) + tailnet via serve | `home/common/default.nix:20` (`codexProxyPort`), unit + socket in the same file | live |
| 18745 | antigravity-ls-proxy | loopback | retired 2026-10-30 | retired — **stale `tailscale serve` forward still present** |
| 18746 | design-code-proxy | loopback | retired 2026-11-03 (`home/common/default.nix` comment block) | retired — **stale `tailscale serve` forward still present** |
| 18747 | muse-proxy | loopback | `home/common/default.nix:21` (`museProxyPort`) | live |
| 18748 | commandcode-proxy | loopback | retired 2026-11-06 (proxy-infra decommission) | retired — free |
| 18749 | claude-code-proxy | loopback | retired 2026-11-06 (proxy-infra decommission) | retired — free |
| 18750 | opencode-route-proxy | loopback | retired 2026-11-04 | retired — free |
| 18751 | antigravity-direct proxy | loopback | retired 2026-11-04 | retired — free |
| 18760 | atlas-search service | loopback + tailnet via serve | `home/common/default.nix:39` (`atlasSearchPort`); `pkgs/atlas-search/crates/atlas-search/src/server.rs:285` | live |
| 18789 | OpenClaw gateway (Control UI / WebChat) | loopback + tailnet | `home/openclaw/default.nix:316` (`port = 18789`) | live |
| 18791 | OpenClaw embedding server (llama-server) | loopback | `home/openclaw/default.nix:35` (`embeddingPort`) | live |

## Other live listeners observed at audit (not pm-stack owned)

| Port | Service | Bind | Source of truth | Status |
|---|---|---|---|---|
| 22 | sshd | all interfaces | NixOS system config | live |
| 3000 | OpenChamber | loopback + tailnet via serve | `openchamber.service` (user unit) | live |
| 3005 | `id` web serve | loopback | `nixos/services/id-mail/default.nix:79` (`port = 3005`) | live |
| 3773 | t3code (T3 Code) | loopback (+ tailnet HTTPS 8443 via its `--tailscale-serve`) | `t3code.service` (user unit); `home/common/default.nix` t3 block | live |
| 4747 | vibe-provider CLI `serve` (worktree `~/git/wt-vibe-provider`) | loopback | no assignment in this repo — observed only | live (observed-other; not pm-stack) |
| 5600 | ActivityWatch (`aw-server`) | loopback | `activitywatch.service` (user unit) | live |
| 7719 | codedb serve (this repo) | loopback | `codedb-code.service` (user unit) | live |
| 4226 | sccache server | loopback | user process | live |
| 20241 | cloudflared (pm tunnel connector local endpoint) | loopback | `pm-tunnel.service` (user unit) | live |
| 21118/21119 | RustDesk (TCP/UDP) | all interfaces | `rustdesk.service` (user unit) | live |
| 39925 | mcpx daemon | loopback | `mcpx.service` (user unit); no committed port assignment found | live (observed) |
| 443 (tailnet HTTPS) | `tailscale serve` → aggregator 4870 | tailnet | imperative tailscaled state | live |
| 4096 (tailnet HTTPS) | `tailscale serve` → OpenCode 13420 | tailnet | imperative tailscaled state | live |
| 8443 (tailnet HTTPS) | `tailscale serve` → t3code 3773 | tailnet | t3code `--tailscale-serve-port 8443` | live |

## Mismatches found at audit (2026-11-05) and their disposition

1. **fleet-config default vs deployed CONNECT-UDP port.**
   `DEFAULT_CONNECT_UDP_LISTEN` was `127.0.0.1:3131` while the
   one deployed dark unit binds 3132 from its own config.
   **Fixed** (this change): default → 3132, with the port
   history recorded at the constant. No live behaviour change:
   the running unit reads its own config file, not the default.
2. **fleet-udp-forward default proxy.** `DEFAULT_PROXY` and its
   module doc said 3131. **Fixed** (this change): → 3132.
3. **No in-repo declaration for the connect-udp unit.** The
   `fleet-connect-udp.service` unit and its config are
   hand-installed on amd; no home-manager/NixOS declaration
   exists in this repo, so a rebuild-from-repo would lose the
   unit. **Not fixed here** — recorded as a declaration gap for
   the overhaul's declaration phase (the parent recon noted a
   home-manager declaration riding the pending switch queue;
   none was found in-repo at audit time).
4. **Stale `tailscale serve` forwards for retired proxies.**
   Serve still forwards tailnet 18743, 18745, 18746 to loopback
   ports whose services were retired 2026-10-30 / 2026-11-03.
   Serve state is imperative tailscaled state, not committed
   config, so it is **not changed by this change**; removing
   those three forwards is a follow-up live action.
5. **The false Pad-on-3131 claim in the deployed connect-udp
   config comment** (see "The Pad correction"). Laptop-local
   file, not committed. **Fixed in place on amd** (comment
   only) at audit time.
6. **OpenClaw committed docs list retired proxies as current.**
   `home/openclaw/workspace/TOOLS.md`, `home/openclaw/README.md`,
   `home/openclaw/workspace/HEARTBEAT.md`, and the gateway
   prompt text at `home/openclaw/default.nix:301` still present
   18742/18743/18745/18746/18750 as live doors. **Not edited
   here** (OpenClaw's own workspace surface); this registry is
   authoritative where they disagree. Flagged for follow-up.
7. **Reviewed, not a mismatch:** the aggregator topology test
   (`pm-stack/aggregator/test/topology.test.ts`) uses
   `PM_AGG_PAD_URL: "http://127.0.0.1:3131"` purely as an
   arbitrary explicit-override example (alongside Kaneo 6000,
   Grist 9000) asserting that configured ports follow env. It
   asserts nothing about defaults; the default-ports test in
   the same file expects Pad 7777. Left unchanged.
8. **Browser BiDi gap:** 19225 (Firefox) is live with no
   committed declaration, and 19226 (Zen) is reserved with no
   committed declaration. Recorded above; declarations are a
   follow-up.
