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
| 6 | [x] | Arista 7050QX-32-F | DC (EOS) | 32xQSFP+ 40G, Intel FM6000, 2xPWR-460AC(460W), ~150W typ(4.5W/port), 550ns, LACP L2/L3/L4 sym+resilient, MLAG/2 ISSU, VRRP v2/v3+anycast-gw, BGP/OSPF/IS-IS/ECMP-64/VRF/BFD, VXLAN EVPN HW VTEP, sFlow/LANZ/eAPI/gNMI, SSU/SFR, PFC/ECN, SONiC ok, no MACsec |
| 7 | [x] | Mono Gateway | Router (OpenWrt) | NXP LS1046A 4xA72@1.6GHz, 8GB LPDDR4 ECC, DPAA HW offload 17+Gbps, USB-C PD 65W/~15-25W typ, ~10-50µs DPAA/~100-500µs SW, 5x2.5GbE+1xSFP+, WiFi6+tri-radio M.2, OpenWrt/VyOS/VPP, VRRP/keepalived, FRR BGP/OSPF, SQM/CAKE, IPsec CAAM HW, WireGuard SW ~1-3G |
| 8 | [x] | Cisco 2811 | Router (IOS) | ISR G1, 2xGbE+HWIC/NM, 130W PSU, ~80-100W typ, CEF ~50-200µs, ~100-200Mbps routing, HSRP/VRRP/GLBP, OSPF/BGP/EIGRP, ZBF ~50-100Mbps, AIM-VPN ~85Mbps IPsec, NetFlow/IP SLA/EEM, 2 units for HSRP pair |
| 9 | [x] | Cisco 1841 | Router (IOS) | ISR G1 compact, 2xFE only, 50W PSU, ~40-50W typ, CEF ~100-300µs, ~40-80Mbps routing, FastEthernet limits throughput, HSRP/VRRP, OSPF/BGP/EIGRP, ~20Mbps IPsec SW, lab/learning device |
| 10 | [x] | Cisco 881 | Router (IOS) | ISR G1 SOHO, 4xFE LAN+1xFE WAN, 30W ext adapter, ~15-20W typ, CEF ~50-200µs, ~50-100Mbps routing, integrated switch w/VLANs, ZBF, ~15-25Mbps IPsec SW only, 802.1X on switch ports, fanless desktop |
| 11 | [x] | Netgear XS712T | Prosumer 10GbE | 12x10GBASE-T+2xSFP+ combo, ~25W idle / ~50-65W typ / ~80-95W max, 10GBASE-T PHY ~4-5W/port, S&F ~5-8µs copper / ~1-2µs SFP+, 256 VLANs, LACP 8×8, RSTP only, basic 802.1X/ACLs, SNMP v1/v2c/v3, no CLI/sFlow/L3 |
| 12 | [x] | TRENDnet TEG-30284 | Prosumer L2+ | 24xGbE+4xSFP+, fanless ~10-20W typ / ~28-32W max, S&F ~3-5µs GbE / ~1-3µs SFP+, 256 VLANs, LACP 8×8, STP/RSTP/MSTP, 32 static routes, 802.1X/RADIUS/TACACS+/ACLs/DHCP-snoop/DAI, SNMP v3, CLI |
| 13 | [x] | TP-Link SG3210XHP-M2 | Prosumer PoE | 8x2.5G-PoE+2xSFP+, 240W PoE, ~20-25W no-PoE / ~265W max, S&F ~3-5µs, 4K VLANs, LACP 8×8, MSTP, 48 static routes, DHCP server, 802.1X/RADIUS/TACACS+, SNMP v3, Omada SDN |
| 14 | [x] | Dell PowerConnect 5448 | Prosumer stackable | 48xGbE+4xSFP combo, ~30-35W idle / ~45-55W typ / ~65-75W max, stack up to 12 units (48Gbps backplane, cross-stack LAG), S&F ~5-10µs local / ~15-25µs cross-stack, 256 VLANs, LACP 6×8, RSTP only, 802.1X/RADIUS/DHCP-snoop, SNMP v3 |
| 15 | [x] | Cisco SG300-52 | SMB L3-lite | 50xGbE+2xSFP combo, ~15-20W idle / ~35-42W typ / ~55-60W max, S&F ~5-8µs GbE / ~3-5µs SFP, 4094 VLANs, LACP 8×8, STP/RSTP/MSTP, L3-lite 128 static routes / 64 SVIs / inter-VLAN / DHCP relay+server, private VLAN, 802.1X multi-session / RADIUS / TACACS+ / ACLs L2-L4 / DHCP-snoop / DAI / IP-src-guard, SNMP v3, RMON, SPAN |
| 16 | [x] | Netgear GS116E | Consumer | 16xGbE desktop, ~5W idle / ~7-8W typ / ~10W max, ext 12V PSU, S&F ~3-5µs, 64 VLANs only, no LACP/STP/802.1X/ACL/CLI, loop-detect only, 4K MAC, IGMP snoop v1/v2, SNMP v1/v2c, 1 SPAN session, Plus web GUI only |
| 17 | [x] | Cisco 3560 | Enterprise L3 | 24/48xGbE+4xSFP, ~25-30W idle / ~35-50W typ / ~60-80W max (non-PoE), ~370W PoE models, S&F ~3-6µs GbE / ~2-4µs SFP, 4094 VLANs+PVLAN, LACP 48×8 L2/L3/L4 hash, no stacking (3750 is stackable), HSRP v1/v2+VRRP+GLBP (IP Svc), OSPF/EIGRP/BGP/PBR/VRF-lite, SPAN/RSPAN, IP SLA, EEM, 802.1X multi-auth/RADIUS/TACACS+/DAI/IPSG/port-sec |
| 18 | [x] | Cisco 2960 | Enterprise L2 | 24/48xGbE+2-4xSFP, ~15-25W idle / ~25-40W typ / ~45-60W max (non-PoE), ~740W PoE+ models, S&F ~3-6µs / +10-20µs cross-stack, 4094 VLANs LAN Base / 64 LAN Lite, LACP 48×8 L2/L3, cross-stack LAG (FlexStack S/X), no MC-LAG, L2 only (limited static on S/X), 802.1X multi-auth/MAB/RADIUS/TACACS+/DAI/IPSG, SNMP v3, RMON, SPAN/RSPAN, no NetFlow |
| 19 | [x] | Cisco ASA 5505 | Firewall | 8xFE switch, ~12-15W idle / ~18-22W typ / ~25-30W max, ext 12V PSU fanless, 2-port PoE 7W total, ~0.5-2ms firewall / +1-5ms VPN, 3 VLANs base / 20 Security Plus, no STP/LACP/LLDP, 150Mbps stateful FW, 100Mbps VPN (3DES/AES), 10/25 IPsec peers, SSL VPN 2 peers, A/S failover (Sec+), NSEL NetFlow, RADIUS/TACACS+, SNMP v3, ASDM GUI |
| 20 | [x] | Cisco 4402 WLC | WLAN Controller | 4xGbE (2 dist+2 svc), ~65-85W typ, internal AC single PSU, 802.11a/b/g only (54Mbps max, no n/ac/ax), up to 50 APs (base 12/25), ~1-3ms CAPWAP tunnel / ~5-10ms roaming, RRM auto RF, WPA/WPA2 enterprise (EAP-TLS/PEAP/EAP-FAST), rogue AP detect+contain, wIPS (MSE), WMM QoS, SNMP v3, WCS/Prime |
| 21 | [x] | Calix GP1101X | ISP CPE (ONT) | 1x10GBASE-T+1xPOTS, XGS-PON 10G/10G, ~5-7W idle / ~8-10W typ / ~12-15W max, ext 12V DC, optional battery 4-8hr VoIP standby, ~100-500µs total pass-through (PON grant ~125µs-1ms + PHY ~2-3µs), ISP-configured 802.1Q/QoS only, no user config, OMCI/AXOS/SMx mgmt, LED status only, ISP remote diagnostics |

## Gaps

_To be populated during Phase 2 gap analysis._
