package pool_test

import (
	"context"
	"path/filepath"
	"testing"
	"time"

	"github.com/developing-today/code/pkgs/mcpx/internal/config"
	"github.com/developing-today/code/pkgs/mcpx/internal/pool"
	"github.com/developing-today/code/pkgs/mcpx/internal/testsupport"
)

// Regression tests for the Codex review findings on PR #5135 (#960):
// the creating caller's cwd must survive pool-driven replacement, the
// exec watch must monitor the effective (substituted) definition, and
// schema discovery must be able to run on a caller's scope.

// Finding 1: a non-lazy Restart rebuilds instances from the pool's own
// records; without the stored caller cwd the replacement's Effective
// call fails (or substitutes an empty cwd) for ${mcpx.cwd} definitions.
func TestRestartPreservesCallerCwd(t *testing.T) {
	bin := testsupport.FakeMCPBinary(t)
	root := t.TempDir()
	caller := t.TempDir()
	r := resolvedWith(t, &config.Server{Command: bin, Env: map[string]string{
		"MCPX_REPORT_ENV": "SUB", "SUB": "cwd=${mcpx.cwd}",
	}})
	p := pool.New(r)
	defer p.Close()
	rs := config.ResolvedScope{Key: "worktree:" + root, Root: root, Cwd: caller}
	echo := func() string {
		raw, err := p.Call(context.Background(), rs, "echo", map[string]any{"message": "x"})
		if err != nil {
			t.Fatalf("call: %v", err)
		}
		return textOf(t, raw)
	}
	if got := echo(); got != "cwd="+caller {
		t.Fatalf("before restart: echo = %q, want %q", got, "cwd="+caller)
	}
	res := p.Restart(context.Background(), false)
	if len(res.Failed) > 0 || len(res.Started) != 1 {
		t.Fatalf("restart = %+v, want exactly one started and no failures", res)
	}
	if got := echo(); got != "cwd="+caller {
		t.Fatalf("after restart: echo = %q, want %q (the replacement must keep the creating caller's cwd)", got, "cwd="+caller)
	}
}

// Findings 1+2: with a config cwd of "${mcpx.root}", the exec watch
// must notice a changed executable (it previously stamped the literal
// placeholder directory and reported the executable missing forever),
// and the replacement it starts must still substitute the creating
// caller's cwd.
func TestExecWatchReplacesPlaceholderCwdChild(t *testing.T) {
	setExecWatch(t, execTick, 10*time.Second)
	fake := testsupport.FakeMCPBinary(t)
	root := t.TempDir()
	caller := t.TempDir()
	bin := filepath.Join(root, "server")
	writeExecWrapper(t, fake, bin, "initial")
	r := resolvedWith(t, &config.Server{
		Command: "./server",
		Cwd:     "${mcpx.root}",
		Env:     map[string]string{"MCPX_REPORT_ENV": "SUB", "SUB": "cwd=${mcpx.cwd}"},
	})
	if !r.ExecUsesScopeRoot() {
		t.Fatal("a config cwd referencing ${mcpx.root} must route exec monitoring per-instance")
	}
	p := pool.New(r)
	defer p.Close()
	rs := config.ResolvedScope{Key: "worktree:" + root, Root: root, Cwd: caller}
	lease, err := p.Acquire(context.Background(), rs)
	if err != nil {
		t.Fatalf("acquire: %v", err)
	}
	pids := childPIDs(p)
	if len(pids) != 1 {
		t.Fatalf("children = %v, want exactly one", pids)
	}
	old := pids[0]
	lease.Release()

	writeExecWrapper(t, fake, bin, "changed")
	awaitReplacement(t, p, old)

	raw, err := p.Call(context.Background(), rs, "echo", map[string]any{"message": "x"})
	if err != nil {
		t.Fatalf("call after replacement: %v", err)
	}
	if got := textOf(t, raw); got != "cwd="+caller {
		t.Fatalf("after exec replacement: echo = %q, want %q", got, "cwd="+caller)
	}
}

// Finding 3: an unscoped schema refresh cannot start a definition
// that references ${mcpx.root}; a caller-scoped refresh lists from an
// instance started with the caller's root and succeeds.
func TestRefreshSchemasForCallerScope(t *testing.T) {
	bin := testsupport.FakeMCPBinary(t)
	root := t.TempDir()
	caller := t.TempDir()
	r := resolvedWith(t, &config.Server{Command: bin, Env: map[string]string{
		"MCPX_REPORT_ENV": "SUB", "SUB": "root=${mcpx.root}",
	}})
	p := pool.New(r)
	defer p.Close()
	if !p.CallerScoped() {
		t.Fatal("a definition referencing ${mcpx.root} must report CallerScoped")
	}
	if _, _, err := p.RefreshSchemas(context.Background()); err == nil {
		t.Fatal("unscoped refresh of a caller-scoped definition must not manufacture a rootless start")
	}
	tools, _, err := p.RefreshSchemasFor(context.Background(), config.ResolvedScope{
		Key: "worktree:" + root, Root: root, Cwd: caller,
	})
	if err != nil {
		t.Fatalf("caller-scoped refresh: %v", err)
	}
	if len(tools) == 0 {
		t.Fatal("caller-scoped refresh returned no tools")
	}
}
