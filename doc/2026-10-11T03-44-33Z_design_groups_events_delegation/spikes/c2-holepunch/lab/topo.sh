# Sourced inside `unshare -Urn` by lab/run.sh. Builds the lab topology in this namespace (the
# "root" namespace of the brief). Nothing here touches the host's network.
#
#   relay 10.0.0.1 (root) --- br-wan --- gw-a (wan 10.0.0.2) --- lan --- host-a 192.168.1.2
#                                    \-- gw-b (wan 10.0.0.3) --- lan --- host-b 192.168.2.2
#
# Each gateway runs one NAT. $NAT selects it:
#   cone        port-preserving masquerade, plus a static DNAT of $PORT from the WAN to the host:
#               endpoint-independent mapping and endpoint-independent filtering (full cone)
#   restricted  port-preserving masquerade only: endpoint-independent mapping, address-and-port
#               filtering (port-restricted cone)
#   symmetric   masquerade fully-random: a new random external port per destination (symmetric)
#
# Each namespace is held by one `unshare -n sleep` process, with its PID in $STATE/<name>.pid.

ns_new() {
	unshare -n sleep infinity &
	echo $! >"$STATE/$1.pid"
	# Wait until the namespace exists (unshare execs sleep, so the PID is the namespace holder).
	local pid
	pid=$(cat "$STATE/$1.pid")
	for _ in $(seq 50); do
		[ -e "/proc/$pid/ns/net" ] && return 0
		sleep 0.02
	done
	echo "namespace $1 did not appear" >&2
	return 1
}

# Runs a command inside the named namespace.
ns_exec() {
	local name=$1
	shift
	nsenter --net="/proc/$(cat "$STATE/$name.pid")/ns/net" -- "$@"
}

ns_pid() {
	cat "$STATE/$1.pid"
}

# Gateway ruleset. $1 is the gateway's index (1 or 2) and $2 the NAT type.
gateway_rules() {
	local idx=$1 nat=$2 masq
	case "$nat" in
	cone | restricted) masq="masquerade" ;;
	symmetric) masq="masquerade fully-random" ;;
	*)
		echo "unknown NAT type: $nat" >&2
		return 1
		;;
	esac
	local dnat=""
	if [ "$nat" = cone ]; then
		dnat="iifname \"wan\" udp dport $PORT dnat to 192.168.$idx.2:$PORT"
	fi
	cat <<EOF
table ip nat {
	chain pre {
		type nat hook prerouting priority dstnat; policy accept;
		$dnat
	}
	chain post {
		type nat hook postrouting priority srcnat; policy accept;
		oifname "wan" $masq
	}
}
table ip filt {
	chain in {
		# Unsolicited UDP to the gateway itself is dropped, not answered with ICMP.
		type filter hook input priority filter; policy accept;
		iifname "wan" ct state new drop
	}
}
EOF
}

# Builds the topology. Arguments: state directory, peer UDP port, NAT type.
topo_up() {
	STATE=$1
	PORT=$2
	NAT=$3
	# Gateway b's NAT can differ from gateway a's (mixed pairs). Default: the same.
	NAT_B=${4:-$NAT}
	export STATE PORT NAT NAT_B
	mkdir -p "$STATE"

	ip link set lo up
	ip link add br-wan type bridge
	ip addr add 10.0.0.1/24 dev br-wan
	ip link set br-wan up

	local name idx
	for name in a b; do
		idx=$([ "$name" = a ] && echo 1 || echo 2)
		ns_new "gw-$name"
		ns_new "host-$name"
		local gw_pid host_pid
		gw_pid=$(ns_pid "gw-$name")
		host_pid=$(ns_pid "host-$name")

		# WAN: root bridge side <-> gateway's "wan".
		ip link add "wan-root-$name" type veth peer name wan
		ip link set "wan-root-$name" master br-wan up
		ip link set wan netns "$gw_pid"
		ns_exec "gw-$name" ip link set lo up
		ns_exec "gw-$name" ip addr add "10.0.0.$((idx + 1))/24" dev wan
		ns_exec "gw-$name" ip link set wan up

		# LAN: gateway's "lan" <-> host's "eth0".
		ip link add "lan-gw-$name" type veth peer name eth0
		ip link set "lan-gw-$name" netns "$gw_pid"
		ip link set eth0 netns "$host_pid"
		ns_exec "gw-$name" ip addr add "192.168.$idx.1/24" dev "lan-gw-$name"
		ns_exec "gw-$name" ip link set "lan-gw-$name" up
		ns_exec "host-$name" ip link set lo up
		ns_exec "host-$name" ip addr add "192.168.$idx.2/24" dev eth0
		ns_exec "host-$name" ip link set eth0 up
		ns_exec "host-$name" ip route add default via "192.168.$idx.1"

		ns_exec "gw-$name" sysctl -qw net.ipv4.ip_forward=1
		# The gateway's WAN and LAN names are "wan" and "lan-gw-*"; the ruleset matches "wan".
		local gw_nat=$NAT
		[ "$name" = b ] && gw_nat=$NAT_B
		gateway_rules "$idx" "$gw_nat" | ns_exec "gw-$name" "$NFT" -f -
	done
}

# Removes the namespaces. The veth pairs go with them. Safe to call twice.
topo_down() {
	local name pid
	for name in gw-a gw-b host-a host-b; do
		[ -e "$STATE/$name.pid" ] || continue
		pid=$(cat "$STATE/$name.pid")
		kill "$pid" 2>/dev/null || true
	done
}
