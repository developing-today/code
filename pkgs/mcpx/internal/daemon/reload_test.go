package daemon

import (
	"context"
	"os"
	"path/filepath"
	"syscall"
	"testing"
	"time"

	"github.com/developing-today/code/pkgs/mcpx/internal/config"
	"github.com/developing-today/code/pkgs/mcpx/internal/pool"
	"github.com/developing-today/code/pkgs/mcpx/internal/testsupport"
)

// childPID starts (or finds) the global instance of p and reports its pid.
func childPID(t *testing.T, p *pool.Pool) int {
	t.Helper()
	lease, err := p.Acquire(context.Background(), "global")
	if err != nil {
		t.Fatalf("acquire %s: %v", p.Name(), err)
	}
	lease.Release()
	for _, in := range p.Status().Instances {
		if in.Key == "global" && in.PID > 0 {
			return in.PID
		}
	}
	t.Fatalf("no global instance for %s", p.Name())
	return 0
}

func nsExtras(ns string) *config.Extras {
	if ns == "" {
		return nil
	}
	return &config.Extras{Namespace: ns}
}

func waitGone(t *testing.T, pid int) {
	t.Helper()
	// stdio shutdown is stdinGrace + termGrace + killWait in the worst case
	deadline := time.Now().Add(15 * time.Second)
	for syscall.Kill(pid, 0) == nil {
		if time.Now().After(deadline) {
			t.Fatalf("pid %d still running after reload", pid)
		}
		time.Sleep(20 * time.Millisecond)
	}
}

func TestStampConfigNoticesARepointedSymlinkWithSameSizeAndMtime(t *testing.T) {
	// Nix store files all carry mtime 1, so a re-pointed config link whose
	// new target has the same byte length as the old one is invisible to a
	// stamp built from the followed file's mtime+size alone (#368). The
	// stamp must notice the link itself moved.
	dir := t.TempDir()
	stampTime := time.Unix(1, 0)
	genA := filepath.Join(t.TempDir(), "config.json")
	genB := filepath.Join(t.TempDir(), "config.json")
	// Same length, different content.
	for p, body := range map[string]string{genA: `{"a":1}`, genB: `{"b":2}`} {
		if err := os.WriteFile(p, []byte(body), 0o444); err != nil {
			t.Fatal(err)
		}
		if err := os.Chtimes(p, stampTime, stampTime); err != nil {
			t.Fatal(err)
		}
	}
	cfg := filepath.Join(dir, "config.json")
	repoint := func(target string) {
		t.Helper()
		_ = os.Remove(cfg)
		if err := os.Symlink(target, cfg); err != nil {
			t.Fatal(err)
		}
	}
	repoint(genA)
	before := stampConfig([]string{cfg})
	if again := stampConfig([]string{cfg}); again != before {
		t.Fatalf("stamp is not stable when nothing moved: %q vs %q", before, again)
	}
	repoint(genB)
	if after := stampConfig([]string{cfg}); after == before {
		t.Errorf("stamp did not notice the config link was re-pointed at a same-size, same-mtime file")
	}
}

func TestReloadKeepsUnchangedChildAndReplacesChangedOne(t *testing.T) {
	fake := testsupport.FakeMCPBinary(t)
	dir := t.TempDir()
	// b starts with args that differ from a: identical definitions share a pool.
	cfgOf := func(bArgs []string) *config.Config {
		return &config.Config{MCPServers: map[string]*config.Server{
			"a": {Name: "a", Command: fake},
			"b": {Name: "b", Command: fake, Args: bArgs},
		}}
	}
	r, err := NewRegistry(cfgOf([]string{"--initial"}), Paths{State: dir, Cache: dir}, nil)
	if err != nil {
		t.Fatal(err)
	}
	defer r.Close()

	pa, _ := r.Pool("a")
	pb, _ := r.Pool("b")
	pidA, pidB := childPID(t, pa), childPID(t, pb)

	if _, _, err := r.Reload(cfgOf([]string{"--changed"})); err != nil {
		t.Fatal(err)
	}
	pa2, _ := r.Pool("a")
	pb2, _ := r.Pool("b")
	if pa2 != pa {
		t.Fatalf("unchanged server a got a new pool")
	}
	if got := childPID(t, pa2); got != pidA {
		t.Fatalf("unchanged server a restarted: pid %d -> %d", pidA, got)
	}
	if pb2 == pb {
		t.Fatalf("changed server b kept its old pool")
	}
	if got := childPID(t, pb2); got == pidB {
		t.Fatalf("changed server b kept its old child pid %d", pidB)
	}
	waitGone(t, pidB)
}

func TestReloadRenamedNamespaceKeepsChildAndResolvesNewName(t *testing.T) {
	fake := testsupport.FakeMCPBinary(t)
	dir := t.TempDir()
	cfgOf := func(ns string) *config.Config {
		return &config.Config{Path: filepath.Join(dir, "mcp.json"), MCPServers: map[string]*config.Server{
			"a": {Name: "a", Command: fake, Mcpx: nsExtras(ns)},
		}}
	}
	r, err := NewRegistry(cfgOf("alpha"), Paths{State: dir, Cache: dir}, nil)
	if err != nil {
		t.Fatal(err)
	}
	defer r.Close()

	p, _ := r.Pool("alpha")
	pid := childPID(t, p)

	if _, _, err := r.Reload(cfgOf("beta")); err != nil {
		t.Fatal(err)
	}
	got, ok := r.Pool("beta")
	if !ok {
		t.Fatal(`namespace "beta" does not resolve after reload`)
	}
	if got != p {
		t.Fatal("renaming a namespace replaced an unchanged server's pool")
	}
	if childPID(t, got) != pid {
		t.Fatalf("renaming a namespace restarted the child")
	}
}
