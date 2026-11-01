//go:build realsoftware

package e2e_test

import (
	"fmt"
	"strings"
	"testing"
	"time"

	"github.com/developing-today/code/pkgs/mcpx/internal/testsupport"
)

// The real-software subset (#1990): the real mcpx binary against a real,
// flake-pinned third-party MCP server — never in the per-PR path, which
// is what the realsoftware build tag guarantees. Run it via the nightly
// workflow, or by hand:
//
//	nix build .#mcpx-real-servers   (from the repo root)
//	MCPX_REAL_SERVERS_DIR=$PWD/result go test -tags realsoftware -run "^TestReal" ./internal/e2e

func realConfig(serverPath, namespace, description string, args ...string) string {
	argsJSON := ""
	if len(args) > 0 {
		quoted := make([]string, len(args))
		for i, a := range args {
			quoted[i] = fmt.Sprintf("%q", a)
		}
		argsJSON = `, "args": [` + strings.Join(quoted, ", ") + `]`
	}
	return fmt.Sprintf(`{
  "mcpServers": {
    %q: { "command": %q%s, "mcpx": { "sharing": "shared", "scope": "global", "description": %q } }
  }
}`, namespace, serverPath, argsJSON, description)
}

// realConfirmedCall runs one mcpx call that mcpx's destructive-action
// gate will stop for confirmation, and confirms it the way a person at
// the terminal would: by answering the elicitation question. Real
// servers annotate their mutating tools destructiveHint=true (memory's
// create_entities, filesystem's write_file); the fake's tools mostly do
// not, so this path is precisely what the per-PR suite cannot see —
// unconfirmed, the call is cancelled and nothing is written.
func realConfirmedCall(t *testing.T, e *env, args ...string) string {
	t.Helper()
	done := make(chan string, 1)
	go func() {
		out, _ := e.try(args...)
		done <- out
	}()
	id := waitForQuestion(t, e)
	e.run("elicit", "answer", id, `{"confirm":true}`)
	select {
	case out := <-done:
		return out
	case <-time.After(60 * time.Second):
		t.Fatal("the confirmed call did not finish")
		return ""
	}
}

func TestRealTimeServerTools(t *testing.T) {
	server := testsupport.RealServer(t, "mcp-server-time")
	e := newEnv(t, realConfig(server, "time", "the real mcp-server-time, pinned by the flake"))

	out := e.run("ls")
	if !strings.Contains(out, "time") {
		t.Fatalf("namespace missing from ls:\n%s", out)
	}
	out = e.run("types", "time")
	for _, want := range []string{"get_current_time", "convert_time"} {
		if !strings.Contains(out, want) {
			t.Errorf("types output missing real tool %q:\n%s", want, out)
		}
	}
	out = e.run("call", "time.get_current_time", `{"timezone":"UTC"}`)
	if !strings.Contains(out, "datetime") {
		t.Fatalf("get_current_time output wrong:\n%s", out)
	}
	out = e.run("call", "time.convert_time", `{"source_timezone":"UTC","time":"12:00","target_timezone":"America/Chicago"}`)
	if !strings.Contains(out, "datetime") {
		t.Fatalf("convert_time output wrong:\n%s", out)
	}
}

// TestRealLsLatencyRecorded is the discovery-latency leg of
// scripts/stress.sh folded into the subset (#1990): the 200 ms figure in
// the script is a laptop number, so the timings are recorded and only a
// pathological regression (two orders of magnitude over) fails.
func TestRealLsLatencyRecorded(t *testing.T) {
	server := testsupport.RealServer(t, "mcp-server-time")
	e := newEnv(t, realConfig(server, "time", "the real mcp-server-time, pinned by the flake"))

	e.run("ls") // warm: daemon start + discovery
	var worst time.Duration
	for i := 0; i < 10; i++ {
		start := time.Now()
		e.run("ls")
		if d := time.Since(start); d > worst {
			worst = d
		}
	}
	t.Logf("realsoftware latency: worst warm `mcpx ls` over 10 runs: %s (stress.sh laptop budget: 200ms; recorded, not gated)", worst)
	if worst > 20*time.Second {
		t.Fatalf("warm ls took %s; discovery against a real server regressed pathologically", worst)
	}
}
