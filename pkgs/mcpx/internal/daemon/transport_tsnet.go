package daemon

import (
	"context"
	"fmt"
	"net"
	"net/url"
	"os"
	"strconv"
	"strings"
	"sync"

	"tailscale.com/tsnet"
)

// Identity transport (#1967): a daemon reachable by its tailnet identity
// (a MagicDNS name) rather than by an address — from another network,
// with no port forwarded and no public IP. The decision doc
// (dezren39/nix planning/plans/mcpx-identity-transport.md) picked tsnet:
// it embeds a Tailscale node in this Go process, so mcpx stays a pure-Go
// CGO_ENABLED=0 binary; iroh via cgo was rejected there and is not
// revisited here.

// TSNetAuthKeyEnv is where the tsnet transports read their Tailscale
// auth key: the standard variable name for tsnet programs. The key is a
// credential — it is never a setting, so it can never land in a config
// file, a settings dump, or this package's logs.
const TSNetAuthKeyEnv = "TS_AUTHKEY"

// TSNetPort is the port the daemon's tsnet listener serves on, and the
// default port of a tsnet:// endpoint.
const TSNetPort = 80

// tsnetTransport serves the daemon API on a tailnet. StateDir holds the
// node's identity (its keys) across restarts, so the daemon keeps the
// same MagicDNS name.
type tsnetTransport struct {
	hostname string
	stateDir string
	authKey  string
}

func (t tsnetTransport) Name() string { return "tsnet" }

func (t tsnetTransport) Listen() (net.Listener, error) {
	node, err := newTSNetNode(t.hostname, t.stateDir, t.authKey, false)
	if err != nil {
		return nil, err
	}
	return node.Listen()
}

// tsnetNode is a running tsnet server plus the listener it produced, so
// status queries can reach the node's local API.
type tsnetNode struct {
	server *tsnet.Server
}

func newTSNetNode(hostname, stateDir, authKey string, ephemeral bool) (*tsnetNode, error) {
	if strings.TrimSpace(hostname) == "" {
		return nil, fmt.Errorf("tsnet transport: no hostname: set daemon.tsnet.hostname to this daemon's tailnet name")
	}
	if authKey == "" {
		return nil, fmt.Errorf("tsnet transport: %s is not set: the daemon cannot join the tailnet without an auth key, and it will not fall back to another route — create one in the Tailscale admin console and export it as %s", TSNetAuthKeyEnv, TSNetAuthKeyEnv)
	}
	return &tsnetNode{server: &tsnet.Server{
		Hostname:  hostname,
		Dir:       stateDir,
		AuthKey:   authKey,
		Ephemeral: ephemeral,
	}}, nil
}

// Listen serves the daemon's HTTP API on the node's tailnet address.
func (n *tsnetNode) Listen() (net.Listener, error) {
	ln, err := n.server.Listen("tcp", ":"+strconv.Itoa(TSNetPort))
	if err != nil {
		return nil, fmt.Errorf("tsnet listen %s: %w", n.server.Hostname, err)
	}
	return &tsnetListener{Listener: ln, node: n}, nil
}

// tsnetListener keeps the node reachable from the listener the daemon
// holds, so status can ask it about peers.
type tsnetListener struct {
	net.Listener
	node *tsnetNode
}

// TSNetPeer is one tailnet peer as the status surface reports it:
// whether it is online, and whether traffic to it goes direct or via a
// DERP relay.
type TSNetPeer struct {
	DNSName    string
	Online     bool
	Active     bool
	Connection string // "direct", "relayed", or "" when neither is known
	Relay      string // DERP region code when relayed
}

// TSNetStatus is the tsnet half of daemon status: this node's identity
// and, per peer, direct-vs-relayed — the distinction that decides
// whether an identity endpoint is fast or merely reachable.
type TSNetStatus struct {
	Hostname string
	DNSName  string
	IPs      []string
	Peers    []TSNetPeer
}

// Status reads the node's local API for its identity and peer states.
func (l *tsnetListener) Status(ctx context.Context) (TSNetStatus, error) {
	lc, err := l.node.server.LocalClient()
	if err != nil {
		return TSNetStatus{}, fmt.Errorf("tsnet status: %w", err)
	}
	st, err := lc.Status(ctx)
	if err != nil {
		return TSNetStatus{}, fmt.Errorf("tsnet status: %w", err)
	}
	out := TSNetStatus{Hostname: l.node.server.Hostname}
	if st.Self != nil {
		out.DNSName = strings.TrimSuffix(st.Self.DNSName, ".")
		for _, ip := range st.Self.TailscaleIPs {
			out.IPs = append(out.IPs, ip.String())
		}
	}
	for _, ps := range st.Peer {
		peer := TSNetPeer{
			DNSName: strings.TrimSuffix(ps.DNSName, "."),
			Online:  ps.Online,
			Active:  ps.Active,
			Relay:   ps.Relay,
		}
		switch {
		case ps.CurAddr != "":
			peer.Connection = "direct"
		case ps.Relay != "":
			peer.Connection = "relayed"
		}
		out.Peers = append(out.Peers, peer)
	}
	return out, nil
}

// ParseTSNetEndpoint parses a tsnet:// daemon endpoint —
// tsnet://<magicdns-name>[:<port>] — into the host a client dials and
// the port. ok is false when the endpoint is not a tsnet one at all; a
// malformed tsnet endpoint is an error, never a silent reinterpretation
// as some other route.
func ParseTSNetEndpoint(endpoint string) (host string, port int, ok bool, err error) {
	rest, isTSNet := strings.CutPrefix(endpoint, "tsnet://")
	if !isTSNet {
		return "", 0, false, nil
	}
	u, perr := url.Parse("tsnet://" + rest)
	if perr != nil || u.Hostname() == "" {
		return "", 0, true, fmt.Errorf("invalid tsnet endpoint %q: want tsnet://<daemon's tailnet name>[:%d]", endpoint, TSNetPort)
	}
	port = TSNetPort
	if p := u.Port(); p != "" {
		n, cerr := strconv.Atoi(p)
		if cerr != nil || n < 1 || n > 65535 {
			return "", 0, true, fmt.Errorf("invalid tsnet endpoint %q: bad port %q", endpoint, p)
		}
		port = n
	}
	return u.Hostname(), port, true, nil
}

// TSNetDialer dials a tsnet:// endpoint from a client process. The
// client joins the tailnet as an ephemeral node on first use: it exists
// only to reach the daemon, holds no state, and disappears with the
// process. Without an auth key the dial fails with the reason — a
// tsnet endpoint is never silently retried as a plain TCP address.
type TSNetDialer struct {
	hostname string
	authKey  string

	once sync.Once
	node *tsnetNode
	err  error
}

// NewTSNetDialer builds a dialer for one client hostname (the name this
// client appears under on the tailnet).
func NewTSNetDialer(hostname string) *TSNetDialer {
	return &TSNetDialer{hostname: hostname, authKey: envTSNetAuthKey()}
}

func envTSNetAuthKey() string {
	return strings.TrimSpace(os.Getenv(TSNetAuthKeyEnv))
}

// DialContext dials addr ("host:port", as an http.Transport hands it
// over) through the client's tsnet node.
func (d *TSNetDialer) DialContext(ctx context.Context, _, addr string) (net.Conn, error) {
	d.once.Do(func() {
		d.node, d.err = newTSNetNode(d.hostname, "", d.authKey, true)
	})
	if d.err != nil {
		return nil, d.err
	}
	conn, err := d.node.server.Dial(ctx, "tcp", addr)
	if err != nil {
		return nil, fmt.Errorf("dial tsnet %s: %w", addr, err)
	}
	return conn, nil
}
