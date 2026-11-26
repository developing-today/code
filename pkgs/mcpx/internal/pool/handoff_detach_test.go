package pool_test

import (
	"context"
	"net"
	"os"
	"path/filepath"
	"testing"
	"time"

	"github.com/developing-today/code/pkgs/mcpx/internal/config"
	"github.com/developing-today/code/pkgs/mcpx/internal/mcpclient"
	"github.com/developing-today/code/pkgs/mcpx/internal/pool"
)

// The takeover race distinguishes the transient already-attached
// refusal (predecessor committed, supervisor still clearing EOF) from
// every other quiet no-op. Only the former may be retried.
func TestAdoptDetailedDistinguishesAlreadyAttached(t *testing.T) {
	stateDir := t.TempDir()
	t.Setenv("MCPX_STATE_DIR", stateDir)
	sock := mcpclient.SupervisorSocketPath(stateDir, "era")
	if err := os.MkdirAll(filepath.Dir(sock), 0o700); err != nil {
		t.Fatal(err)
	}
	ln, err := net.Listen("unix", sock)
	if err != nil {
		t.Fatal(err)
	}
	defer func() { _ = ln.Close() }()
	go func() {
		for {
			conn, err := ln.Accept()
			if err != nil {
				return
			}
			go func(conn net.Conn) {
				defer func() { _ = conn.Close() }()
				buf := make([]byte, 4096)
				_ = conn.SetReadDeadline(time.Now().Add(5 * time.Second))
				if _, err := conn.Read(buf); err != nil {
					return
				}
				_, _ = conn.Write([]byte("{\"error\":\"already attached\"}\n"))
			}(conn)
		}
	}()

	tr := true
	cfg := &config.Config{MCPServers: map[string]*config.Server{
		"era": {Name: "era", Command: "cat", Mcpx: &config.Extras{Supervised: &tr}},
	}}
	r, err := cfg.Resolve("era")
	if err != nil {
		t.Fatal(err)
	}
	p := pool.New(r)
	defer p.Close()
	adopted, alreadyAttached := p.AdoptRunningSupervisedDetailed(context.Background())
	if adopted {
		t.Fatal("stub refuses every attach, nothing should be adopted")
	}
	if !alreadyAttached {
		t.Fatal("already-attached refusal must be reported as retryable")
	}
	if p.AdoptRunningSupervised(context.Background()) {
		t.Fatal("legacy wrapper must still report no adoption")
	}
}
