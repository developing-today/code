package testsupport

import (
	"os"
	"os/exec"
	"strings"
	"testing"
)

// MCPX_TEST_REQUIRE names the external binaries a test run must have, as a
// comma-separated list (e.g. MCPX_TEST_REQUIRE=deno,bun,node,python3).
//
// A missing runtime normally skips the tests that need it, which is right
// on a laptop and wrong in CI: a leg that skips is a green leg that tested
// nothing (developing-today/code#1983, the #35 lesson). CI and container
// runs set MCPX_TEST_REQUIRE so a missing required binary fails the test
// instead of silently narrowing what the suite covered.
func RequiredBinaries() map[string]bool {
	out := map[string]bool{}
	for _, name := range strings.Split(os.Getenv("MCPX_TEST_REQUIRE"), ",") {
		if name = strings.ToLower(strings.TrimSpace(name)); name != "" {
			out[name] = true
		}
	}
	return out
}

// MissingRequired reports whether name is missing from PATH while
// MCPX_TEST_REQUIRE names it -- the one case where a test that cannot run
// must fail rather than skip or continue past the gap.
func MissingRequired(name string) bool {
	if !RequiredBinaries()[strings.ToLower(name)] {
		return false
	}
	_, err := exec.LookPath(name)
	return err != nil
}

// RequireBinary returns the path of name on PATH. When name is missing it
// skips the test, unless MCPX_TEST_REQUIRE names it, in which case the
// test fails: the environment promised the binary and did not provide it.
func RequireBinary(t testing.TB, name string) string {
	t.Helper()
	path, err := exec.LookPath(name)
	if err == nil {
		return path
	}
	if MissingRequired(name) {
		t.Fatalf("%s is required by MCPX_TEST_REQUIRE but is not on PATH: %v", name, err)
	}
	t.Skipf("%s not installed", name)
	return ""
}
