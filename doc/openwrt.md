- set password
  - todo: use passwordless root with ssh key (please don't hack me uwu)
  - todo: only use password for http interface
- ```
# ssh in from a machine with tailscale web login
# (do not change ssh interface to tailscale,)
# (advertise lan through tailscale as a route,)
# (don't want to lockout rest of lan from ssh,)
# (at least for now.)
opkg install tailscale
# these packages aren't enough but pretend they do something,
# ipv6 etc dont work and you need to manually add fw rules anyways
opkg install iptables-nft kmod-ipt-conntrack kmod-ipt-conntrack-extra kmod-ipt-conntrack-label kmod-nft-nat
tailscale up
# open link
tailscale status
# add tailscale interface https://openwrt.org/docs/guide-user/services/vpn/tailscale/start
# add tailscale firewall zone https://openwrt.org/docs/guide-user/services/vpn/tailscale/start
ip address show tailscale0
tailscale up --advertise-routes=10.10.0.0/16 --accept-routes --advertise-exit-node
# from the tailscale web interface, approve the route, approve the exit node, disable key expiry
```
- ```
# done
opkg install nano # - for editing text files through ssh
opkg install htop # - for pretty colors #  export TERM=xterm # https://github.com/kovidgoyal/kitty/issues/1613
opkg install iperf3 # - useful to confirm network wifi performance
opkg install lsof
opkg install tmux
opkg install tcpdump # debug
opkg install wget
opkg install mtr # debug
opkg install htop # system info
opkg install curl
# consider
opkg install luci-app-attendedsysupgrade
opkg install luci-app-sqm # (cake ftw)
opkg install luci-app-simple-adblock # (if not done elsewhere)
opkg install luci-app-ddns # (if dynamic IP)
opkg install luci-app-statistics # (graphs of bandwidth, cpu etc.)
opkg install luci-app-advanced-reboot # (if your router is partitioned)
opkg install luci-app-nlbwmon # (monthly breakdown of bandwidth per client / protocol)
opkg install ethtool # check the NIC info
opkg install knot-dig # DNS tool
opkg install stress # to test the system stability
opkg install avahi-nodbus # (mdns across vlans)
opkg install kmod-usb-net-rndis # (tethering support)
opkg install adblock
opkg install doh
opkg install netdata
# nah
```
  - https://www.reddit.com/r/openwrt/comments/ygq14h/must_have_packages_discussion/
- https://github.com/openwrt/packages
- investigate
  - --netfilter-mode=off
    - https://www.reddit.com/r/Tailscale/comments/11btcxf/how_to_setup_tailscale_on_openwrt_router/
  - https://github.com/asvow/luci-app-tailscale
