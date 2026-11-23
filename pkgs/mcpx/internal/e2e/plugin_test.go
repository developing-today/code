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

// TestPluginBackendChoosesDaemonOrBinaryFromTheEnvironment is the
// effect probe for plugin.backend (#8472; the effect guard is #1971).
//
// The setting's whole effect lives in the opencode plugin host: on
// every tool call it decides whether the host talks to the daemon
// over its socket or spawns the mcpx binary — auto prefers the
// socket and falls back, v1 refuses to fall back, cli never uses
// the socket. No Go caller makes that choice, so the probe drives
// the host the way the plugin suite does — bun, the real tool
// executors, options resolved from the environment — and the bun
// cases in plugin/opencode/v2/backend.test.ts assert the observable
// differences: who answered (daemon, binary, or the v1 refusal) and
// who was never asked. A run that skipped the cases is a run that
// did not check what this probe exists to check, so skips fail it,
// as in TestThePluginSuiteRunsAgainstARealDaemon.
func TestPluginBackendChoosesDaemonOrBinaryFromTheEnvironment(t *testing.T) {
	bun := testsupport.RequireBinary(t, "bun")
	recordSettingProbes(t)
	cmd := exec.Command(bun, "test", "plugin/opencode/v2/backend.test.ts")
	cmd.Dir = repoRoot(t)
	out, err := cmd.CombinedOutput()
	if err != nil {
		t.Fatalf("bun test plugin/opencode/v2/backend.test.ts: %v\n%s", err, out)
	}
	if strings.Contains(string(out), " skip") && !strings.Contains(string(out), " 0 skip") {
		t.Fatalf("the plugin.backend probe cases were skipped:\n%s", out)
	}
	if !strings.Contains(string(out), " 0 fail") {
		t.Fatalf("the plugin.backend probe did not report a clean run:\n%s", out)
	}
}

// TestPluginBinInvokesTheConfiguredExecutableFromTheEnvironment is
// the effect probe for plugin.bin (#8472; the effect guard is #1971).
//
// The setting's whole effect lives in the opencode plugin host: when
// a tool call takes the binary path (backend=cli always, backend=auto
// when no daemon answers), the host spawns the executable
// MCPX_PLUGIN_BIN names instead of the built-in default. No Go caller
// spawns it, so the probe drives the host the way the plugin suite
// does — bun, the real tool executors, options resolved from the
// environment, and crucially the real runBin — and the bun cases in
// plugin/opencode/v2/bin.test.ts assert the observable differences:
// the configured stub executable answers, a different value makes a
// different executable answer, and a path naming nothing runnable
// yields the no-binary refusal. A run that skipped the cases is a
// run that did not check what this probe exists to check, so skips
// fail it, as in TestThePluginSuiteRunsAgainstARealDaemon.
func TestPluginBinInvokesTheConfiguredExecutableFromTheEnvironment(t *testing.T) {
	bun := testsupport.RequireBinary(t, "bun")
	recordSettingProbes(t)
	cmd := exec.Command(bun, "test", "plugin/opencode/v2/bin.test.ts")
	cmd.Dir = repoRoot(t)
	out, err := cmd.CombinedOutput()
	if err != nil {
		t.Fatalf("bun test plugin/opencode/v2/bin.test.ts: %v\n%s", err, out)
	}
	if strings.Contains(string(out), " skip") && !strings.Contains(string(out), " 0 skip") {
		t.Fatalf("the plugin.bin probe cases were skipped:\n%s", out)
	}
	if !strings.Contains(string(out), " 0 fail") {
		t.Fatalf("the plugin.bin probe did not report a clean run:\n%s", out)
	}
}

// TestPluginBinArgsPrependsTheConfiguredArgumentsFromTheEnvironment
// is the effect probe for plugin.binArgs (#8472; the effect guard
// is #1971).
//
// The setting's whole effect lives in the opencode plugin host:
// whenever a tool call takes the binary path, runBin spawns the
// mcpx executable with the arguments MCPX_PLUGIN_BIN_ARGS names
// prepended to the call's own argv — a JSON array when the value
// starts with "[", whitespace-split otherwise. No Go caller
// prepends them, so the probe drives the host the way the plugin
// suite does — bun, the real tool executors, options resolved
// from the environment, and crucially the real runBin — and the
// bun cases in plugin/opencode/v2/bin-args.test.ts assert the
// observable differences: the configured arguments arrive first,
// in order; a space inside a JSON-form argument survives as one
// argv element; and with the setting unset the executable sees
// only the call's own argv. A run that skipped the cases is a
// run that did not check what this probe exists to check, so
// skips fail it, as in TestThePluginSuiteRunsAgainstARealDaemon.
func TestPluginBinArgsPrependsTheConfiguredArgumentsFromTheEnvironment(t *testing.T) {
	bun := testsupport.RequireBinary(t, "bun")
	recordSettingProbes(t)
	cmd := exec.Command(bun, "test", "plugin/opencode/v2/bin-args.test.ts")
	cmd.Dir = repoRoot(t)
	out, err := cmd.CombinedOutput()
	if err != nil {
		t.Fatalf("bun test plugin/opencode/v2/bin-args.test.ts: %v\n%s", err, out)
	}
	if strings.Contains(string(out), " skip") && !strings.Contains(string(out), " 0 skip") {
		t.Fatalf("the plugin.binArgs probe cases were skipped:\n%s", out)
	}
	if !strings.Contains(string(out), " 0 fail") {
		t.Fatalf("the plugin.binArgs probe did not report a clean run:\n%s", out)
	}
}

// TestPluginToolsRegistersTheToolSetFromTheEnvironment is the
// effect probe for plugin.tools (#8472; the effect guard is
// #1971).
//
// The setting's whole effect lives in the opencode plugin host:
// MCPX_PLUGIN_TOOLS is the opt-in that puts mcpx's own tool set
// (discover / exec / observe) in front of the model — setup hands
// the resolved value to registerTools as toolsEnabled, and the
// same value feeds the daemon tools' default, so on also offers
// the daemon tools while off registers nothing but the namespace.
// No Go caller registers those tools, so the probe drives the
// host the way the plugin suite does — bun, the real tool
// runtime and the real registerTools, options resolved from the
// environment, a recording editor as the oracle — and the bun
// cases in plugin/opencode/v2/plugin-tools.test.ts assert the
// observable differences: unset and explicit false register no
// tools; true (and another truthy spelling) registers the full
// set, daemon tools included. A run that skipped the cases is a
// run that did not check what this probe exists to check, so
// skips fail it, as in TestThePluginSuiteRunsAgainstARealDaemon.
func TestPluginToolsRegistersTheToolSetFromTheEnvironment(t *testing.T) {
	bun := testsupport.RequireBinary(t, "bun")
	recordSettingProbes(t)
	cmd := exec.Command(bun, "test", "plugin/opencode/v2/plugin-tools.test.ts")
	cmd.Dir = repoRoot(t)
	out, err := cmd.CombinedOutput()
	if err != nil {
		t.Fatalf("bun test plugin/opencode/v2/plugin-tools.test.ts: %v\n%s", err, out)
	}
	if strings.Contains(string(out), " skip") && !strings.Contains(string(out), " 0 skip") {
		t.Fatalf("the plugin.tools probe cases were skipped:\n%s", out)
	}
	if !strings.Contains(string(out), " 0 fail") {
		t.Fatalf("the plugin.tools probe did not report a clean run:\n%s", out)
	}
}

// TestPluginEnvLevelChangesShellEnvFromTheEnvironment is the effect
// probe for plugin.env (#8472; the effect guard is #1971).
//
// The setting's whole effect lives in the opencode plugin host: the
// level (minimal / standard / full, from MCPX_PLUGIN_ENV) gates the
// shell.env hook — minimal injects only the session id, standard
// adds the call/project/harness block, and full additionally
// describes the session and injects the session block. No Go caller
// produces those variables, so the probe drives the host the way
// the plugin suite does — bun, the real session plugin, options
// resolved from the environment — and the bun cases in
// plugin/opencode/mcpx/env.test.ts assert the observable
// with/without differences, including that the session is only
// ever described at full. A run that skipped the cases is a run
// that did not check what this probe exists to check, so skips
// fail it, as in TestThePluginSuiteRunsAgainstARealDaemon.
func TestPluginEnvLevelChangesShellEnvFromTheEnvironment(t *testing.T) {
	bun := testsupport.RequireBinary(t, "bun")
	recordSettingProbes(t)
	cmd := exec.Command(bun, "test", "plugin/opencode/mcpx/env.test.ts")
	cmd.Dir = repoRoot(t)
	out, err := cmd.CombinedOutput()
	if err != nil {
		t.Fatalf("bun test plugin/opencode/mcpx/env.test.ts: %v\n%s", err, out)
	}
	if strings.Contains(string(out), " skip") && !strings.Contains(string(out), " 0 skip") {
		t.Fatalf("the plugin.env probe cases were skipped:\n%s", out)
	}
	if !strings.Contains(string(out), " 0 fail") {
		t.Fatalf("the plugin.env probe did not report a clean run:\n%s", out)
	}
}

// TestPluginDaemonToolsAreOfferedFromTheEnvironment is the effect
// probe for plugin.daemonTools (#8472; the effect guard is #1971).
//
// The setting's whole effect lives in the opencode plugin host: it
// gates whether the session offers the three daemon tools
// (mcpx_daemon_status, mcpx_daemon_select, mcpx_daemon_forget). The
// resolution is a tri-state — the plugin option first, then
// MCPX_PLUGIN_DAEMON_TOOLS parsed as a truthy value, then a fallback
// to the main plugin.tools switch or an ambiguous discovery — and
// "auto", the registry default, decides nothing on the first two
// rungs. No Go caller offers those tools, so the probe drives the
// host the way the plugin suite does — bun, the real session plugin,
// options resolved from the environment — and the bun cases in
// plugin/opencode/mcpx/daemon-tools.test.ts assert the observable
// with/without differences in the returned tool map, with daemon
// discovery isolated to empty state directories so the fallback arm
// is deterministic. A run that skipped the cases is a run that did
// not check what this probe exists to check, so skips fail it, as
// in TestThePluginSuiteRunsAgainstARealDaemon.
func TestPluginDaemonToolsAreOfferedFromTheEnvironment(t *testing.T) {
	bun := testsupport.RequireBinary(t, "bun")
	recordSettingProbes(t)
	cmd := exec.Command(bun, "test", "plugin/opencode/mcpx/daemon-tools.test.ts")
	cmd.Dir = repoRoot(t)
	out, err := cmd.CombinedOutput()
	if err != nil {
		t.Fatalf("bun test plugin/opencode/mcpx/daemon-tools.test.ts: %v\n%s", err, out)
	}
	if strings.Contains(string(out), " skip") && !strings.Contains(string(out), " 0 skip") {
		t.Fatalf("the plugin.daemonTools probe cases were skipped:\n%s", out)
	}
	if !strings.Contains(string(out), " 0 fail") {
		t.Fatalf("the plugin.daemonTools probe did not report a clean run:\n%s", out)
	}
}
