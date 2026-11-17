package e2e_test

import (
	"strings"
	"testing"
)

// Burn-down exercises for #8462: the cli discovery group (batch,
// list, namespaces) carried feature-coverage exemptions instead of
// integration tests. Each test below runs the real command through
// the e2e harness against a live daemon, which is what records the
// feature's ledger row; the exemption lines are deleted with them,
// and the coverage guard fails if a covered feature keeps one.

// TestBatchRunsCallsThroughTheCLI exercises `mcpx batch`: several
// tool calls in one turn, no JavaScript runtime, results rendered
// per call.
func TestBatchRunsCallsThroughTheCLI(t *testing.T) {
	e := newEnv(t, oneServer)
	out := e.run("batch", "-c", `[{"server":"demo","tool":"echo","args":{"message":"first"}},{"server":"demo","tool":"echo","args":{"message":"second"}}]`)
	for _, want := range []string{"first", "second"} {
		if !strings.Contains(out, want) {
			t.Fatalf("batch output missing %q:\n%s", want, out)
		}
	}
}

// TestListAliasListsNamespaces exercises `mcpx list`, the long alias
// of ls: it must show the configured namespace and its description.
func TestListAliasListsNamespaces(t *testing.T) {
	e := newEnv(t, oneServer)
	out := e.run("list")
	if !strings.Contains(out, "demo") || !strings.Contains(out, "a fake server") {
		t.Fatalf("list should show the demo namespace and its description:\n%s", out)
	}
}

// TestNamespacesAliasListsNamespaces exercises `mcpx namespaces`,
// the descriptive alias of ls, with the same expectation.
func TestNamespacesAliasListsNamespaces(t *testing.T) {
	e := newEnv(t, oneServer)
	out := e.run("namespaces")
	if !strings.Contains(out, "demo") || !strings.Contains(out, "a fake server") {
		t.Fatalf("namespaces should show the demo namespace and its description:\n%s", out)
	}
}
