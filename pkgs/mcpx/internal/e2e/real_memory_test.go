//go:build realsoftware

package e2e_test

import (
	"fmt"
	"path/filepath"
	"strings"
	"testing"

	"github.com/developing-today/code/pkgs/mcpx/internal/testsupport"
)

// Real mcp-server-memory: state created through one mcpx call must be
// visible to the next — the daemon has to keep the one real server
// process alive and stateful across calls, as it does for the fake.
//
// The server persists its graph to a memory.jsonl file, defaulting to
// a path inside its own install directory — which, for the flake-pinned
// build, is the read-only Nix store (EROFS on the first write; the
// subset's first catch). MEMORY_FILE_PATH is the server's own knob for
// it, so the config points it at scratch space.
func TestRealMemoryServerRoundTrip(t *testing.T) {
	server := testsupport.RealServer(t, "mcp-server-memory")
	memoryFile := filepath.Join(t.TempDir(), "memory.jsonl")
	cfg := fmt.Sprintf(`{
  "mcpServers": {
    "memory": { "command": %q, "env": { "MEMORY_FILE_PATH": %q }, "mcpx": { "sharing": "shared", "scope": "global", "description": "the real mcp-server-memory, pinned by the flake" } }
  }
}`, server, memoryFile)
	e := newEnv(t, cfg)
	e.setenv("MCPX_ELICIT_CONFIRM_DESTRUCTIVE=true", "MCPX_ELICIT_ASK_TIMEOUT=60s")
	e.run("ls")

	out := realConfirmedCall(t, e, "call", "memory.create_entities", `{"entities":[{"name":"realsoftware-probe","entityType":"probe","observations":["created by the mcpx realsoftware subset"]}]}`)
	if !strings.Contains(out, "realsoftware-probe") {
		t.Fatalf("create_entities output wrong:\n%s", out)
	}
	out = e.run("call", "memory.search_nodes", `{"query":"realsoftware-probe"}`)
	if !strings.Contains(out, "realsoftware-probe") {
		t.Fatalf("search_nodes did not find the entity created moments earlier:\n%s", out)
	}
}
