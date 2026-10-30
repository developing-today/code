package daemon

import (
	"context"
	"net"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"runtime"
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
	// peerUID reads SO_PEERCRED, which only Linux offers: elsewhere the
	// build deliberately reports no uid and the principal falls back to
	// the surface alone (peercred_other.go).
	want := api.UnixPrincipal()
	if runtime.GOOS == "linux" {
		want = api.UnixPeerPrincipal(uint32(os.Getuid()))
	}
	if p != want {
		t.Errorf("principal = %+v, want %+v", p, want)
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

// The decision is recorded before the handler runs: a handler that
// streams until the client disconnects (GET /v1/events) is recorded
// for its whole lifetime, and a daemon that stops first still leaves
// the record behind.
func TestRouteRecordsDecisionBeforeHandlerRuns(t *testing.T) {
	s := &Server{}
	mux := http.NewServeMux()
	const op = "health"
	before := len(decisionsFor(op))
	s.route(mux, "GET /v1/health", func(w http.ResponseWriter, r *http.Request) {
		if got := len(decisionsFor(op)); got != before+1 {
			t.Errorf("decisions for health during handler = %d, want %d (recorded before the handler ran)", got-before, 1)
		}
		w.WriteHeader(http.StatusNoContent)
	})
	req := httptest.NewRequest(http.MethodGet, "/v1/health", nil)
	s.authorize(mux).ServeHTTP(httptest.NewRecorder(), req)
	if got := len(decisionsFor(op)); got != before+1 {
		t.Errorf("decisions for health after request = %d, want %d", got-before, 1)
	}
}

// A route no operation declares records nothing: a principal but no
// decision, exactly as the funnel documents.
func TestUndeclaredRouteRecordsNothing(t *testing.T) {
	s := &Server{}
	mux := http.NewServeMux()
	s.route(mux, "GET /v1/health", func(w http.ResponseWriter, r *http.Request) {
		w.WriteHeader(http.StatusNoContent)
	})
	before := len(api.Decisions())
	req := httptest.NewRequest(http.MethodGet, "/v1/no-such-route", nil)
	s.authorize(mux).ServeHTTP(httptest.NewRecorder(), req)
	if got := len(api.Decisions()); got != before {
		t.Errorf("decisions grew by %d for an undeclared route, want 0", got-before)
	}
}

// withPrincipal states the caller explicitly: the stamped principal
// reaches the handler's context even when the request's address would
// imply another surface (net.Pipe reads as RemoteAddr "pipe").
func TestWithPrincipalStampsRequests(t *testing.T) {
	inner := http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		p, ok := api.PrincipalFrom(r.Context())
		if !ok {
			t.Error("stamped request carries no principal")
			return
		}
		if p != api.InProcessPrincipal() {
			t.Errorf("principal = %+v, want the stamped in-process principal", p)
		}
	})
	req := httptest.NewRequest(http.MethodGet, "/", nil)
	req.RemoteAddr = "pipe"
	withPrincipal(api.InProcessPrincipal(), inner).ServeHTTP(httptest.NewRecorder(), req)
}

func decisionsFor(op string) []api.Decision {
	var out []api.Decision
	for _, d := range api.Decisions() {
		if d.Op == op {
			out = append(out, d)
		}
	}
	return out
}
