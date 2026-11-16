# Tailscale — Setup Cycle

Verified 2026-10-21: `tailscale status` → **Connected**. Important: the `tailscale` on this box (`/opt/hatch/bin/tailscale`) is the **Muse platform wrapper, NOT the upstream Tailscale CLI**. Its help text says so explicitly, and upstream subcommands (`ping`, `ssh`, `serve`, `funnel`) are unsupported.

## Download / Install
- Nothing to install — the wrapper is provided by the platform at `/opt/hatch/bin/tailscale`. There is no tailscaled to manage in the sandbox; the tunnel runs outside the container and accepts no inbound connections.

## Gather secrets
- No auth key file is used. Joining the tailnet prints a link that **Drew must approve** — joining and leaving need his approval; read-only queries (`status`, `ip`) do not.

## Configure / Use
- Subcommands that exist: `up` (join, prints approval link), `down` (disconnect and erase this device's identity — afterwards delete the device in the Tailscale admin console / control server), `status` (enrollment state + known devices), `ip` (this device's tailnet addresses).
- **"Connected" means enrolled, not verified.** It does not prove any machine or service is reachable. Read `~/docs/devices/tailscale.md` before accessing a machine (per the wrapper's own help).
- Ordinary networking does not reach the tailnet. Reuse the runtime HTTP proxy with only its port changed to **3130**:
  ```bash
  tunnel_proxy="${HTTPS_PROXY%:*}:3130"
  curl --proxy "$tunnel_proxy" --fail-with-body "http://<tailnet-ip>:<port>/<path>"
  ```
- TCP only: direct curl/SSH, ping, ICMP, and UDP do **not** test this path. Other TCP clients (including SSH) must use the same HTTP proxy with CONNECT support. Use tailnet IPs from `tailscale status`, or names when MagicDNS is enabled — the proxy honours the tailnet's DNS servers, search domains, and split DNS.
- Approvals: the first tunnel request to an address needs its own approval — wait for Drew instead of timing out/retrying while an approval is pending. One approval covers other ports/TCP services on that *exact* name or IP for the chosen permission lifetime; another name/IP for the same machine needs its own.

## Test
```bash
tailscale status   # expected: "Connected" plus known devices listed
tailscale ip       # expected: this device's tailnet address(es)
```

## Known quirks & what has NOT worked
- `tailscale version` and `tailscale --version` both fail ("unrecognized subcommand" / "unexpected argument") — this is a wrapper, so don't script against upstream CLI behaviour, flags, or output formats.
- Never conclude a tailnet service is down from a direct (non-proxy) connection attempt — that path cannot reach the tailnet at all here.

## Keep a real CLI + API keys available — Drew's decision, 2026-10-21
Tailscale is otherwise handled outside this sandbox, but Drew wants a Tailscale CLI kept installed here anyway, just in case it becomes useful — do not remove it as "unused". The reason to keep it (and to keep Tailscale API keys filed in Bitwarden Secrets Manager) is the control-plane work the full CLI/API enables on Drew's own machines and tailnet:
- **Serve / Funnel** — expose a private local website or service publicly through the tailnet (Funnel is the public-exposure tool; Serve is tailnet-only).
- **Route changes per host** — advertise/accept subnet routes, set a host up as a **subnet router or exit-node gateway** for other devices.
- General tailnet administration (ACLs, device and route management) driven by API keys from the secrets manager rather than interactive logins.
Caveat: none of that works through this sandbox's platform wrapper today (see top of this doc — `serve`/`funnel` unsupported here). This section records intent for Drew's machines and for API-driven work, not a verified sandbox capability.
