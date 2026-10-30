package daemon

import (
	"context"
	"io"
	"log"
	"net"
	"net/http"
	"testing"
	"time"

	"github.com/dezren39/mcpx/internal/settings"
)

// The tailnet listener's clients name the daemon by its MagicDNS name;
// the loopback Host rule would refuse every one of them. These tests
// pin the distinction the Host check now makes, and that the check did
// not get weaker for it: on the tailnet listener only this daemon's own
// name or address is accepted.

func requestWithLocalAddr(t *testing.T, local string) *http.Request {
	t.Helper()
	r, err := http.NewRequest(http.MethodGet, "http://example.invalid/v1/status", nil)
	if err != nil {
		t.Fatal(err)
	}
	host, port, err := net.SplitHostPort(local)
	if err != nil {
		t.Fatal(err)
	}
	var p int
	if _, err := net.LookupPort("tcp", port); err == nil {
		p, _ = net.LookupPort("tcp", port)
	}
	addr := &net.TCPAddr{IP: net.ParseIP(host), Port: p}
	return r.WithContext(context.WithValue(r.Context(), http.LocalAddrContextKey, addr))
}

func TestTailnetLocalIP(t *testing.T) {
	cases := []struct {
		local string
		ok    bool
	}{
		{"100.101.102.103:80", true},     // Tailscale CGNAT range
		{"100.64.0.1:80", true},          // range start
		{"100.127.255.254:80", true},     // range end
		{"100.128.0.1:80", false},        // just outside 100.64.0.0/10
		{"127.0.0.1:8065", false},        // loopback is not the tailnet
		{"192.168.1.10:8065", false},     // LAN is not the tailnet
		{"[fd7a:115c:a1e0::1]:80", true}, // Tailscale ULA
		{"[fd7a:115c:a1e1::1]:80", false},
	}
	for _, tc := range cases {
		_, ok := tailnetLocalIP(requestWithLocalAddr(t, tc.local))
		if ok != tc.ok {
			t.Errorf("tailnetLocalIP(local=%s) ok = %v; want %v", tc.local, ok, tc.ok)
		}
	}
}

func TestTailnetHost(t *testing.T) {
	sch, err := settings.New(settings.Registry())
	if err != nil {
		t.Fatal(err)
	}
	set := settings.NewSet(sch).WithOverrides(map[string]string{
		"daemon.tsnet.hostname": "mcpx-daemon",
	}, "test")
	srv := &Server{set: set}
	local := net.ParseIP("100.101.102.103")
	cases := []struct {
		host string
		want bool
	}{
		{"mcpx-daemon", true},
		{"mcpx-daemon:80", true},
		{"mcpx-daemon.my-tailnet.ts.net", true},
		{"mcpx-daemon.my-tailnet.ts.net:80", true},
		{"MCPX-Daemon.My-Tailnet.ts.net", true},
		{"100.101.102.103", true},
		{"100.101.102.103:80", true},
		{"other-daemon.my-tailnet.ts.net", false},
		{"mcpx-daemon.evil.example", false},
		{"100.101.102.104", false}, // another peer's address
		{"127.0.0.1", false},
		{"localhost", false},
		{"", false},
	}
	for _, tc := range cases {
		if got := srv.tailnetHost(tc.host, local); got != tc.want {
			t.Errorf("tailnetHost(%q) = %v; want %v", tc.host, got, tc.want)
		}
	}
}

// A connection with no descriptor (a tsnet gonet connection) must be
// drained by sendConns, not fail the takeover.
func TestSendConnsDrainsDescriptorless(t *testing.T) {
	srv := &Server{logger: log.New(io.Discard, "", 0)}
	mine, peer := net.Pipe()
	defer peer.Close()
	hc := newHandoffConn(mine, false)
	if err := srv.sendConns(nil, []*handoffConn{hc}); err != nil {
		t.Fatalf("sendConns with a descriptorless connection = %v; want nil (drained)", err)
	}
	// The drained connection is closed: the peer sees EOF.
	_ = peer.SetReadDeadline(time.Now().Add(5 * time.Second))
	buf := make([]byte, 1)
	if _, err := peer.Read(buf); err == nil {
		t.Fatal("peer read after drain = nil error; want EOF/closed")
	}
}
