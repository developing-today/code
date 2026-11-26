package pool_test

import (
	"context"
	"errors"
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
// start with a default config loaded from a file — no supervised key
// anywhere — runs its stdio server supervised. The pool is what
// a daemon start builds from the resolved config, so the proof is:
// load, resolve, acquire, and a supervisor socket exists with the
// instance's child behind it.
func TestDefaultConfigStdioRunsSupervised(t *testing.T) {
	stateDir := t.TempDir()
	t.Setenv("MCPX_STATE_DIR", stateDir)
	bin := testsupport.EraMCPBinary(t)
	log := filepath.Join(t.TempDir(), "frames")
	cfgPath := filepath.Join(t.TempDir(), "mcpx.json")
	body := `{"mcpServers": {"era": {"command": "` + bin + `", "env": {"ERAMCP_MODE": "modern", "ERAMCP_LOG": "` + log + `"}}}}`
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
		t.Fatal("a default file-loaded config should resolve to supervised")
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
	// An attached daemon owns this supervisor: a scan during pool shutdown
	// can legitimately be refused until the transport finishes detaching.
	outcomes := mcpclient.ScanSupervisorSockets(stateDir, nil, nil)
	if len(outcomes) != 1 || outcomes[0].Action != "refused" {
		t.Fatalf("scan while attached = %+v, want one refused supervisor", outcomes)
	}
	lease.Release()
	p.Close()
	// The child survives the pool (that is the point of supervised);
	// reap it through the scan so the test leaves nothing behind. Closing
	// the pool's connection detaches asynchronously; StopSupervisors retries
	// a refusal while that detach finishes and waits for the child to be
	// reaped before returning.
	if err := testsupport.StopSupervisors(stateDir); err != nil {
		t.Fatalf("stop supervised child: %v", err)
	}
	if err := syscall.Kill(childPID, 0); err == nil {
		t.Fatalf("supervised child %d still alive after the reap", childPID)
	} else if !errors.Is(err, syscall.ESRCH) {
		t.Fatalf("check supervised child %d after the reap: %v", childPID, err)
	}
}

// The defect that forced #11343 to pause the default: one server
// acquired under two scopes at once. The supervisor socket is keyed
// per instance, so the second acquire no longer dials the first
// instance's supervisor and gets refused "already attached" — each
// scope gets its own supervisor, its own socket, and its own child.
func TestSameServerTwoScopesGetDistinctSupervisors(t *testing.T) {
	stateDir := t.TempDir()
	t.Setenv("MCPX_STATE_DIR", stateDir)
	bin := testsupport.EraMCPBinary(t)
	log := filepath.Join(t.TempDir(), "frames")
	cfgPath := filepath.Join(t.TempDir(), "mcpx.json")
	body := `{"mcpServers": {"era": {"command": "` + bin + `", "env": {"ERAMCP_MODE": "modern", "ERAMCP_LOG": "` + log + `"}, "mcpx": {"scope": "cwd", "max": 4}}}}`
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
	p := pool.New(r)
	defer p.Close()
	ctx, cancel := context.WithTimeout(context.Background(), 30*time.Second)
	defer cancel()
	rootA, rootB := t.TempDir(), t.TempDir()
	scopeA := config.ResolvedScope{Key: "cwd:" + rootA, Root: rootA}
	scopeB := config.ResolvedScope{Key: "cwd:" + rootB, Root: rootB}
	defer func() { _ = testsupport.StopSupervisors(stateDir) }()
	leaseA, err := p.Acquire(ctx, scopeA)
	if err != nil {
		t.Fatalf("acquire scope A: %v", err)
	}
	defer leaseA.Release()
	leaseB, err := p.Acquire(ctx, scopeB)
	if err != nil {
		t.Fatalf("acquire scope B (the #11343 refusal case): %v", err)
	}
	defer leaseB.Release()

	sockA := mcpclient.SupervisorSocketPathForInstance(stateDir, "era", scopeA.Key)
	sockB := mcpclient.SupervisorSocketPathForInstance(stateDir, "era", scopeB.Key)
	if sockA == sockB {
		t.Fatalf("two scopes share supervisor socket %s", sockA)
	}
	for _, sock := range []string{sockA, sockB} {
		if _, err := os.Stat(sock); err != nil {
			t.Fatalf("no supervisor socket at %s: %v", sock, err)
		}
	}
	st := p.Status()
	if len(st.Instances) != 2 {
		t.Fatalf("status instances = %+v, want two", st.Instances)
	}
	pids := map[int]bool{}
	for _, in := range st.Instances {
		if in.PID <= 0 {
			t.Fatalf("instance %+v has no child pid", in)
		}
		pids[in.PID] = true
	}
	if len(pids) != 2 {
		t.Fatalf("instances share a child: %+v", st.Instances)
	}
	leaseA.Release()
	leaseB.Release()
	p.Close()
	if err := testsupport.StopSupervisors(stateDir); err != nil {
		t.Fatalf("stop supervised children: %v", err)
	}
	for pid := range pids {
		if err := syscall.Kill(pid, 0); err == nil {
			t.Fatalf("supervised child %d still alive after the reap", pid)
		} else if !errors.Is(err, syscall.ESRCH) {
			t.Fatalf("check supervised child %d after the reap: %v", pid, err)
		}
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
