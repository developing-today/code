package cli

import (
	"os"
	"path/filepath"
	"testing"

	"github.com/developing-today/code/pkgs/mcpx/internal/daemon"
)

// Codex round 2 on PR #2561: Socket() must follow the same re-resolution
// as the transport. A client built while a stale record names socket A
// exports A to scripts as MCPX_SOCKET even after EnsureDaemon's
// replacement daemon republishes the record with socket B.
func TestSocketReResolvesAfterRecordChanges(t *testing.T) {
	t.Setenv("MCPX_SOCKET", "")
	dir := t.TempDir()
	paths := daemon.PathsAt(filepath.Join(dir, "state"), filepath.Join(dir, "cache"))
	if err := paths.EnsureDirs(); err != nil {
		t.Fatal(err)
	}
	sockA := filepath.Join(dir, "a.sock")
	sockB := filepath.Join(dir, "b.sock")
	if err := paths.WriteInfo(daemon.Info{PID: os.Getpid(), Socket: sockA}); err != nil {
		t.Fatal(err)
	}
	c := NewClient(paths, "")
	if got := c.Socket(); got != sockA {
		t.Fatalf("Socket() = %q, want record socket %q", got, sockA)
	}
	if err := paths.WriteInfo(daemon.Info{PID: os.Getpid(), Socket: sockB}); err != nil {
		t.Fatal(err)
	}
	if got := c.Socket(); got != sockB {
		t.Fatalf("Socket() after record republish = %q, want %q", got, sockB)
	}
}

// Codex round 2 on PR #2561: a record-targeted client (stop --all,
// CmdDaemons) is built with an explicit unix:// endpoint; MCPX_SOCKET
// names the caller's own daemon (#1886 step 6) and must not retarget it.
func TestExplicitUnixEndpointIgnoresOverride(t *testing.T) {
	dir := t.TempDir()
	paths := daemon.PathsAt(filepath.Join(dir, "state"), filepath.Join(dir, "cache"))
	record := filepath.Join(dir, "recorded.sock")
	t.Setenv("MCPX_SOCKET", filepath.Join(dir, "override.sock"))
	c := NewClientAt(paths, "", "unix://"+record)
	if got := c.Socket(); got != record {
		t.Fatalf("Socket() = %q, want explicit endpoint %q", got, record)
	}
}
