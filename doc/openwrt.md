- set password
  - todo: use passwordless root with ssh key (please don't hack me uwu)
  - todo: only use password for http interface
- ```
# ssh in from a machine with tailscale web login
# (do not change ssh interface to tailscale,)
# (advertise lan through tailscale as a route,)
# (don't want to lockout rest of lan from ssh,)
# (at least for now.)
echo "nameserver 8.8.8.8" > /etc/resolv.conf
apk update
apk upgrade
apk add luci-app-attendedsysupgrade
uci set attendedsysupgrade.client.login_check_for_upgrades='1'
apk update
apk add python3-pip
pip install speedtest-cli
<!--speedtest-cli-->
apk add luci-app-sqm
apk add tailscale
# these packages aren't enough but pretend they do something,
# ipv6 etc dont work and you need to manually add fw rules anyways
apk add iptables-nft kmod-ipt-conntrack kmod-ipt-conntrack-extra kmod-ipt-conntrack-label kmod-nft-nat
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
apk add nano # - for editing text files through ssh
apk add htop # - for pretty colors #  export TERM=xterm # https://github.com/kovidgoyal/kitty/issues/1613
apk add iperf3 # - useful to confirm network wifi performance
apk add lsof
apk add tmux
apk add tcpdump # debug
apk add wget
apk add mtr # debug
apk add htop # system info
apk add curl
# consider
apk add luci-app-attendedsysupgrade
apk add luci-app-sqm # (cake ftw)
apk add luci-app-simple-adblock # (if not done elsewhere)
apk add luci-app-ddns # (if dynamic IP)
apk add luci-app-statistics # (graphs of bandwidth, cpu etc.)
apk add luci-app-advanced-reboot # (if your router is partitioned)
apk add luci-app-nlbwmon # (monthly breakdown of bandwidth per client / protocol)
apk add ethtool # check the NIC info
apk add knot-dig # DNS tool
apk add stress # to test the system stability
apk add avahi-nodbus # (mdns across vlans)
apk add kmod-usb-net-rndis # (tethering support)
apk add adblock
apk add doh
apk add netdata
# nah
```
  - https://www.reddit.com/r/openwrt/comments/ygq14h/must_have_packages_discussion/
- https://github.com/openwrt/packages
- investigate
  - --netfilter-mode=off
    - https://www.reddit.com/r/Tailscale/comments/11btcxf/how_to_setup_tailscale_on_openwrt_router/
  - https://github.com/asvow/luci-app-tailscale
