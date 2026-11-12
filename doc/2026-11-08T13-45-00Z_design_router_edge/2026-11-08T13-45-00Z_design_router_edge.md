# Design: new-router edge — WAN-on-a-VLAN, dual Mono Gateways, VRRP

Issue: #6649 (investigation; design-first, no hardware moves yet). Related:
#1280 (current Deco edge and the IPv6 uplink stall), #6073–#6075 and #6077
(lab switching and cluster interconnect), #6066 (home-lab epic).

This record promotes the desk research of 2026-11-06 (originally
`ROUTER-RESEARCH.md`, a laptop burn-folder artifact) into the repository, adds
the two sections that research did not carry — IPv6 and the Deco afterlife —
and closes with the decisions that are Drew's to make, each with options and a
recommendation.

**It also contradicts a live document.** `doc/network-topology.md` describes the
Mono gateways bonding both SFP+ ports (`bond-sfp`, balance-tlb) and VLANs on
that bond. The research below finds that shape costs an order of magnitude of
throughput on this hardware. The design here supersedes that part of the
topology document; see "Reconciliation with `doc/network-topology.md`".

## Intent

Drew's stated concept (2026-11-06, in his words): the ISP's single Ethernet
handoff lands on a **switch VLAN** instead of a router port; both SFP+ ports of
each router reach the switch; WAN and LAN ride as VLANs; both routers are wired
identically and run VRRP; and the **primary reason is not VRRP — it is not
wasting one of the two SFP+ ports on the Internet link.** Failover is the bonus.

The design below keeps that intent (the Internet link consumes no dedicated
router port; both routers see every VLAN) and changes only the mechanism the
research shows to be harmful: the LACP bond.

## Hardware

The three routers are **Mono Gateway Development Kits** (`mono.si`, Tomaž Zaman
and team): NXP Layerscape LS1046A, 4× Cortex-A72 @ 1.6 GHz, 8 GB ECC DDR4,
2× 10G SFP+, 3× 1GbE RJ45, DPAA/FMan hardware offload, OpenWrt preloaded.
Inventory entry: `doc/inventory/routing-and-switching.md` ("Mono Gateway Router
(3x)"). The ONT is a Calix GP1101X (10G XGS-PON) on Brightspeed fiber.

This satisfies #6649's first acceptance criterion: the models are identified
from inventory and recorded.

## The finding that shapes everything: the LACP offload trap

The Mono's speed comes from DPAA hardware offload via the `ASK` flowtable. That
offload **declines upper devices**: in `we-are-mono/ASK`
(`docs/flowtable/vlan.md`) the flowtable walk states that a `bond`, a `MACVLAN`,
or a second session beneath a tag is "declined by the walk itself".

- 802.1Q VLANs **directly on a physical port** (`eth3.100`, `eth3.10`) are
  hardware-offloaded at line rate — 17+ Gbps L2/L3/NAT at under 2% CPU.
- VLANs **on an LACP bond** (`bond0.100`) fall back to the Linux CPU software
  path. On this 1.6 GHz quad Cortex-A72 that is roughly **1.5–2.5 Gbps**.

So bonding both SFP+ ports to "avoid wasting a 10G port" reduces total routing
throughput by about an order of magnitude. The port is not wasted if it is put
to a better use (see D2).

## The design

### WAN-on-a-VLAN, one trunk per router

Each Mono presents **one 10G 802.1Q trunk** to the switch on a single SFP+ port
(`eth3`). The ISP handoff lands on a switch access port in a dedicated WAN VLAN.
WAN and every LAN ride as tagged VLANs on that trunk, with sub-interfaces
directly on the physical port so offload applies:

```
  Brightspeed fiber
        │
   Calix GP1101X ONT  (transparent bridge)
        │  single RJ45
        ▼
  ┌──────────────────────────────────────────────────────────────┐
  │  Core switch                                                 │
  │                                                              │
  │  P10  access  PVID=100  ── ISP handoff (WAN-Fiber)           │
  │  P11  access  PVID=101  ── Starlink (later)                  │
  │  P12  access  PVID=102  ── cellular hotspot (later)          │
  │                                                              │
  │  P4   trunk   tagged 10,20,30,100  ◄── Mono A  eth3          │
  │  P5   trunk   tagged 10,20,30,100  ◄── Mono B  eth3          │
  │  P6   trunk   tagged 10,20,30      ◄── Mono C  eth3 (spare)  │
  └──────────────────────────────────────────────────────────────┘
```

Both routers trunk the same VLAN set, so either can serve the WAN and every
LAN at any moment. This is the "identical wiring" requirement, achieved without
a bond.

### VLAN plan

| VLAN | Name               | MTU                           | Notes                                    |
| ---- | ------------------ | ----------------------------- | ---------------------------------------- |
| 10   | LAN (home)         | 1500                          | Unchanged from `doc/network-topology.md` |
| 20   | Transit            | 1500                          | Router-to-router mesh, always tagged     |
| 30   | Internal / cluster | 9216                          | DRBD/LINSTOR jumbo, unchanged            |
| 100  | WAN-Fiber          | 1500 (1508 if baby-jumbo, D5) | ISP handoff                              |
| 101  | WAN-Starlink       | 1500                          | Reserved                                 |
| 102  | WAN-Hotspot        | 1500                          | Reserved                                 |

### VRRP

Both routers run `keepalived` VRRP on every VLAN they serve. On the LAN VLANs
the pair shares a virtual IP that is the default gateway (as `10.0.200.1` is
today for VLAN 30). On the WAN VLAN the pair shares the **virtual MAC**
(`00:00:5E:00:01:VRID`) so the ISP sees one client, not two.

**How the shared MAC is implemented matters (Codex review, PR #7712).**
The stock way to present a VRRP virtual MAC with keepalived is
`use_vmac`, which puts the virtual address on a **MACVLAN** interface —
and the ASK VLAN/flowtable documentation explicitly rejects MACVLAN
paths, so that shape silently drops the WAN path out of hardware
offload into software forwarding: the same offload loss this design
removes by abandoning the bond (D1), reintroduced through the back
door. So: **no `use_vmac`, no MACVLAN on any offload-carrying VLAN.**
Instead, both routers carry the _same cloned MAC_ on the plain WAN
VLAN interface; only the VRRP master brings the WAN address and the
DHCP client up, and the backup keeps its WAN interface address-less
with its DHCP client disabled. On promotion the backup enables the
interface and sends a gratuitous ARP; the ISP sees the one cloned MAC
(now on the other switch port, which relearns it from the GARP) and
the lease survives. The split-brain interlock is VRRP itself plus the
`eth4` peer-check (D2): a router that can still see its peer as master
must never enable its WAN MAC. At install time, verify from the ASK
flowtable counters that the WAN VLAN path is actually offloaded; if a
future ASK version refuses even this shape, the accepted fallback is
software forwarding on the WAN VLAN **only** — WAN rates are
ISP-bound (~2G today), inside the LS1046A's software-forwarding
budget — never a silent MACVLAN.

Priority and tracking are set so that a router which has lost its trunk
demotes itself and the peer takes over. The failover ladder is
A(10G) → B(10G) → A(1G RJ45 fallback) → B(1G), mirroring the existing
topology document's intent, but with the trunk as the primary path.

### The second SFP+ port

With the trunk on `eth3`, `eth4` is free. It is **not** wasted: it becomes a
direct 10G DAC link between the two routers (D2), carrying `conntrackd` state
synchronisation and a VRRP peer-check. That converts the VRRP pair from
"two independent routers that agree on a VIP" into "two routers that can hand
over live TCP/UDP state", and it removes the switch from the split-brain path.

### Multi-WAN

Every WAN lands on its own switch VLAN (100/101/102) and is trunked to both
routers, so a router failure never strands a secondary WAN. `mwan3` (or
OPNsense multi-WAN) sets policy: fiber first, Starlink second, hotspot last,
with ICMP health probes per WAN and VRRP priority tracking that demotes a
router which has lost all its WANs.

## IPv6 — and the #1280 stall

Issue #1280 records the live symptom: IPv6 connections to Cloudflare-fronted
hosts stall at zero bytes on payload-sized transfers while IPv4 is healthy;
`curl -6` of a 4 MB tarball gets 0 bytes after ~22 s. The shape — handshake
succeeds, payload dies — is the classic **PMTUD black hole**: large IPv6
packets are dropped and the ICMPv6 "Packet Too Big" never arrives, so the
sender keeps retrying the same size.

The current Deco edge is the prime suspect: #1280's own note says the Deco
advertises an **IPv6 default route with no WAN6 path** — i.e. it hands out an
IPv6 default that leads nowhere, which produces exactly this stall.

What this design does about it:

1. **The new edge terminates IPv6 natively.** The Mono runs OpenWrt with full
   IPv6 (DHCPv6-PD, SLAAC, RA, NPTv6 per inventory). Prefix delegation from
   Brightspeed is handled on the router, not by the Deco.
2. **MTU is correct end to end.** If the handoff is PPPoE (VLAN 201) **and
   RFC 4638 is negotiated** — both endpoints must indicate PPP-Max-Payload
   support during PPPoE discovery; a larger local MTU alone cannot create
   a 1500-byte path (Codex review, PR #7712) — the WAN VLAN is set to
   baby-jumbo (1508/1512) so a full 1500-byte inner MTU survives PPPoE's
   8-byte header, verified end to end (a 1500-byte do-not-fragment probe
   across the WAN) before it is relied on. If RFC 4638 support is absent
   or the probe fails, the PPP MTU stays **1492**, RAs advertise 1492,
   and the PMTUD trap is instead closed by item 3 (unfiltered ICMPv6
   Packet Too Big) plus MSS clamping on the WAN — the 1492 path is a
   designed outcome, not a degraded accident (D5).
3. **ICMPv6 is not filtered.** The edge must pass ICMPv6 type 2 (Packet Too
   Big) unfiltered in both directions; that is what makes PMTUD work at all.
4. **RA MTU matches the real path.** Router advertisements carry the true MTU
   so hosts do not start above the path limit.

This is the design-level answer to #1280's "what fixed means": with the new
edge, `curl -6` should sustain multi-MB downloads at IPv4-comparable speed, and
the temporary `getaddrinfo` IPv4-preference workaround can come out. Whether it
actually does is a verification step after cutover, not a claim made here.

Open question carried forward: whether Brightspeed here is DHCP/IPoE or PPPoE
(D5) determines which of the above applies; both paths are designed for.

## The Deco's afterlife

The Deco BE11000 is the current edge and the source of the #1280 IPv6 stall.
Three options, and the recommendation:

- **Retire it.** Simplest and removes the suspect entirely. Costs the Wi-Fi
  mesh coverage the Decos provide.
- **Demote to AP-only mesh behind the new edge.** The Decos stop routing, stop
  holding the WAN, and stop advertising IPv6 defaults; they become bridged
  access points on the LAN VLAN. Keeps Wi-Fi coverage, removes the routing and
  IPv6 defects, and is reversible.
- **Keep it as a fallback edge.** Rejected: it keeps the #1280 defect live and
  creates a second thing that can silently take over the WAN.

**Recommendation: demote to AP-only mesh (D4).** It preserves coverage, kills
the IPv6 defect at its source, and is a configuration change rather than a
purchase. It also makes the cutover gradual: the new edge takes the WAN first,
the Decos move to AP mode second.

## Reconciliation with `doc/network-topology.md`

`doc/network-topology.md` currently specifies `bond-sfp` (SFP+0 + SFP+1,
balance-tlb) on all three Monos, with VLANs on the bond. On the Mono that is
the offload trap. This record supersedes the bonding shape for the **edge**
routers; the internal-gateway role (Mono 2/3 serving VLAN 30) should be
re-examined the same way when that work is picked up, because it has the same
hardware and the same trap.

The topology document's VLAN plan, MTU rules, traffic flows and sysctls remain
valid and are reused here. A follow-up should edit `doc/network-topology.md` to
point at this record rather than carry two conflicting bonding descriptions;
that edit is deliberately **not** made in this PR, which is a design record and
should not silently rewrite a maintained document.

## Decisions for Drew

Each is a real choice with a recommendation. None is made here.

### D1 — Bonded SFP+ or single-trunk router-on-a-stick?

- **Options:** (a) LACP bond of both SFP+ with VLANs on the bond — Drew's
  original shape; (b) single 10G trunk on one SFP+, second port repurposed.
- **(a)** keeps both ports on the switch but drops routing to ~1.5–2.5 Gbps
  because the offload declines bonds. **(b)** keeps full 10G offload and frees
  the second port for state sync.
- **Recommendation: (b).** It preserves the actual goal (the Internet link
  costs no dedicated port) while keeping line rate.

### D2 — What does the freed second SFP+ port do?

- **Options:** (a) direct 10G DAC to the peer router for `conntrackd` + VRRP
  peer-check; (b) second trunk to a second switch (needs MLAG); (c) leave it
  for a future 10G segment.
- **Recommendation: (a).** It is the cheapest way to get stateful failover and
  a split-brain-free VRRP pair, and it needs one DAC cable.

### D3 — One switch or two (does the switch SPOF get removed)?

- **Options:** (a) one switch, accept it as the SPOF; (b) two switches in
  MLAG/vLAG with each router dual-homed.
- **(a)** moves the SPOF from router to switch — the trade Drew already
  accepted. **(b)** removes it but needs MLAG-capable switches and more cabling;
  the inventory has IBM/Lenovo G8264 units that do vLAG (see #6073–#6075).
- **Recommendation: (a) for v1, (b) as a later step.** The ISP ONT cable and
  the fiber are external SPOFs anyway; a single core switch is the standard
  home/lab trade and keeps the first cutover small.

### D4 — Deco afterlife?

- **Options:** retire; demote to AP-only mesh; keep as fallback edge.
- **Recommendation: demote to AP-only mesh.** See "The Deco's afterlife".

### D5 — Brightspeed handoff: DHCP/IPoE or PPPoE?

- This is a fact to determine, not a preference, but it is Drew's to confirm
  (it depends on his service). **DHCP/IPoE** gives near-hitless VRRP failover
  with a shared virtual MAC. **PPPoE** cannot be shared: failover means a new
  session, a 60–120 s BNG keepalive timeout, and usually a new public IP, so
  every established connection drops.
- **Recommendation:** confirm the protocol before the cutover. If PPPoE, plan
  for disruptive failover and lean harder on D2 (state sync) and on a static
  IP block if the ISP offers one. Confirming PPPoE also means confirming
  **RFC 4638** (PPP-Max-Payload) support on the BNG: with it, baby-jumbo
  gives a true 1500-byte inner MTU; without it, the design runs 1492 +
  PMTUD/MSS clamping as specified in the IPv6 section — do not raise the
  WAN MTU on the assumption the far end will take it.

### D6 — Third Mono's role?

- **Options:** cold spare; lab/test segment; third VRRP node.
- **Recommendation: bench test / pre-production node (cold spare).** Use it to
  validate OpenWrt/ASK updates before they reach the live pair.

## Verification after cutover

- `curl -6` multi-MB downloads from Cloudflare hosts at IPv4-comparable speed,
  repeatedly — then remove the `getaddrinfo` IPv4-preference workaround (#1280).
- VRRP failover by pulling a router's power: gateway reachable again within the
  keepalived interval; with D2, established TCP survives on DHCP/IPoE.
- Throughput: iperf3 through the edge at line rate on the trunk, confirming
  offload is active (CPU near idle).

## Sources

- Mono Gateway: <https://mono.si/>, <https://docs.mono.si/gateway-development-kit/hardware-description/>
- ASK flowtable (the bond trap): <https://github.com/we-are-mono/ASK>, <https://github.com/we-are-mono/ASK/blob/master/docs/flowtable/vlan.md>
- Mono review/benchmarks: <https://www.jeffgeerling.com/blog/2026/testing-mono-gateway-custom-built-10-gbps-router/>
- MikroTik L3HW (VRRP vs offload): <https://help.mikrotik.com/docs/spaces/ROS/pages/62390319/L3+Hardware+Offloading>
- pfSense HA / CARP: <https://docs.netgate.com/pfsense/en/latest/highavailability/index.html>
- VyOS HA / conntrack-sync: <https://docs.vyos.io/en/latest/configuration/highavailability/index.html>
- RFC 4638 (baby jumbo for PPPoE MTU), RFC 5798 (VRRPv3)
