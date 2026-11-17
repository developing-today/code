package pool_test

import (
	"context"
	"net"
	"os"
	"path/filepath"
	"strings"
	"syscall"
	"testing"
	"time"

	"github.com/developing-today/code/pkgs/mcpx/internal/config"
	"github.com/developing-today/code/pkgs/mcpx/internal/mcpclient"
	"github.com/developing-today/code/pkgs/mcpx/internal/pool"
	"github.com/developing-today/code/pkgs/mcpx/internal/testsupport"
)

// Phase 3 of #1384 (#9963), the headline acceptance: a fresh daemon
// start with a config loaded from a file that opts its stdio server
// in runs it supervised (opt-in while sockets are keyed by name). The pool is what
// a daemon start builds from the resolved config, so the proof is:
// load, resolve, acquire, and a supervisor socket exists with the
// instance's child behind it.
func TestDefaultConfigStdioRunsSupervised(t *testing.T) {
	stateDir := t.TempDir()
	t.Setenv("MCPX_STATE_DIR", stateDir)
	bin := testsupport.EraMCPBinary(t)
	log := filepath.Join(t.TempDir(), "frames")
	cfgPath := filepath.Join(t.TempDir(), "mcpx.json")
	body := `{"mcpServers": {"era": {"command": "` + bin + `", "env": {"ERAMCP_MODE": "modern", "ERAMCP_LOG": "` + log + `"}, "mcpx": {"supervised": true}}}}`
	if err := os.WriteFile(cfgPath, []byte(body), 0o644); err != nil {
		t.Fatal(err)
	}
	cfg, err := config.Load(cfgPath)
	if err != nil {
		t.Fatal(err)
	}
	r, err := cfg.Resolve("era")
	if err != nil {
		t.Fatal(err)
	}
	if !r.Mcpx.SupervisedEnabled() {
		t.Fatal("an opted-in config should resolve to supervised")
	}
	p := pool.New(r)
	defer p.Close()
	ctx, cancel := context.WithTimeout(context.Background(), 30*time.Second)
	defer cancel()
	lease, err := p.Acquire(ctx, config.ResolvedScope{})
	if err != nil {
		t.Fatalf("acquire: %v", err)
	}
	sock := mcpclient.SupervisorSocketPath(stateDir, "era")
	if _, err := os.Stat(sock); err != nil {
		t.Fatalf("no supervisor socket at %s: %v", sock, err)
	}
	st := p.Status()
	if len(st.Instances) != 1 || st.Instances[0].PID <= 0 {
		t.Fatalf("status instances = %+v, want one with a child pid", st.Instances)
	}
	childPID := st.Instances[0].PID
	lease.Release()
	p.Close()
	// The child survives the pool (that is the point of supervised);
	// reap it through the scan so the test leaves nothing behind.
	// The reap is acknowledged before the supervisor finishes the
	// kill, so wait for the child to actually exit.
	mcpclient.ScanSupervisorSockets(stateDir, nil, nil)
	deadline := time.Now().Add(10 * time.Second)
	for syscall.Kill(childPID, 0) == nil {
		if time.Now().After(deadline) {
			t.Fatalf("supervised child %d still alive after the reap", childPID)
		}
		time.Sleep(20 * time.Millisecond)
	}
}

// The other #9963 status acceptance: when the adoption scan meets a
// supervisor speaking a different protocol version, the refusal is
// recorded as the pool's last error — what `mcpx status` prints —
// instead of being silently worked around.
func TestAdoptProtocolRefusalSurfacesInStatus(t *testing.T) {
	stateDir := t.TempDir()
	t.Setenv("MCPX_STATE_DIR", stateDir)
	sock := mcpclient.SupervisorSocketPath(stateDir, "era")
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
				_, _ = conn.Write([]byte("{\"version\":999,\"pid\":4242}\n"))
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
	if p.AdoptRunningSupervised(context.Background()) {
		t.Fatal("adoption should refuse a version-mismatched supervisor")
	}
	st := p.Status()
	if !strings.Contains(st.LastError, "protocol version mismatch") {
		t.Fatalf("status lastError = %q, want it to name the protocol version mismatch", st.LastError)
	}
}
