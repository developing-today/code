package testsupport

import (
	"strings"
	"testing"
)

// TestRequiredBinariesAreOnPath is the suite-level enforcement for
// MCPX_TEST_REQUIRE (#6904).
//
// Per-test enforcement only covers an entry that some test passes to
// RequireBinary: an entry with no consumer -- python3 in CI's list -- was
// checked by nothing, so a missing python3 narrowed the run silently.
// This test walks the whole list, so every entry in CI's list is enforced
// even when no individual test names it.
func TestRequiredBinariesAreOnPath(t *testing.T) {
	if missing := MissingRequiredBinaries(); len(missing) > 0 {
		t.Fatalf("MCPX_TEST_REQUIRE lists entries that are not available (on PATH or under MCPX_REAL_SERVERS_DIR): %s", strings.Join(missing, ", "))
	}
}
