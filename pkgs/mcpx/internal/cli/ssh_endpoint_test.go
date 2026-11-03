package cli

import (
	"context"
	"fmt"
	"io"
	"net"
	"net/http"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"github.com/developing-today/code/pkgs/mcpx/internal/daemon"
	"github.com/developing-today/code/pkgs/mcpx/internal/events"
)

// The ssh transport runs the system ssh binary. These tests put a fake
// ssh on PATH: a shell script that drops ssh's own arguments and execs
// this test binary as a shim, which copies the session's stdio to a unix
// socket. That is the same shape as a real remote `mcpx stdio-proxy`
// without needing an sshd.

// TestSSHShimProcess is the fake remote end. It is not a test of its
// own: it returns immediately unless the fake ssh script passes the
// "shim" marker, which only happens in a child process.
func TestSSHShimProcess(t *testing.T) {
	marked := false
	for _, a := range os.Args {
		if a == "shim" {
			marked = true
		}
	}
	if !marked {
		return
	}
	sock := os.Getenv("MCPX_TEST_SSH_SOCK")
	conn, err := net.Dial("unix", sock)
	if err != nil {
		fmt.Fprintln(os.Stderr, err)
		os.Exit(1)
	}
	go func() {
		_, _ = io.Copy(conn, os.Stdin)
		if cw, ok := conn.(interface{ CloseWrite() error }); ok {
			_ = cw.CloseWrite()
		}
	}()
	_, _ = io.Copy(os.Stdout, conn)
	os.Exit(0)
}

// fakeSSH writes an executable named ssh into a temporary directory and
// puts that directory first on PATH.
func fakeSSH(t *testing.T, body string) {
	t.Helper()
	dir := t.TempDir()
	script := filepath.Join(dir, "ssh")
	if err := os.WriteFile(script, []byte(body), 0o755); err != nil {
		t.Fatal(err)
	}
	t.Setenv("PATH", dir+string(os.PathListSeparator)+os.Getenv("PATH"))
}

// shimSSH is a fake ssh that logs its arguments and execs the test
// binary as the stdio shim.
func shimSSH(t *testing.T, sock, logPath string) {
	t.Helper()
	t.Setenv("MCPX_TEST_SSH_SOCK", sock)
	t.Setenv("MCPX_TEST_SSH_LOG", logPath)
	fakeSSH(t, "#!/bin/sh\n"+
		"printf '%s\\n' \"$@\" > \"$MCPX_TEST_SSH_LOG\"\n"+
		"exec \""+os.Args[0]+"\" -test.run '^TestSSHShimProcess$' -- shim\n")
}

// testSocketServer serves the daemon routes a client test needs: a
// health check and a server-sent event stream.
func testSocketServer(t *testing.T) string {
	t.Helper()
	sock := filepath.Join(t.TempDir(), "daemon.sock")
	ln, err := net.Listen("unix", sock)
	if err != nil {
		t.Fatal(err)
	}
	mux := http.NewServeMux()
	mux.HandleFunc("/v1/health", func(w http.ResponseWriter, r *http.Request) {
		_, _ = io.WriteString(w, `{"ok":true}`)
	})
	mux.HandleFunc("/v1/events", func(w http.ResponseWriter, r *http.Request) {
		w.Header().Set("Content-Type", "text/event-stream")
		_, _ = io.WriteString(w, "data: {\"seq\":1,\"kind\":\"server\",\"message\":\"hello\"}\n\n")
		if f, ok := w.(http.Flusher); ok {
			f.Flush()
		}
		<-r.Context().Done()
	})
	srv := &http.Server{Handler: mux}
	go func() { _ = srv.Serve(ln) }()
	t.Cleanup(func() { _ = srv.Close() })
	return sock
}

// An ssh:// endpoint reaches the remote daemon over the tunnel, and the
// tunnel really is `ssh ... <bin> stdio-proxy`.
func TestSSHClientDialsThroughSSH(t *testing.T) {
	sock := testSocketServer(t)
	logPath := filepath.Join(t.TempDir(), "ssh.args")
	shimSSH(t, sock, logPath)

	paths := daemon.Paths{State: t.TempDir(), Socket: socketPathFor(t)}
	c := NewClientAt(paths, "", "ssh://buildbox:2222?bin=/opt/mcpx/mcpx")

	ctx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
	defer cancel()
	if !c.Ping(ctx) {
		b, err := c.do(ctx, http.MethodGet, "/v1/health", nil)
		t.Fatalf("Ping over ssh failed: %v (%s)", err, b)
	}
	if err := c.EnsureDaemon(ctx); err != nil {
		t.Fatalf("EnsureDaemon over ssh: %v", err)
	}

	args, err := os.ReadFile(logPath)
	if err != nil {
		t.Fatal(err)
	}
	got := string(args)
	for _, want := range []string{"-T", "BatchMode=yes", "2222", "buildbox", "/opt/mcpx/mcpx", "stdio-proxy"} {
		if !strings.Contains(got, want) {
			t.Errorf("ssh args %q missing %q", got, want)
		}
	}
}

// A stream is a long-lived GET, so it proves the tunnel is not a
// request/response shortcut: bytes arrive before the stream ends.
func TestSSHClientStreamsEvents(t *testing.T) {
	sock := testSocketServer(t)
	shimSSH(t, sock, filepath.Join(t.TempDir(), "ssh.args"))

	paths := daemon.Paths{State: t.TempDir(), Socket: socketPathFor(t)}
	c := NewClientAt(paths, "", "ssh://buildbox")

	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()
	got := make(chan string, 1)
	go func() {
		_ = c.Stream(ctx, events.Filter{}, func(e events.Event) {
			select {
			case got <- string(e.Kind):
			default:
			}
			cancel()
		})
	}()
	select {
	case kind := <-got:
		if kind != "server" {
			t.Fatalf("streamed event kind = %q; want server", kind)
		}
	case <-time.After(5 * time.Second):
		t.Fatal("no event arrived over the ssh stream")
	}
}

// An ssh endpoint must never start a local daemon, and the failure must
// name the host and the remote command.
func TestSSHEndpointDoesNotAutostart(t *testing.T) {
	fakeSSH(t, "#!/bin/sh\necho 'ssh: connect to host buildbox port 22: Connection refused' >&2\nexit 255\n")
	paths := daemon.Paths{State: t.TempDir(), Socket: socketPathFor(t)}
	c := NewClientAt(paths, "", "ssh://buildbox?bin=/opt/mcpx/mcpx")

	ctx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
	defer cancel()
	err := c.EnsureDaemon(ctx)
	if err == nil {
		t.Fatal("EnsureDaemon succeeded with an unreachable ssh host")
	}
	msg := err.Error()
	for _, want := range []string{"ssh://buildbox", "/opt/mcpx/mcpx stdio-proxy", "not started from here"} {
		if !strings.Contains(msg, want) {
			t.Errorf("error %q missing %q", msg, want)
		}
	}
	if _, statErr := os.Stat(paths.Socket); statErr == nil {
		t.Error("a local daemon socket was created for a remote endpoint")
	}
}

// A malformed ssh endpoint fails at dial time with the parse error, not
// as a silent plain-TCP connection to the host.
func TestMalformedSSHEndpointFails(t *testing.T) {
	paths := daemon.Paths{State: t.TempDir(), Socket: socketPathFor(t)}
	c := NewClientAt(paths, "", "ssh://buildbox:notaport")
	if c.Ping(context.Background()) {
		t.Fatal("a malformed ssh endpoint reported a reachable daemon")
	}
	_, err := c.do(context.Background(), http.MethodGet, "/v1/health", nil)
	if err == nil || !strings.Contains(err.Error(), "invalid ssh endpoint") {
		t.Fatalf("dial error = %v; want the parse error", err)
	}
}

// socketPathFor is a socket path that must not exist: a client that
// falls back to the local daemon would create or use it.
func socketPathFor(t *testing.T) string {
	t.Helper()
	return filepath.Join(t.TempDir(), "should-not-be-used.sock")
}

// The CLI connection ladder gets the same slow-handshake fallback as
// EnsureDaemon: Connect's named-endpoint rung used to Ping once, so a
// first ssh handshake slower than autostart.pingTimeout (2s) made every
// ordinary command report a reachable daemon as dead. The fake ssh
// sleeps 3s on its first invocation only; the fallback health request
// dials again and answers at once.
func TestSSHConnectFallsBackPastSlowHandshake(t *testing.T) {
	sock := testSocketServer(t)
	t.Setenv("MCPX_TEST_SSH_SOCK", sock)
	marker := filepath.Join(t.TempDir(), "ssh.slowed")
	t.Setenv("MCPX_TEST_SSH_SLOW", marker)
	fakeSSH(t, "#!/bin/sh\n"+
		"if [ ! -f \"$MCPX_TEST_SSH_SLOW\" ]; then : > \"$MCPX_TEST_SSH_SLOW\"; sleep 3; fi\n"+
		"exec \""+os.Args[0]+"\" -test.run '^TestSSHShimProcess$' -- shim\n")

	a := &App{Paths: daemon.Paths{State: t.TempDir(), Socket: socketPathFor(t)}}
	ctx, cancel := context.WithTimeout(context.Background(), 20*time.Second)
	defer cancel()
	c, ladder, err := a.Connect(ctx, ConnectOptions{Endpoint: "ssh://buildbox"})
	if err != nil {
		t.Fatalf("Connect over a slow ssh handshake: %v\n%s", err, ladder.Describe())
	}
	if c == nil || ladder.Chosen != "url" {
		t.Fatalf("Connect chose %q; want the named ssh endpoint", ladder.Chosen)
	}
}
