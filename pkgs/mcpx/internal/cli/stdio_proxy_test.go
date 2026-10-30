package cli

import (
	"bytes"
	"context"
	"io"
	"net"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"github.com/dezren39/mcpx/internal/daemon"
)

// The remote half of an ssh:// endpoint: stdin really does reach the
// daemon socket and replies come back on stdout.
func TestPipeConnCopiesBothWays(t *testing.T) {
	sock := filepath.Join(t.TempDir(), "daemon.sock")
	ln, err := net.Listen("unix", sock)
	if err != nil {
		t.Fatal(err)
	}
	defer ln.Close()

	got := make(chan string, 1)
	go func() {
		conn, err := ln.Accept()
		if err != nil {
			return
		}
		defer conn.Close()
		b, _ := io.ReadAll(io.LimitReader(conn, 4))
		got <- string(b)
		_, _ = conn.Write([]byte("pong"))
	}()

	conn, err := net.Dial("unix", sock)
	if err != nil {
		t.Fatal(err)
	}
	var out bytes.Buffer
	if err := pipeConn(conn, strings.NewReader("ping"), &out); err != nil {
		t.Fatalf("pipeConn: %v", err)
	}
	select {
	case in := <-got:
		if in != "ping" {
			t.Errorf("daemon received %q; want ping", in)
		}
	case <-time.After(5 * time.Second):
		t.Fatal("nothing arrived at the socket")
	}
	if out.String() != "pong" {
		t.Errorf("stdout = %q; want pong", out.String())
	}
}

// A remote with no daemon running says which socket was tried and how to
// start one, rather than a bare EOF.
func TestStdioProxyNamesTheSocket(t *testing.T) {
	sock := filepath.Join(t.TempDir(), "absent.sock")
	a := &App{Paths: daemon.Paths{State: t.TempDir(), Socket: sock}}
	err := a.CmdStdioProxy(context.Background(), []string{"--sock", sock})
	if err == nil {
		t.Fatal("CmdStdioProxy succeeded with no daemon")
	}
	msg := err.Error()
	for _, want := range []string{"absent.sock", "mcpx daemon", "?sock="} {
		if !strings.Contains(msg, want) {
			t.Errorf("error %q missing %q", msg, want)
		}
	}
}
