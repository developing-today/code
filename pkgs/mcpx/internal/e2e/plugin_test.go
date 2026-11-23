package e2e_test

import (
	"encoding/json"
	"github.com/developing-today/code/pkgs/mcpx/internal/testsupport"
	"os/exec"
	"strings"
	"testing"
)

// TestThePluginSuiteRunsAgainstARealDaemon runs the opencode plugin's own
// tests, with a live daemon for the half that needs one.
//
// Nothing ran them before: CI runs `go test ./...`, and the plugin's bun
// tests sat beside it passing on whoever's machine happened to run them.
// A fake daemon proves the plugin sends what its author believed; only a
// real one proves the route reads it -- the difference between the two was
// DaemonClient.call(), which sent its arguments under a key /v1/call ignores.
func TestThePluginSuiteRunsAgainstARealDaemon(t *testing.T) {
	bun := testsupport.RequireBinary(t, "bun")
	e := newEnv(t, oneServer)
	e.run("ls")
	var where struct {
		Socket string `json:"socket"`
	}
	if err := json.Unmarshal([]byte(e.run("--json", "resolve")), &where); err != nil || where.Socket == "" {
		t.Fatalf("could not find the daemon's socket: %v", err)
	}

	cmd := exec.Command(bun, "test", "plugin/opencode/mcpx/", "plugin/core/")
	cmd.Dir = repoRoot(t)
	cmd.Env = append(e.envVars, "OPS_LIVE_SOCKET="+where.Socket, "OPS_LIVE_DIR="+e.dir)
	out, err := cmd.CombinedOutput()
	if err != nil {
		t.Fatalf("bun test plugin/opencode/mcpx/ plugin/core/: %v\n%s", err, out)
	}
	// The live tests skip themselves without a socket; a run that skipped
	// them is a run that did not check what this test exists to check.
	if strings.Contains(string(out), " skip") && !strings.Contains(string(out), " 0 skip") {
		t.Fatalf("some plugin tests were skipped:\n%s", out)
	}
	if !strings.Contains(string(out), " 0 fail") {
		t.Fatalf("the plugin suite did not report a clean run:\n%s", out)
	}
}

// TestPluginAnnotateChangesToolResultsFromTheEnvironment is the effect
// probe for plugin.annotate (#8472; the effect guard is #1971).
//
// The setting's whole effect lives in the opencode plugin host: when
// more than one daemon matches the directory, mcpx appends an
// "answered by" note to tool results, and MCPX_PLUGIN_ANNOTATE=false
// turns that note off. No Go caller produces the note, so the probe
// drives the host the way the plugin suite does — bun, the real tool
// executors, options resolved from the environment — and the bun
// cases in plugin/opencode/v2/annotate.test.ts assert the observable
// with/without difference. A run that skipped the cases is a run that
// did not check what this probe exists to check, so skips fail it,
// as in TestThePluginSuiteRunsAgainstARealDaemon.
func TestPluginAnnotateChangesToolResultsFromTheEnvironment(t *testing.T) {
	bun := testsupport.RequireBinary(t, "bun")
	recordSettingProbes(t)
	cmd := exec.Command(bun, "test", "plugin/opencode/v2/annotate.test.ts")
	cmd.Dir = repoRoot(t)
	out, err := cmd.CombinedOutput()
	if err != nil {
		t.Fatalf("bun test plugin/opencode/v2/annotate.test.ts: %v\n%s", err, out)
	}
	if strings.Contains(string(out), " skip") && !strings.Contains(string(out), " 0 skip") {
		t.Fatalf("the plugin.annotate probe cases were skipped:\n%s", out)
	}
	if !strings.Contains(string(out), " 0 fail") {
		t.Fatalf("the plugin.annotate probe did not report a clean run:\n%s", out)
	}
}
