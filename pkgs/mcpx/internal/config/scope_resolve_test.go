package config_test

import (
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"testing"

	"github.com/developing-today/code/pkgs/mcpx/internal/config"
)

// Decision 0004 §1: Resolve returns the key Key returns, plus the root
// the scope resolved to. These tests pin the root semantics per scope.

func canon(t *testing.T, p string) string {
	t.Helper()
	r, err := filepath.EvalSymlinks(p)
	if err != nil {
		return filepath.Clean(p)
	}
	return r
}

func TestResolveWorktreeRootIsCheckoutTopLevel(t *testing.T) {
	root := gitRepo(t)
	rs := config.ScopeWorktree.Resolve(config.CallContext{Cwd: root, CallID: "c1"})
	if rs.Degraded != "" {
		t.Fatalf("unexpected degradation: %s", rs.Degraded)
	}
	if !strings.HasPrefix(rs.Key, "worktree:") {
		t.Fatalf("expected a worktree key, got %q", rs.Key)
	}
	if rs.Root != canon(t, root) {
		t.Fatalf("worktree root = %q, want the checkout top level %q", rs.Root, canon(t, root))
	}
}

func TestResolveRepoRootIsMainWorktreeFromSubdir(t *testing.T) {
	root := gitRepo(t)
	sub := filepath.Join(root, "a", "b")
	if err := os.MkdirAll(sub, 0o755); err != nil {
		t.Fatal(err)
	}
	rs := config.ScopeRepo.Resolve(config.CallContext{Cwd: sub, CallID: "c1"})
	if !strings.HasPrefix(rs.Key, "repo:") {
		t.Fatalf("expected a repo key, got %q", rs.Key)
	}
	if rs.Root != canon(t, root) {
		t.Fatalf("repo root = %q, want the main worktree %q", rs.Root, canon(t, root))
	}
}

func TestResolveRepoRootFromLinkedWorktreeIsMainWorktree(t *testing.T) {
	root := gitRepo(t)
	linked := filepath.Join(t.TempDir(), "linked")
	cmd := exec.Command("git", "worktree", "add", "-q", linked)
	cmd.Dir = root
	if out, err := cmd.CombinedOutput(); err != nil {
		t.Skipf("git worktree add failed: %v\n%s", err, out)
	}
	// The linked worktree's own top level roots a worktree scope...
	if rs := config.ScopeWorktree.Resolve(config.CallContext{Cwd: linked, CallID: "c1"}); rs.Root != canon(t, linked) {
		t.Fatalf("worktree root from linked worktree = %q, want %q", rs.Root, canon(t, linked))
	}
	// ...while a repo scope roots at the main worktree, the parent of
	// the shared common dir.
	if rs := config.ScopeRepo.Resolve(config.CallContext{Cwd: linked, CallID: "c2"}); rs.Root != canon(t, root) {
		t.Fatalf("repo root from linked worktree = %q, want main worktree %q", rs.Root, canon(t, root))
	}
}

func TestResolveCwdRootIsCallerCwd(t *testing.T) {
	dir := t.TempDir()
	rs := config.ScopeCwd.Resolve(config.CallContext{Cwd: dir, CallID: "c1"})
	if rs.Key != "cwd:"+rs.Root {
		t.Fatalf("cwd root must be the key's directory: key %q root %q", rs.Key, rs.Root)
	}
	if rs.Root != canon(t, dir) {
		t.Fatalf("cwd root = %q, want %q", rs.Root, canon(t, dir))
	}
}

func TestResolveDegradedRepoRootIsCallerCwd(t *testing.T) {
	dir := t.TempDir()
	cmd := exec.Command("git", "rev-parse", "--git-common-dir")
	cmd.Dir = dir
	if err := cmd.Run(); err == nil {
		t.Skip("temp dir is inside a git repository")
	}
	rs := config.ScopeRepo.Resolve(config.CallContext{Cwd: dir, CallID: "c1"})
	if rs.Degraded == "" {
		t.Fatal("falling back should be reported")
	}
	if !strings.HasPrefix(rs.Key, "cwd:") {
		t.Fatalf("expected a degraded cwd key, got %q", rs.Key)
	}
	if rs.Root != canon(t, dir) {
		t.Fatalf("degraded root = %q, want the caller cwd %q", rs.Root, canon(t, dir))
	}
}

func TestResolveRootlessScopesHaveNoRoot(t *testing.T) {
	cc := ctx()
	for _, s := range []config.Scope{config.ScopeGlobal, config.ScopeSession, config.ScopeParentSession, config.ScopePid, config.ScopeCall} {
		if rs := s.Resolve(cc); rs.Root != "" {
			t.Errorf("scope %q resolved root %q, want none", s, rs.Root)
		}
	}
}

func TestKeyIsResolveKey(t *testing.T) {
	repo := gitRepo(t)
	for _, s := range []config.Scope{config.ScopeGlobal, config.ScopeCwd, config.ScopeRepo, config.ScopeWorktree, config.ScopeSession, config.ScopeParentSession, config.ScopePid, config.ScopeCall} {
		cc := config.CallContext{Cwd: repo, SessionID: "s1", ParentSessionID: "p1", PID: 4242, CallID: "c1"}
		key, deg := s.Key(cc)
		rs := s.Resolve(cc)
		if key != rs.Key || deg != rs.Degraded {
			t.Errorf("scope %q: Key = (%q, %q), Resolve = (%q, %q)", s, key, deg, rs.Key, rs.Degraded)
		}
	}
}
