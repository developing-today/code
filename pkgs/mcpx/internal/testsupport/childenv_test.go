package testsupport

import (
	"path/filepath"
	"strings"
	"testing"
)

func TestChildEnvIsAnAllowlist(t *testing.T) {
	dir := filepath.Join("/tests", "case")
	got := ChildEnv([]string{
		"PATH=/bin",
		"TMPDIR=/tmp",
		"LANG=C.UTF-8",
		"LC_ALL=C",
		"TERM=xterm-256color",
		"SYSTEMROOT=C:\\Windows",
		"DENO_DIR=/cache/deno",
		"BUN_INSTALL_CACHE_DIR=/cache/bun",
		"NPM_CONFIG_CACHE=/cache/npm",
		"HOME=/caller/home",
		"MCPX_TRACE=1",
		"MCPX_DAEMON_ENDPOINT=http://127.0.0.1:9",
		"MCPX_SOCKET=/caller/daemon.sock",
		"XDG_CONFIG_HOME=/caller/config",
		"XDG_DATA_HOME=/caller/data",
		"XDG_STATE_HOME=/caller/state",
		"XDG_CACHE_HOME=/caller/cache",
		"XDG_RUNTIME_DIR=/caller/run",
		"OPENCODE_SESSION_ID=caller-session",
		"HTTP_PROXY=http://proxy:9",
		"NODE_OPTIONS=--require caller.js",
		"AWS_ACCESS_KEY_ID=not-for-tests",
		"GITHUB_TOKEN=not-for-tests",
	}, dir, "HOME="+filepath.Join(dir, "home"), "MCPX_CONFIG="+filepath.Join(dir, "config"))

	env := envMap(got)
	for _, name := range []string{
		"PATH", "TMPDIR", "LANG", "LC_ALL", "TERM", "SYSTEMROOT",
		"DENO_DIR", "BUN_INSTALL_CACHE_DIR", "NPM_CONFIG_CACHE",
		"HOME", "MCPX_CONFIG",
	} {
		if env[name] == "" {
			t.Errorf("allowlisted or explicitly set variable %s is missing", name)
		}
	}
	for _, name := range []string{
		"MCPX_TRACE", "MCPX_DAEMON_ENDPOINT", "MCPX_SOCKET",
		"OPENCODE_SESSION_ID", "HTTP_PROXY", "NODE_OPTIONS",
		"AWS_ACCESS_KEY_ID", "GITHUB_TOKEN",
	} {
		if _, ok := env[name]; ok {
			t.Errorf("caller variable %s reached the child", name)
		}
	}
	for name, want := range map[string]string{
		"XDG_CONFIG_HOME": filepath.Join(dir, ".config"),
		"XDG_DATA_HOME":   filepath.Join(dir, ".local", "share"),
		"XDG_STATE_HOME":  filepath.Join(dir, ".local", "state"),
		"XDG_CACHE_HOME":  filepath.Join(dir, ".cache"),
		"XDG_RUNTIME_DIR": filepath.Join(dir, "run"),
	} {
		if env[name] != want {
			t.Errorf("%s=%q, want %q", name, env[name], want)
		}
	}
}

func TestChildEnvExtraWinsOverParent(t *testing.T) {
	got := ChildEnv([]string{"PATH=/bin", "TMPDIR=/caller/tmp"}, "/tests/case",
		"TMPDIR=/tests/tmp", "MCPX_STATE_DIR=/tests/state")
	env := envMap(got)
	if env["TMPDIR"] != "/tests/tmp" {
		t.Errorf("TMPDIR=%q, want extra value", env["TMPDIR"])
	}
	if env["MCPX_STATE_DIR"] != "/tests/state" {
		t.Errorf("MCPX_STATE_DIR=%q, want extra value", env["MCPX_STATE_DIR"])
	}
}

func envMap(env []string) map[string]string {
	out := make(map[string]string, len(env))
	for _, kv := range env {
		name, value, ok := strings.Cut(kv, "=")
		if ok {
			out[name] = value
		}
	}
	return out
}
