package daemon_test

import (
	"net"
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/developing-today/code/pkgs/mcpx/internal/daemon"
)

// longState returns a state directory whose in-state socket path exceeds
// the 100-byte sun_path ceiling, so the computed path relocates.
func longState(t *testing.T) string {
	t.Helper()
	base := t.TempDir()
	state := filepath.Join(base, strings.Repeat("a", 60), strings.Repeat("b", 60), "state")
	if err := os.MkdirAll(state, 0o700); err != nil {
		t.Fatal(err)
	}
	if len(filepath.Join(state, "daemon-abcdef123456.sock")) <= 100 {
		t.Fatalf("state path not long enough: %d", len(filepath.Join(state, "daemon-abcdef123456.sock")))
	}
	return state
}

// #1886 Approach C: a client whose own environment computes a different
// fallback path still finds the daemon through its info record.
func TestResolveThroughInfoRecordWithMismatchedRuntimeDir(t *testing.T) {
	t.Setenv("MCPX_SOCKET", "")
	state := longState(t)
	// Keep WriteInfo from mirroring the test record into the real
	// default state directory.
	t.Setenv("MCPX_STATE_DIR", state)

	// The daemon's environment: its fallback lands in runtime A.
	// Short directories, so the XDG branch itself is taken — with long
	// ones both sides would fall through to the same last-resort path
	// and the premise (different computed paths) would not hold.
	runtimeA, err := os.MkdirTemp("", "ra")
	if err != nil {
		t.Fatal(err)
	}
	defer os.RemoveAll(runtimeA)
	t.Setenv("XDG_RUNTIME_DIR", runtimeA)
	daemonPaths := daemon.Paths{State: state}.ForConfig("abcdef123456")
	realSocket := daemonPaths.Socket
	if realSocket == filepath.Join(state, "daemon-abcdef123456.sock") {
		t.Fatal("computed path did not relocate for the daemon")
	}
	// The daemon binds there and publishes its record.
	if err := os.MkdirAll(filepath.Dir(realSocket), 0o700); err != nil {
		t.Fatal(err)
	}
	ln, err := net.Listen("unix", realSocket)
	if err != nil {
		t.Fatal(err)
	}
	defer ln.Close()
	if err := daemonPaths.WriteInfo(daemon.Info{PID: os.Getpid(), Socket: realSocket}); err != nil {
		t.Fatal(err)
	}

	// The client's environment disagrees: a different XDG_RUNTIME_DIR
	// computes a different fallback path for the same key.
	runtimeB, err := os.MkdirTemp("", "rb")
	if err != nil {
		t.Fatal(err)
	}
	defer os.RemoveAll(runtimeB)
	t.Setenv("XDG_RUNTIME_DIR", runtimeB)
	clientPaths := daemon.Paths{State: state}.ForConfig("abcdef123456")
	if clientPaths.Socket == realSocket {
		t.Fatal("test premise broken: client computed the daemon's path")
	}
	if got := clientPaths.Resolve(); got != realSocket {
		t.Fatalf("Resolve = %q, want the record's %q", got, realSocket)
	}
	if got := clientPaths.ClientSocket(); got != realSocket {
		t.Fatalf("ClientSocket = %q, want %q", got, realSocket)
	}
}

// #1886 step 6: MCPX_SOCKET names one daemon for the process it was
// given to; it must not change the computed path or Resolve's answer
// for another key.
func TestMCPXSocketIgnoredForOtherKeys(t *testing.T) {
	t.Setenv("MCPX_SOCKET", "/tmp/someone-elses.sock")
	state := t.TempDir()
	p := daemon.Paths{State: state}.ForConfig("0123456789ab")
	if strings.Contains(p.Socket, "someone-elses") {
		t.Fatalf("computed path leaked the override: %q", p.Socket)
	}
	if got := p.Resolve(); strings.Contains(got, "someone-elses") {
		t.Fatalf("Resolve leaked the override: %q", got)
	}
	if got := p.ClientSocket(); got != "/tmp/someone-elses.sock" {
		t.Fatalf("ClientSocket = %q, want the override for its own daemon", got)
	}
}

// #1886 step 3: shutdown unlinks the record only when it is still ours.
func TestRemoveInfoOwnership(t *testing.T) {
	t.Setenv("MCPX_SOCKET", "")
	dir := t.TempDir()
	t.Setenv("MCPX_STATE_DIR", dir)
	p := daemon.Paths{State: dir, Info: filepath.Join(dir, "daemon-k.json")}

	other := daemon.Info{PID: os.Getpid() + 100000, Socket: filepath.Join(dir, "daemon-k.sock")}
	if err := p.WriteInfo(other); err != nil {
		t.Fatal(err)
	}
	p.RemoveInfo()
	if _, err := os.Stat(p.Info); err != nil {
		t.Fatalf("another daemon's record was removed: %v", err)
	}

	own := daemon.Info{PID: os.Getpid(), Socket: other.Socket}
	if err := p.WriteInfo(own); err != nil {
		t.Fatal(err)
	}
	p.RemoveInfo()
	if _, err := os.Stat(p.Info); !os.IsNotExist(err) {
		t.Fatalf("own record should be removed: %v", err)
	}
}

// Relocation is reported when the socket leaves the state directory.
func TestRelocated(t *testing.T) {
	t.Setenv("MCPX_SOCKET", "")
	state := longState(t)
	p := daemon.Paths{State: state}.ForConfig("abcdef123456")
	preferred, relocated := p.Relocated()
	if !relocated {
		t.Fatalf("expected relocation for long state path, socket %q", p.Socket)
	}
	if preferred != filepath.Join(state, "daemon-abcdef123456.sock") {
		t.Fatalf("preferred = %q", preferred)
	}
	short := daemon.Paths{State: t.TempDir()}.ForConfig("abcdef123456")
	if _, relocated := short.Relocated(); relocated {
		t.Fatal("short state path must not report relocation")
	}
}
