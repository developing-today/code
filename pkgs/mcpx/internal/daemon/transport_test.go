package daemon

import (
	"context"
	"net"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"github.com/dezren39/mcpx/internal/config"
)

// The two listeners the daemon has always had now run through the
// Transport interface; this pins that the seam did not change what they
// do — bind, and be reachable at the address they report.
func TestBuiltinTransportsListen(t *testing.T) {
	for _, tr := range []Transport{
		unixTransport{socket: filepath.Join(t.TempDir(), "mcpx.sock")},
		tcpTransport{address: "127.0.0.1:0"},
	} {
		ln, err := tr.Listen()
		if err != nil {
			t.Fatalf("%s transport: %v", tr.Name(), err)
		}
		defer ln.Close()
		go func() {
			conn, err := ln.Accept()
			if err == nil {
				conn.Close()
			}
		}()
		ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
		defer cancel()
		var d net.Dialer
		conn, err := d.DialContext(ctx, ln.Addr().Network(), ln.Addr().String())
		if err != nil {
			t.Fatalf("%s transport: dial %s: %v", tr.Name(), ln.Addr(), err)
		}
		conn.Close()
	}
}

func TestParseTSNetEndpoint(t *testing.T) {
	cases := []struct {
		endpoint string
		host     string
		port     int
		ok       bool
		wantErr  bool
	}{
		{"tsnet://laptop.tailnet.ts.net", "laptop.tailnet.ts.net", TSNetPort, true, false},
		{"tsnet://laptop.tailnet.ts.net:8443", "laptop.tailnet.ts.net", 8443, true, false},
		{"tsnet://", "", 0, true, true},
		{"tsnet://host:notaport", "", 0, true, true},
		{"http://127.0.0.1:8065", "", 0, false, false},
		{"unix:///tmp/mcpx.sock", "", 0, false, false},
		{"", "", 0, false, false},
	}
	for _, tc := range cases {
		host, port, ok, err := ParseTSNetEndpoint(tc.endpoint)
		if ok != tc.ok || host != tc.host || port != tc.port || (err != nil) != tc.wantErr {
			t.Errorf("ParseTSNetEndpoint(%q) = %q, %d, %v, %v; want %q, %d, %v, err=%v",
				tc.endpoint, host, port, ok, err, tc.host, tc.port, tc.ok, tc.wantErr)
		}
	}
}

// A tsnet listener with no auth key must fail with the reason, never
// quietly serve somewhere else: the whole point of the identity route
// is that it is the route the operator named.
func TestTSNetTransportRequiresAuthKey(t *testing.T) {
	_, err := tsnetTransport{hostname: "mcpx-daemon", stateDir: t.TempDir()}.Listen()
	if err == nil || !strings.Contains(err.Error(), TSNetAuthKeyEnv) {
		t.Fatalf("Listen without auth key = %v; want an error naming %s", err, TSNetAuthKeyEnv)
	}
}

func TestTSNetDialerRequiresAuthKey(t *testing.T) {
	d := &TSNetDialer{hostname: "mcpx-client"}
	_, err := d.DialContext(context.Background(), "tcp", "laptop.tailnet.ts.net:80")
	if err == nil || !strings.Contains(err.Error(), TSNetAuthKeyEnv) {
		t.Fatalf("Dial without auth key = %v; want an error naming %s", err, TSNetAuthKeyEnv)
	}
}

// The tsnet node state directory is per-daemon, keyed by the
// configuration file set: two daemons sharing one state directory
// must not load each other's node keys (Codex finding on PR #2203),
// and one daemon's directory must not move when its configuration is
// reloaded or handed over -- the fingerprint is over the file set,
// not the contents.
func TestTSNetStateDirKeyedByConfig(t *testing.T) {
	state := t.TempDir()
	mk := func(sources ...string) *Server {
		s := &Server{paths: Paths{State: state}}
		if sources != nil {
			s.cfg = &config.Config{Sources: sources}
		}
		return s
	}
	a := mk("/proj-a/.mcpx.json")
	b := mk("/proj-b/.mcpx.json")
	if a.tsnetStateDir() == b.tsnetStateDir() {
		t.Fatalf("two configurations share a tsnet state dir: %s", a.tsnetStateDir())
	}
	if again := mk("/proj-a/.mcpx.json"); again.tsnetStateDir() != a.tsnetStateDir() {
		t.Fatalf("same configuration, different tsnet state dir: %s vs %s", again.tsnetStateDir(), a.tsnetStateDir())
	}
	want := filepath.Join(state, "tsnet")
	if got := mk().tsnetStateDir(); got != want {
		t.Fatalf("no configuration: got %s, want the unkeyed %s", got, want)
	}
}
