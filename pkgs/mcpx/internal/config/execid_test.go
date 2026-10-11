package config

import (
	"os"
	"path/filepath"
	"strings"
	"testing"
)

func writeExec(t *testing.T, path, body string) {
	t.Helper()
	if err := os.MkdirAll(filepath.Dir(path), 0o755); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(path, []byte(body), 0o755); err != nil {
		t.Fatal(err)
	}
}

func TestExecIdentityFollowsTheExecutableNotTheCommandName(t *testing.T) {
	dir := t.TempDir()
	bin := filepath.Join(dir, "bin", "tool")
	writeExec(t, bin, "#!/bin/sh\necho one\n")
	r := &Resolved{Server: &Server{Command: "tool", Env: map[string]string{"PATH": filepath.Join(dir, "bin")}}}

	first := r.ExecIdentity()
	if !strings.HasPrefix(first, bin+"|") {
		t.Fatalf("identity %q does not name the resolved file %s", first, bin)
	}
	if again := r.ExecIdentity(); again != first {
		t.Fatalf("identity moved with nothing changed: %q then %q", first, again)
	}
	writeExec(t, bin, "#!/bin/sh\necho two\n")
	if changed := r.ExecIdentity(); changed == first {
		t.Fatal("a rewritten executable kept its identity")
	}
}

func TestExecIdentityResolvesAgainstTheServersOwnPath(t *testing.T) {
	dir := t.TempDir()
	writeExec(t, filepath.Join(dir, "elsewhere", "tool"), "#!/bin/sh\n")
	r := &Resolved{Server: &Server{Command: "tool", Env: map[string]string{"PATH": filepath.Join(dir, "elsewhere")}}}
	if id := r.ExecIdentity(); !strings.Contains(id, filepath.Join(dir, "elsewhere", "tool")) {
		t.Fatalf("identity %q ignores the PATH the server is given", id)
	}
}

func TestExecIdentityOfAMissingCommandIsItsOwnIdentity(t *testing.T) {
	r := &Resolved{Server: &Server{Command: "no-such-mcpx-tool-anywhere", Env: map[string]string{"PATH": t.TempDir()}}}
	if id := r.ExecIdentity(); !strings.HasPrefix(id, "missing:") {
		t.Fatalf("identity %q for a command that does not resolve", id)
	}
}

func TestAServerThatIsNotAChildHasNoExecIdentity(t *testing.T) {
	r := &Resolved{Server: &Server{URL: "https://example.invalid/mcp"}}
	if id := r.ExecIdentity(); id != "" {
		t.Fatalf("a remote server has identity %q", id)
	}
	if r.ProcessID() != r.PoolID()+"/" {
		t.Fatalf("ProcessID %q is not PoolID plus the identity", r.ProcessID())
	}
}
