package pool_test

import (
	"context"
	"strings"
	"testing"

	"github.com/developing-today/code/pkgs/mcpx/internal/config"
	"github.com/developing-today/code/pkgs/mcpx/internal/pool"
	"github.com/developing-today/code/pkgs/mcpx/internal/testsupport"
)

// Decision 0004 §4 / #960 end to end: substitution happens at spawn on
// the effective definition, an empty root fails the spawn loudly, and
// the era identity follows the substituted definition.

func resolvedWith(t *testing.T, srv *config.Server) *config.Resolved {
	t.Helper()
	srv.Name = "fake"
	cfg := &config.Config{MCPServers: map[string]*config.Server{"fake": srv}}
	r, err := cfg.Resolve("fake")
	if err != nil {
		t.Fatalf("resolve: %v", err)
	}
	return r
}

func TestSpawnSubstitutesEnvPlaceholders(t *testing.T) {
	bin := testsupport.FakeMCPBinary(t)
	root := t.TempDir()
	caller := t.TempDir()
	for _, tc := range []struct {
		name string
		env  map[string]string
		want string
	}{
		{"root", map[string]string{"MCPX_REPORT_ENV": "SUB", "SUB": "root=${mcpx.root}"}, "root=" + root},
		{"cwd", map[string]string{"MCPX_REPORT_ENV": "SUB", "SUB": "cwd=${mcpx.cwd}"}, "cwd=" + caller},
	} {
		t.Run(tc.name, func(t *testing.T) {
			r := resolvedWith(t, &config.Server{Command: bin, Env: tc.env})
			p := pool.New(r)
			defer p.Close()
			rs := config.ResolvedScope{Key: "worktree:" + root, Root: root, Cwd: caller}
			raw, err := p.Call(context.Background(), rs, "echo", map[string]any{"message": "x"})
			if err != nil {
				t.Fatalf("call: %v", err)
			}
			if got := textOf(t, raw); got != tc.want {
				t.Fatalf("echo = %q, want %q (the child must see the substituted value)", got, tc.want)
			}
			// The stored config still shows the placeholder.
			if !strings.Contains(r.Env["SUB"], "${mcpx.") {
				t.Fatalf("stored config was substituted in place: %q", r.Env["SUB"])
			}
		})
	}
}

func TestSpawnFailsLoudlyOnEmptyRoot(t *testing.T) {
	bin := testsupport.FakeMCPBinary(t)
	r := resolvedWith(t, &config.Server{Command: bin, Args: []string{"--root", "${mcpx.root}"}})
	p := pool.New(r)
	defer p.Close()
	_, err := p.Acquire(context.Background(), config.ResolvedScope{Key: "global"})
	if err == nil || !strings.Contains(err.Error(), "${mcpx.root}") || !strings.Contains(err.Error(), `"fake"`) {
		t.Fatalf("acquire with no root: err = %v, want a loud failure naming server and placeholder", err)
	}
}

func TestEraIdentityFollowsSubstitutedDefinition(t *testing.T) {
	r := resolvedWith(t, &config.Server{Command: "x", Args: []string{"--root", "${mcpx.root}"}})
	a, err := r.Effective("/roots/a", "/caller")
	if err != nil {
		t.Fatal(err)
	}
	b, err := r.Effective("/roots/b", "/caller")
	if err != nil {
		t.Fatal(err)
	}
	if pool.Identity(a) == pool.Identity(b) {
		t.Fatal("instances differing only by substituted root must not share an era identity")
	}
	a2, err := r.Effective("/roots/a", "/caller")
	if err != nil {
		t.Fatal(err)
	}
	if pool.Identity(a) != pool.Identity(a2) {
		t.Fatal("same substituted definition must keep the same era identity")
	}
}
