//go:build realsoftware

package e2e_test

import (
	"encoding/json"
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/developing-today/code/pkgs/mcpx/internal/testsupport"
)

// Real mcp-server-filesystem, scoped to one scratch directory by its own
// allowed-directories argument: a write through mcpx must land on the
// real disk, and a read through mcpx must see it.
func TestRealFilesystemServerRoundTrip(t *testing.T) {
	server := testsupport.RealServer(t, "mcp-server-filesystem")
	dir := t.TempDir()
	e := newEnv(t, realConfig(server, "realfs", "the real mcp-server-filesystem, pinned by the flake", dir))
	e.setenv("MCPX_ELICIT_CONFIRM_DESTRUCTIVE=true", "MCPX_ELICIT_ASK_TIMEOUT=60s")
	e.run("ls")

	probe := filepath.Join(dir, "probe.txt")
	realConfirmedCall(t, e, "call", "realfs.write_file", `{"path":`+jsonString(probe)+`,"content":"real software, pinned by the flake"}`)
	data, err := os.ReadFile(probe)
	if err != nil {
		t.Fatalf("write_file through mcpx did not reach the real disk: %v", err)
	}
	if string(data) != "real software, pinned by the flake" {
		t.Fatalf("written content wrong: %q", data)
	}
	out := e.run("call", "realfs.read_file", `{"path":`+jsonString(probe)+`}`)
	if !strings.Contains(out, "real software, pinned by the flake") {
		t.Fatalf("read_file output wrong:\n%s", out)
	}
	out = e.run("call", "realfs.list_directory", `{"path":`+jsonString(dir)+`}`)
	if !strings.Contains(out, "probe.txt") {
		t.Fatalf("list_directory output wrong:\n%s", out)
	}
}

func jsonString(s string) string {
	b, err := json.Marshal(s)
	if err != nil {
		panic(err)
	}
	return string(b)
}
