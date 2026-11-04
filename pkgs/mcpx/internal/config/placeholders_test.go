package config_test

import (
	"strings"
	"testing"

	"github.com/developing-today/code/pkgs/mcpx/internal/config"
)

// Decision 0004 §4 / #960: placeholder names are validated at config
// load, and substitution happens only on the per-instance effective
// definition.

func resolveServer(t *testing.T, srv *config.Server) (*config.Resolved, error) {
	t.Helper()
	srv.Name = "s"
	cfg := &config.Config{MCPServers: map[string]*config.Server{"s": srv}}
	return cfg.Resolve("s")
}

func TestPlaceholderValidationAtResolve(t *testing.T) {
	for _, tc := range []struct {
		name    string
		srv     *config.Server
		wantErr string
	}{
		{"known placeholders pass", &config.Server{Command: "x", Args: []string{"--root", "${mcpx.root}"}, Env: map[string]string{"W": "${mcpx.cwd}"}, Cwd: "${mcpx.root}/sub"}, ""},
		{"other namespaces pass through", &config.Server{Command: "x", Args: []string{"${HOME}/bin", "${env.FOO}", "$mcpx.root"}}, ""},
		{"unknown name in args", &config.Server{Command: "x", Args: []string{"${mcpx.home}"}}, "unknown placeholder ${mcpx.home}"},
		{"unknown name in env", &config.Server{Command: "x", Env: map[string]string{"W": "${mcpx.workspace}"}}, "unknown placeholder ${mcpx.workspace}"},
		{"unknown name in cwd", &config.Server{Command: "x", Cwd: "${mcpx.base}"}, "unknown placeholder ${mcpx.base}"},
		{"unterminated in args", &config.Server{Command: "x", Args: []string{"${mcpx.root"}}, "malformed placeholder"},
		{"empty name", &config.Server{Command: "x", Args: []string{"${mcpx.}"}}, "malformed placeholder"},
		{"bad characters in name", &config.Server{Command: "x", Args: []string{"${mcpx.ro ot}"}}, "malformed placeholder"},
	} {
		t.Run(tc.name, func(t *testing.T) {
			_, err := resolveServer(t, tc.srv)
			if tc.wantErr == "" {
				if err != nil {
					t.Fatalf("Resolve: %v", err)
				}
				return
			}
			if err == nil || !strings.Contains(err.Error(), tc.wantErr) {
				t.Fatalf("Resolve error = %v, want containing %q", err, tc.wantErr)
			}
			if !strings.Contains(err.Error(), `server "s"`) {
				t.Fatalf("error should name the server: %v", err)
			}
		})
	}
}

func TestEffectiveSubstitutesArgsEnvCwd(t *testing.T) {
	r, err := resolveServer(t, &config.Server{
		Command: "x",
		Args:    []string{"--root", "${mcpx.root}", "--both", "${mcpx.cwd}:${mcpx.root}", "plain"},
		Env:     map[string]string{"ROOT": "${mcpx.root}", "KEEP": "${HOME}"},
		Cwd:     "${mcpx.root}/sub",
	})
	if err != nil {
		t.Fatalf("resolve: %v", err)
	}
	eff, err := r.Effective("/work/root", "/caller/dir")
	if err != nil {
		t.Fatalf("Effective: %v", err)
	}
	if eff.Args[1] != "/work/root" || eff.Args[3] != "/caller/dir:/work/root" || eff.Args[4] != "plain" {
		t.Fatalf("args not substituted: %v", eff.Args)
	}
	if eff.Env["ROOT"] != "/work/root" || eff.Env["KEEP"] != "${HOME}" {
		t.Fatalf("env not substituted: %v", eff.Env)
	}
	if eff.Cwd != "/work/root/sub" {
		t.Fatalf("cwd not substituted: %q", eff.Cwd)
	}
	// The stored config keeps its placeholders, untouched.
	if r.Args[1] != "${mcpx.root}" || r.Env["ROOT"] != "${mcpx.root}" || r.Cwd != "${mcpx.root}/sub" {
		t.Fatalf("stored config was mutated: args=%v env=%v cwd=%q", r.Args, r.Env, r.Cwd)
	}
}

func TestEffectiveEmptyRootOrCwdFailsLoudly(t *testing.T) {
	r, err := resolveServer(t, &config.Server{Command: "x", Args: []string{"${mcpx.root}"}})
	if err != nil {
		t.Fatalf("resolve: %v", err)
	}
	if _, err := r.Effective("", "/caller"); err == nil || !strings.Contains(err.Error(), "${mcpx.root}") {
		t.Fatalf("empty root: err = %v", err)
	}
	r2, err := resolveServer(t, &config.Server{Command: "x", Env: map[string]string{"W": "${mcpx.cwd}"}})
	if err != nil {
		t.Fatalf("resolve: %v", err)
	}
	if _, err := r2.Effective("/root", ""); err == nil || !strings.Contains(err.Error(), "${mcpx.cwd}") {
		t.Fatalf("empty caller cwd: err = %v", err)
	}
	// No placeholders: empty root/cwd is fine and the copy is equal.
	r3, err := resolveServer(t, &config.Server{Command: "x", Args: []string{"plain"}})
	if err != nil {
		t.Fatalf("resolve: %v", err)
	}
	eff, err := r3.Effective("", "")
	if err != nil || eff.Args[0] != "plain" {
		t.Fatalf("no-placeholder Effective: eff=%v err=%v", eff, err)
	}
}
