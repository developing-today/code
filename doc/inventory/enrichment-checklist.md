# Device Enrichment Checklist

Track progress of enriching each inventory device with standard attributes
from `standard-attributes.md`. One device at a time, commit after each.

## Progress

| # | Done | Device | Class | Notes |
|---|------|--------|-------|-------|
| 1 | [x] | Celestica DX010 | DC Fabric (SONiC) | 2x800W PSU, ~150-200W typ, ~400ns cut-through, LACP L3+L4 hash, MC-LAG ICCP pairs, VRRP+SAG anycast-gw, BGP/OSPF/IS-IS via FRR, VXLAN EVPN, 128K MAC, sFlow/gNMI |
| 2 | [x] | IBM G8264 | DC TOR (ENOS) | 2x450W, ~330W typ, 880ns cut-through, LACP L2/L3 hash, vLAG (pairs/2)+peer-gw, VRRP IPv4, OSPF/BGP/RIP/PBR, CEE/FCoE/iSCSI, 802.1X, OpenFlow 1.0/1.3.1, sFlow, PTP |
| 3 | [x] | IBM G8264e | DC TOR (ENOS) | Copper variant of G8264: 48x10GBASE-T+4xQSFP+, ~550-750W PSU est, ~450-550W typ (48 PHYs add ~144-168W), ~2-4µs latency (copper PHY DSP ~1.5-3µs + 880ns ASIC), LACP L2/L3 hash, vLAG pairs/2, VRRP IPv4, OSPF/BGP, CEE/FCoE, all features same as G8264 ENOS |
| 4 | [x] | IBM G8316 | DC Spine (ENOS) | 16xQSFP+ 40G spine, 2x450W PSU, ~330W typ, 880ns cut-through (same ASIC as G8264), LACP L2/L3 hash, vLAG pairs/2+peer-gw, VRRP IPv4 only, OSPF/BGP(v4)/RIP/PBR, CEE/FCoE, CoPP, OpenFlow, sFlow, PTP. No stacking, no VXLAN, no VRF |
| 5 | [x] | Mellanox SX6036 | HPC/DC (MLNX-OS) | 36xQSFP VPI (IB FDR 56G or 40GbE), SwitchX-2, 170ns IB/~300ns Eth, 126W passive/231W active, native IB RDMA + RoCE(adapter), SM 648 nodes, 9 VLs, PFC/ECN/DCBX, CoPP, LACP, OSPF/BGP(Eth), no MC-LAG, no stacking |
| 6 | [ ] | Arista 7050QX-32-F | DC (EOS) | |
| 7 | [ ] | Mono Gateway | Router (OpenWrt) | |
| 8 | [ ] | Cisco 2811 | Router (IOS) | |
| 9 | [ ] | Cisco 1841 | Router (IOS) | |
| 10 | [ ] | Cisco 881 | Router (IOS) | |
| 11 | [ ] | Netgear XS712T | Prosumer 10GbE | |
| 12 | [ ] | TRENDnet TEG-30284 | Prosumer L2+ | |
| 13 | [ ] | TP-Link SG3210XHP-M2 | Prosumer PoE | |
| 14 | [ ] | Dell PowerConnect 5448 | Prosumer stackable | |
| 15 | [ ] | Cisco SG300-52 | SMB L3-lite | |
| 16 | [ ] | Netgear GS116E | Consumer | |
| 17 | [ ] | Cisco 3560 | Enterprise L3 | |
| 18 | [ ] | Cisco 2960 | Enterprise L2 | |
| 19 | [ ] | Cisco ASA 5505 | Firewall | |
| 20 | [ ] | Cisco 4402 WLC | WLAN Controller | |
| 21 | [ ] | Calix GP1101X | ISP CPE (ONT) | |

## Gaps

_To be populated during Phase 2 gap analysis._
