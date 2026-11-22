# Muse tailnet and Iroh networking

bootstrap.sh keeps the Cloudflare HTTPS/CONNECT path for Fleet and gives
explicitly named agents their own Tailscale node state. For example,
FLEET_AGENT=xaltha defaults to the Tailscale hostname muse-xaltha and the
state directory ~/.local/state/tailscale/agents/xaltha. An explicit
TS_HOSTNAME or TS_STATE still takes precedence. The old unnamed invocation
keeps its existing muse hostname and state directory.

Tailscale hostname collisions must be treated as a naming result to inspect,
not as permission to reuse another node's state. The bootstrap reports the
effective local hostname after tailscale up; its isolated node state means a
new agent cannot log in as and replace another agent's device identity.

## What counts as connected

- Cloudflare/Fleet HTTP: userspace Tailscale mode can use the local HTTP
  CONNECT proxy; Cloudflare HTTPS remains the fallback.
- Direct tailnet UDP: requires kernel TUN mode. The bootstrap reports if the
  laptop probe succeeds by direct path or through DERP. DERP is reachable,
  but is not direct.
- Iroh: id serve advertises direct UDP transports before its relay transport.
  A successful Tailscale ping does not prove an Iroh session used direct UDP,
  and a Cloudflare HTTP success does not prove Iroh connectivity. Verify a
  real Iroh peer session and its selected transport on both sides.

For each new agent, confirm its tailscale status --json self hostname is the
expected muse-<agent> name, then confirm the laptop peer probe reports a
direct route. Finally perform an Iroh operation between the laptop and the
agent and between two agents; record whether the connection used direct UDP
or an Iroh relay. If the agent is in userspace mode, direct Iroh over the
tailnet is unavailable until the sandbox has kernel TUN access. Keep the
Cloudflare HTTPS route available as the application fallback in either case.
