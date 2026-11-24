# The pipes into Cloudflare and externally

How does anything outside the house reach the lab — and how does
the lab reach out? This chapter collects the pipes: Cloudflare
Tunnels inbound, the Tailscale mesh, and the estate's other
recorded transports. Status: **proven and in use**. It is a
member of the between-lab-and-fleet trio, with
[DNS management](dns-management.md) — the names these pipes are
reached by — and
[Automation and automated scaling](automation-and-scaling.md).
Read by the platform census of 2026-11-23 (landed as
`doc/2026-11-24T01-20-46Z_research_code-book-census-platform/`,
PR [#13959](https://github.com/developing-today/code/pull/13959)).

## Cloudflare Tunnels — the inbound pipes

- **`lab.security.cab`** — the lab's first tunnel, **live since
  2026-11-06**
  ([#6455](https://github.com/developing-today/code/issues/6455),
  closed). Its connector runs on the laptop (`amd`): a NixOS
  module at `nixos/services/lab-tunnel/`, token-file based with
  the token never in the Nix store (#8933), a watchdog and an
  edge status service on localhost:8790, and the DNS CNAME
  recorded in the module. Part 1 tells the same tunnel from the
  cluster side in
  [The cluster programme](../cluster/the-cluster-programme.md) —
  the one live network artefact of the lab programme, running on
  the laptop rather than on any cluster machine.
- **`sandbox-link.security.cab`** —
  [#10356](https://github.com/developing-today/code/issues/10356):
  the sandbox-to-laptop-agent control path rides this tunnel as
  its **primary transport**, with Tailscale as fallback (the host
  configuration's comment of record; see
  [The laptop NixOS config](the-laptop-nixos-config.md)).
- **A reusable module** exists for the generic case:
  `nixos/services/cloudflared/`, holding tunnel credentials in
  sops. Its header documents the one-time
  `cloudflared tunnel login` and tunnel-create ceremony honestly,
  as a manual step.
- **Service doors on the zone:** `dispatch.security.cab` (the PM
  dispatch gateway that Actions workflows post to — an
  outside-probe workflow, `dispatch-health-probe.yml`, watches
  its public path and files an issue on failure),
  `pm.security.cab`, and `mail.security.cab`.

## Tailscale — the mesh pipe

`nixos/tailscale-autoconnect/` wires the laptop onto the tailnet
(sops-held auth key, login.tailscale.com, split DNS through
systemd-resolved — after a recorded dhcpcd/openresolv SERVFAIL
incident). The fleet session proxy rides the tailnet to the
laptop: this is the laptop-agent route every agent in the estate
uses to reach the machine.

## Other meshes in the record

- The clan zerotier instance — clan's controller and peer on its
  one defined machine; see [clan.lol and machine fleets](clan.md).
- Clan's researched transport ladder: p2p-ssh over iroh, then
  internet, wireguard, zerotier, mycelium, yggdrasil, tor.
- **iroh** is the peer-to-peer transport of the ID and fleet
  stack (ecosystem scan §7), including `n2p` — nix-copy over
  iroh, named in [The Nix cache worker and its alternatives](nix-caches.md).

## The egress direction

Design #5030 (via the lab census): tunnel or shunt bulk traffic
to cheap-egress services — R2's zero-egress property among them —
and cross the home link only with what must. Enforcement (#8408)
is open. The dual-exposure direction (Tailscale Serve paired
with a private Cloudflare Tunnel, item C9) is **drafted, not
ratified** (`doc/living/lab-integration.md`).

## Gaps

- The C9 dual-exposure draft awaits ratification (#8873).
- Egress enforcement is unbuilt.
- The pipes are documented per service, not in one network map;
  nix-topology is an unused flake input that nominally addresses
  exactly that.

## Sources

Platform census §19 and supplements S10, 2026-11-23, landed as
PR #13959.
