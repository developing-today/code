package e2e_test

import (
	"strings"
	"testing"
)

// Coverage burn-down for the #8462 discovery group: the cli commands
// batch, list, and namespaces had feature-coverage exemptions instead
// of integration-test exercises. Each test below drives the actual CLI
// command through the harness env (which records the coverage ledger
// row on success) against the oneServer daemon setup, so the
// exemptions can be deleted and the #1978 guard verifies the deletion
// on the next full-suite run.

// TestCLIDiscoveryListExercise runs `mcpx list` (the ls alias) against
// a daemon with one configured server and checks the server is listed.
func TestCLIDiscoveryListExercise(t *testing.T) {
	e := newEnv(t, oneServer)
	out := e.run("list")
	if !strings.Contains(out, "demo") {
		t.Fatalf("list: the configured demo server is not listed:\n%s", out)
	}
}

// TestCLIDiscoveryNamespacesExercise runs `mcpx namespaces` (the other
// ls alias) against the same setup; the alias is a distinct ledger
// feature from both ls and list, so it needs its own dispatch.
func TestCLIDiscoveryNamespacesExercise(t *testing.T) {
	e := newEnv(t, oneServer)
	out := e.run("namespaces")
	if !strings.Contains(out, "demo") {
		t.Fatalf("namespaces: the configured demo namespace is not listed:\n%s", out)
	}
}

// TestCLIDiscoveryBatchExercise runs `mcpx batch` with one real tool
// call against the demo server and checks the echoed result comes
// back, exercising the batch dispatch end to end (parse, daemon
// ensure, call, report) rather than only its usage text.
func TestCLIDiscoveryBatchExercise(t *testing.T) {
	e := newEnv(t, oneServer)
	out := e.run("batch", "-c", `[{"name":"demo.echo","args":{"message":"burn-down"}}]`)
	if !strings.Contains(out, "burn-down") {
		t.Fatalf("batch: the demo.echo result is not in the output:\n%s", out)
	}
}
