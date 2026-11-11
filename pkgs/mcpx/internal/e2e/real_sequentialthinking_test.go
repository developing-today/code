//go:build realsoftware

package e2e_test

import (
	"strings"
	"testing"

	"github.com/developing-today/code/pkgs/mcpx/internal/testsupport"
)

// Real mcp-server-sequential-thinking (#6903): tools/list through mcpx
// plus one real sequentialthinking call — the server keeps its thought
// history in-process, so a working answer proves the pinned binary
// actually ran, not merely that it exists in the buildEnv.
func TestRealSequentialThinkingServerTools(t *testing.T) {
	server := testsupport.RealServer(t, "mcp-server-sequential-thinking")
	e := newEnv(t, realConfig(server, "thinking", "the real mcp-server-sequential-thinking, pinned by the flake"))

	out := e.run("ls")
	if !strings.Contains(out, "thinking") {
		t.Fatalf("namespace missing from ls:\n%s", out)
	}
	out = e.run("types", "thinking")
	if !strings.Contains(out, "sequentialthinking") {
		t.Fatalf("types output missing real tool %q:\n%s", "sequentialthinking", out)
	}
	out = e.run("call", "thinking.sequentialthinking", `{"thought":"realsoftware probe thought","nextThoughtNeeded":false,"thoughtNumber":1,"totalThoughts":1}`)
	if !strings.Contains(out, "thoughtNumber") && !strings.Contains(out, "nextThoughtNeeded") {
		t.Fatalf("sequentialthinking output wrong:\n%s", out)
	}
}
