package mcpclient_test

import (
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/developing-today/code/pkgs/mcpx/internal/mcpclient"
)

// Darwin allows 104 bytes of sun_path including the NUL; Linux allows
// 108. Supervisor sockets must fit the smaller figure everywhere,
// even under the deep TMPDIRs macOS builders and the nix darwin check
// use (#15407: scoped sockets failed dial with EINVAL past the limit).
func TestSupervisorSocketPathsStayWithinDarwinLimit(t *testing.T) {
	// Simulate a deep darwin TMPDIR without creating it: the socket
	// dir computation is pure path arithmetic.
	t.Setenv("TMPDIR", "/var/folders/zz/zyzxv9s22db9dv8gh8hl_6r0000gn/T/nix-build-mcpx-check.drv-0")
	deepState := filepath.Join(os.TempDir(), "mcpx-test-state-"+strings.Repeat("s", 40))

	for _, instance := range []string{"", "global", "cwd:/some/deep/worktree/path", "session:another-session-id"} {
		sock := mcpclient.SupervisorSocketPathForInstance(deepState, "demo", instance)
		if len(sock) > 104 {
			t.Fatalf("instance %q socket %q is %d bytes, over darwin's 104-byte sun_path limit", instance, sock, len(sock))
		}
		if sock != filepath.Join(mcpclient.SupervisorSocketDir(deepState), filepath.Base(sock)) {
			t.Fatalf("socket %q is not inside the socket dir %q", sock, mcpclient.SupervisorSocketDir(deepState))
		}
	}
}

// Scoped instance sockets of one server must stay distinct after the
// #15407 shortening, and the primary instance must keep the
// historical name-only socket supervisors from before the fix already
// listen on.
func TestSupervisorSocketShorteningKeepsInstancesDistinct(t *testing.T) {
	dir := t.TempDir()
	legacy := mcpclient.SupervisorSocketPath(dir, "era")
	if got := mcpclient.SupervisorSocketPathForInstance(dir, "era", ""); got != legacy {
		t.Fatalf("empty instance socket = %s, want the legacy name-only socket %s", got, legacy)
	}
	a := mcpclient.SupervisorSocketPathForInstance(dir, "era", "cwd:/a")
	b := mcpclient.SupervisorSocketPathForInstance(dir, "era", "cwd:/b")
	if a == b {
		t.Fatalf("distinct instances share socket %s", a)
	}
	if len(a) > 104 || len(b) > 104 {
		t.Fatalf("scoped sockets exceed the darwin limit: %q %q", a, b)
	}
}
