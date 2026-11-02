package pool_test

import (
	"context"
	"os"
	"path/filepath"
	"strconv"
	"testing"

	"github.com/developing-today/code/pkgs/mcpx/internal/config"
	"github.com/developing-today/code/pkgs/mcpx/internal/pool"
	"github.com/developing-today/code/pkgs/mcpx/internal/testsupport"
)

// Decision 0004 §3: the spawn-cwd decision as a pure function --
// config cwd beats the resolved scope root beats the daemon cwd.
func TestSpawnCwdDecision(t *testing.T) {
	for _, tc := range []struct {
		name       string
		configCwd  string
		root       string
		wantCwd    string
		wantSource string
	}{
		{"config beats root", "/cfg", "/root", "/cfg", "config"},
		{"config alone", "/cfg", "", "/cfg", "config"},
		{"root when no config cwd", "", "/root", "/root", "scope"},
		{"daemon fallback", "", "", "", "daemon"},
	} {
		t.Run(tc.name, func(t *testing.T) {
			cwd, source := pool.SpawnCwd(tc.configCwd, tc.root)
			if cwd != tc.wantCwd || source != tc.wantSource {
				t.Fatalf("SpawnCwd(%q, %q) = (%q, %q), want (%q, %q)",
					tc.configCwd, tc.root, cwd, source, tc.wantCwd, tc.wantSource)
			}
		})
	}
}

// Decision 0004 §2/§3/§6 end to end: an instance created with a
// resolved root keeps it in status and spawns in it when the server
// has no config cwd.
func TestInstanceRootAndSpawnCwd(t *testing.T) {
	bin := testsupport.FakeMCPBinary(t)
	p := pool.New(resolved(t, bin, &config.Extras{Sharing: config.SharingShared, Scope: config.ScopeWorktree}))
	defer p.Close()

	root := t.TempDir()
	if r, err := filepath.EvalSymlinks(root); err == nil {
		root = r
	}
	lease, err := p.Acquire(context.Background(), config.ResolvedScope{Key: "worktree:" + root, Root: root})
	if err != nil {
		t.Fatalf("acquire: %v", err)
	}
	defer lease.Release()

	st := p.Status()
	if len(st.Instances) != 1 {
		t.Fatalf("expected one instance, got %+v", st.Instances)
	}
	if st.Instances[0].Root != root {
		t.Fatalf("status root = %q, want %q", st.Instances[0].Root, root)
	}
	pid := st.Instances[0].PID
	if pid == 0 {
		t.Fatal("stdio instance should report a pid")
	}
	cwd, err := os.Readlink(filepath.Join("/proc", strconv.Itoa(pid), "cwd"))
	if err != nil {
		t.Skipf("cannot read child cwd: %v", err)
	}
	if cwd != root {
		t.Fatalf("child spawned in %q, want the resolved root %q", cwd, root)
	}
}
