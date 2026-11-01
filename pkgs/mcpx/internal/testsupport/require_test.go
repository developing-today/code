package testsupport

import (
	"strings"
	"testing"
)

func TestRequiredBinariesParsesTheList(t *testing.T) {
	t.Setenv("MCPX_TEST_REQUIRE", "deno, bun ,NODE,, python3 ")
	got := RequiredBinaries()
	for _, want := range []string{"deno", "bun", "node", "python3"} {
		if !got[want] {
			t.Errorf("%q should be required, parsed %v", want, got)
		}
	}
	if len(got) != 4 {
		t.Errorf("want exactly 4 required binaries, got %v", got)
	}
}

func TestRequiredBinariesEmptyWhenUnset(t *testing.T) {
	t.Setenv("MCPX_TEST_REQUIRE", "")
	if got := RequiredBinaries(); len(got) != 0 {
		t.Errorf("unset MCPX_TEST_REQUIRE should require nothing, got %v", got)
	}
}

// requireTB records which of Skipf/Fatalf a RequireBinary call used.
type requireTB struct {
	testing.TB
	skipped, fataled string
}

func (r *requireTB) Helper() {}
func (r *requireTB) Skipf(format string, args ...any) {
	r.skipped = format
	panic("skip")
}
func (r *requireTB) Fatalf(format string, args ...any) {
	r.fataled = format
	panic("fatal")
}

func callRequire(name string) (tb *requireTB) {
	tb = &requireTB{}
	defer func() { _ = recover() }()
	RequireBinary(tb, name)
	return tb
}

func TestRequireBinarySkipsAMissingOptionalBinary(t *testing.T) {
	t.Setenv("MCPX_TEST_REQUIRE", "deno")
	tb := callRequire("mcpx-no-such-binary-5584")
	if tb.fataled != "" || !strings.Contains(tb.skipped, "not installed") {
		t.Errorf("missing optional binary should skip, got skip=%q fatal=%q", tb.skipped, tb.fataled)
	}
}

func TestRequireBinaryFailsAMissingRequiredBinary(t *testing.T) {
	t.Setenv("MCPX_TEST_REQUIRE", "mcpx-no-such-binary-5584")
	tb := callRequire("mcpx-no-such-binary-5584")
	if tb.skipped != "" || !strings.Contains(tb.fataled, "MCPX_TEST_REQUIRE") {
		t.Errorf("missing required binary should fail naming MCPX_TEST_REQUIRE, got skip=%q fatal=%q", tb.skipped, tb.fataled)
	}
}

func TestRequireBinaryReturnsThePathOfAPresentBinary(t *testing.T) {
	t.Setenv("MCPX_TEST_REQUIRE", "")
	tb := &requireTB{}
	if path := RequireBinary(tb, "go"); path == "" {
		t.Skip("go not on PATH on this machine; the present-binary path is covered in CI")
	}
	if tb.skipped != "" || tb.fataled != "" {
		t.Errorf("present binary should neither skip nor fail, got skip=%q fatal=%q", tb.skipped, tb.fataled)
	}
}

func TestMissingRequired(t *testing.T) {
	t.Setenv("MCPX_TEST_REQUIRE", "mcpx-no-such-binary-5584")
	if !MissingRequired("mcpx-no-such-binary-5584") {
		t.Error("a required binary that is absent should report missing-required")
	}
	if MissingRequired("go") {
		t.Error("a binary nobody required should not report missing-required")
	}
	t.Setenv("MCPX_TEST_REQUIRE", "")
	if MissingRequired("mcpx-no-such-binary-5584") {
		t.Error("with MCPX_TEST_REQUIRE unset, nothing is missing-required")
	}
}
