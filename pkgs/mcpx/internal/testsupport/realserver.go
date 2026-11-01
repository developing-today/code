package testsupport

import (
	"os"
	"path/filepath"
	"strings"
	"testing"
)

// RealServer resolves the named real (third-party) MCP server binary from
// the directory named by MCPX_REAL_SERVERS_DIR and returns its path.
//
// That directory is built by the flake — `nix build .#mcpx-real-servers` —
// so the server versions the real-software subset runs against are pinned
// in exactly one place (#1990). Servers are looked up as <dir>/bin/<name>
// (the buildEnv layout) and then <dir>/<name> (a hand-assembled directory).
//
// A missing server skips the calling test, naming the server — unless the
// require mechanism demands it: when MCPX_TEST_REQUIRE (the comma list
// #1983 step 1 formalises; PR for it in flight) names this server, the
// subset label "realsoftware", or "real", a missing server FAILS instead.
// That is the #35 lesson: a silently skipped real-server test reads as a
// green run. Until #1983 lands this helper reads the same variable with
// the same spelling, so the flip is automatic when it does.
func RealServer(t *testing.T, name string) string {
	t.Helper()
	dir := os.Getenv("MCPX_REAL_SERVERS_DIR")
	if dir == "" {
		realMissing(t, name, "MCPX_REAL_SERVERS_DIR is not set (build the pinned servers with: nix build .#mcpx-real-servers)")
	}
	path, ok := realServerPath(dir, name)
	if !ok {
		realMissing(t, name, "no executable "+name+" under "+dir+" (stale build? rebuild: nix build .#mcpx-real-servers)")
	}
	t.Logf("realserver: %s -> %s (version pinned by the flake: nix build .#mcpx-real-servers)", name, path)
	return path
}

func realMissing(t *testing.T, name, why string) {
	t.Helper()
	if realRequired(os.Getenv("MCPX_TEST_REQUIRE"), name) {
		t.Fatalf("realserver: %s is required by MCPX_TEST_REQUIRE but unavailable: %s", name, why)
	}
	t.Skipf("realserver: %s unavailable: %s", name, why)
}

// realServerPath finds the executable for name under a real-servers
// directory, in buildEnv layout first, then flat.
func realServerPath(dir, name string) (string, bool) {
	for _, cand := range []string{
		filepath.Join(dir, "bin", name),
		filepath.Join(dir, name),
	} {
		if info, err := os.Stat(cand); err == nil && !info.IsDir() && info.Mode()&0o111 != 0 {
			return cand, true
		}
	}
	return "", false
}

// realRequired reports whether a MCPX_TEST_REQUIRE comma list demands the
// named server: an entry naming the server itself, the subset label, or
// the wildcard demands it.
func realRequired(requireList, name string) bool {
	for _, entry := range strings.Split(requireList, ",") {
		switch strings.TrimSpace(entry) {
		case name, "realsoftware", "real", "*":
			return true
		}
	}
	return false
}
