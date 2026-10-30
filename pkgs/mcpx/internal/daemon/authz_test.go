package daemon

import (
	"context"
	"net"
	"os"
	"path/filepath"
	"testing"

	"github.com/dezren39/mcpx/internal/api"
)

// A unix-socket connection's principal carries the peer uid the kernel
// reports — on this machine, the test process's own uid.
func TestPrincipalForUnixConnReadsPeerUID(t *testing.T) {
	socket := filepath.Join(t.TempDir(), "probe.sock")
	ln, err := net.Listen("unix", socket)
	if err != nil {
		t.Fatal(err)
	}
	defer ln.Close()
	go func() {
		c, err := net.Dial("unix", socket)
		if err == nil {
			c.Close()
		}
	}()
	conn, err := ln.Accept()
	if err != nil {
		t.Fatal(err)
	}
	defer conn.Close()
	p := principalForConn(conn)
	if p.Surface != "unix" {
		t.Fatalf("principal surface = %q, want unix", p.Surface)
	}
	want := api.UnixPeerPrincipal(uint32(os.Getuid()))
	if p != want {
		t.Errorf("principal = %+v, want %+v (peer credentials for our own uid)", p, want)
	}
}

// A loopback TCP connection's principal is its address and no invented
// identity.
func TestPrincipalForTCPConnIsItsAddress(t *testing.T) {
	ln, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		t.Fatal(err)
	}
	defer ln.Close()
	go func() {
		c, err := net.Dial("tcp", ln.Addr().String())
		if err == nil {
			c.Close()
		}
	}()
	conn, err := ln.Accept()
	if err != nil {
		t.Fatal(err)
	}
	defer conn.Close()
	p := principalForConn(conn)
	if p.Surface != "tcp" || p.ID == "" {
		t.Errorf("principal = %+v, want surface tcp with the remote address as ID", p)
	}
}

// connPrincipal puts the connection's principal where handlers look for
// it: the request context.
func TestConnPrincipalCarriesThePrincipal(t *testing.T) {
	ln, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		t.Fatal(err)
	}
	defer ln.Close()
	go func() {
		c, err := net.Dial("tcp", ln.Addr().String())
		if err == nil {
			c.Close()
		}
	}()
	conn, err := ln.Accept()
	if err != nil {
		t.Fatal(err)
	}
	defer conn.Close()
	ctx := connPrincipal(context.Background(), conn)
	p, ok := api.PrincipalFrom(ctx)
	if !ok {
		t.Fatal("connPrincipal context carries no principal")
	}
	if p.Surface != "tcp" {
		t.Errorf("principal = %+v, want the tcp principal for the connection", p)
	}
}
