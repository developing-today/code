package testsupport

import (
	"os"
	"os/exec"
	"sort"
	"strings"
	"testing"
)

// requireList is MCPX_TEST_REQUIRE parsed once (#6904).
//
// MCPX_TEST_REQUIRE names the external things a test run must have, as a
// comma-separated list (e.g. MCPX_TEST_REQUIRE=deno,bun,node,python3).
// Runtime binaries and real-software servers share the one variable, so
// there is exactly one parser for it: entries are trimmed and lower-cased,
// and the three non-binary spellings are stated here and nowhere else:
//
//   - "realsoftware" and "real" name the real-server subset: every real
//     server is demanded.
//   - "*" is the wildcard: every real server is demanded. It does not
//     demand runtime binaries, which have no wildcard meaning.
//   - any other entry names a runtime binary (checked on PATH) and, when a
//     real server has the same name, that server too.
//
// Case is insignificant in every position: "RealSoftware", "REALSOFTWARE"
// and "realsoftware" mean the same thing, for binaries and servers alike.
type requireList struct {
	binaries map[string]bool // lower-cased runtime binary names demanded
	realAll  bool            // a subset label or the wildcard demands every real server
}

// parseRequireList is the single parser for MCPX_TEST_REQUIRE.
func parseRequireList(raw string) requireList {
	l := requireList{binaries: map[string]bool{}}
	for _, entry := range strings.Split(raw, ",") {
		switch entry = strings.ToLower(strings.TrimSpace(entry)); entry {
		case "":
		case "realsoftware", "real", "*":
			l.realAll = true
		default:
			l.binaries[entry] = true
		}
	}
	return l
}

// demandsBinary reports whether the list names name as a runtime binary.
func (l requireList) demandsBinary(name string) bool {
	return l.binaries[strings.ToLower(strings.TrimSpace(name))]
}

// demandsRealServer reports whether the list demands the named real server:
// an entry naming the server itself, a subset label, or the wildcard.
func (l requireList) demandsRealServer(name string) bool {
	return l.realAll || l.demandsBinary(name)
}

// RequiredBinaries returns the external binaries a test run must have, as
// named by MCPX_TEST_REQUIRE.
//
// A missing runtime normally skips the tests that need it, which is right
// on a laptop and wrong in CI: a leg that skips is a green leg that tested
// nothing (developing-today/code#1983, the #35 lesson). CI and container
// runs set MCPX_TEST_REQUIRE so a missing required binary fails the test
// instead of silently narrowing what the suite covered.
//
// The real-server subset labels and the wildcard are not binaries and are
// not returned here; see requireList for the rules.
func RequiredBinaries() map[string]bool {
	parsed := parseRequireList(os.Getenv("MCPX_TEST_REQUIRE"))
	out := make(map[string]bool, len(parsed.binaries))
	for name := range parsed.binaries {
		out[name] = true
	}
	return out
}

// MissingRequiredBinaries returns the required binaries that are not on
// PATH, sorted. It is the suite-level check (#6904): per-test enforcement
// only covers entries that some test happens to pass to RequireBinary, so
// an entry with no consumer -- python3 in CI's list -- would otherwise be
// enforced by nothing and narrow the run silently.
func MissingRequiredBinaries() []string {
	var missing []string
	for name := range RequiredBinaries() {
		if _, err := exec.LookPath(name); err != nil {
			missing = append(missing, name)
		}
	}
	sort.Strings(missing)
	return missing
}

// MissingRequired reports whether name is missing from PATH while
// MCPX_TEST_REQUIRE names it -- the one case where a test that cannot run
// must fail rather than skip or continue past the gap.
func MissingRequired(name string) bool {
	if !parseRequireList(os.Getenv("MCPX_TEST_REQUIRE")).demandsBinary(name) {
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
