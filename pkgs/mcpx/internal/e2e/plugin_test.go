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

// TestPluginToolTimingRecordsToolCallsFromTheEnvironment is the
// effect probe for plugin.toolTiming (#8472; the effect guard is
// #1971).
//
// The setting's whole effect lives in the opencode plugin host: with
// MCPX_PLUGIN_TOOL_TIMING on, the session plugin's
// tool.execute.after hook sends one harness.tool record per finished
// tool call to the daemon's durable log (DaemonClient.record,
// POST /v1/log); off or unset, the hook returns before anything is
// sent. No Go caller produces those records, so the probe drives the
// host the way the plugin suite does — bun, the real session plugin,
// the setting set in the environment before the plugin loads — and
// the bun cases in plugin/opencode/mcpx/tool-timing.test.ts point
// discovery at a stub daemon that answers /v1/health with the
// plugin's own operation fingerprint and captures /v1/log posts.
// They assert the observable with/without difference: the record
// arrives carrying the call's tool, session, call id and title when
// on, and no record is posted when off even though the daemon was
// reachable (its health endpoint was hit). A run that skipped the
// cases is a run that did not check what this probe exists to
// check, so skips fail it, as in
// TestThePluginSuiteRunsAgainstARealDaemon.
func TestPluginToolTimingRecordsToolCallsFromTheEnvironment(t *testing.T) {
	bun := testsupport.RequireBinary(t, "bun")
	recordSettingProbes(t)
	cmd := exec.Command(bun, "test", "plugin/opencode/mcpx/tool-timing.test.ts")
	cmd.Dir = repoRoot(t)
	out, err := cmd.CombinedOutput()
	if err != nil {
		t.Fatalf("bun test plugin/opencode/mcpx/tool-timing.test.ts: %v\n%s", err, out)
	}
	if strings.Contains(string(out), " skip") && !strings.Contains(string(out), " 0 skip") {
		t.Fatalf("the plugin.toolTiming probe cases were skipped:\n%s", out)
	}
	if !strings.Contains(string(out), " 0 fail") {
		t.Fatalf("the plugin.toolTiming probe did not report a clean run:\n%s", out)
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

// TestPluginDiscoveryRetryDelaysRediscoveryFromTheEnvironment is
// the effect probe for plugin.discoveryRetry (#8472; the effect
// guard is #1971).
//
// The setting's whole effect lives in the opencode plugin host:
// after a discovery miss, the tool runtime holds the miss for the
// duration MCPX_PLUGIN_DISCOVERY_RETRY names (a Go duration, the
// registry default is 1m0s) instead of running the discovery
// ladder again on every look. No Go caller reads it, so the probe
// drives the host the way the plugin suite does — bun, the real
// tool runtime, options resolved from the environment — and the
// bun cases in plugin/opencode/v2/discovery-retry.test.ts assert
// the observable differences with a counting ladder that always
// misses: unset and a long value hold the miss (one ladder run
// across repeated looks), 0s rediscovers on every look, a short
// value holds inside its window and releases after it, and an
// unparseable value falls back to the default rather than to
// zero. A run that skipped the cases is a run that did not check
// what this probe exists to check, so skips fail it, as in
// TestThePluginSuiteRunsAgainstARealDaemon.
func TestPluginDiscoveryRetryDelaysRediscoveryFromTheEnvironment(t *testing.T) {
	bun := testsupport.RequireBinary(t, "bun")
	recordSettingProbes(t)
	cmd := exec.Command(bun, "test", "plugin/opencode/v2/discovery-retry.test.ts")
	cmd.Dir = repoRoot(t)
	out, err := cmd.CombinedOutput()
	if err != nil {
		t.Fatalf("bun test plugin/opencode/v2/discovery-retry.test.ts: %v\n%s", err, out)
	}
	if strings.Contains(string(out), " skip") && !strings.Contains(string(out), " 0 skip") {
		t.Fatalf("the plugin.discoveryRetry probe cases were skipped:\n%s", out)
	}
	if !strings.Contains(string(out), " 0 fail") {
		t.Fatalf("the plugin.discoveryRetry probe did not report a clean run:\n%s", out)
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

// TestPluginRememberScopePersistsTheDaemonChoiceFromTheEnvironment
// is the effect probe for plugin.remember (#8472; the effect guard
// is #1971).
//
// The setting's whole effect lives in the opencode plugin host:
// when a daemon choice is made in the picker and the call names no
// scope of its own, the scope is the setting's value (session /
// until-gone / indefinite, from MCPX_PLUGIN_REMEMBER) — session
// keeps the choice in memory only, while until-gone and
// indefinite write it to the host's per-plugin storage, where a
// later session recalls it. No Go caller persists it, so the
// probe drives the host the way the plugin suite does — bun, the
// real tool runtime, the real picker and the real storage-backed
// applyChoice, options resolved from the environment — and the
// bun cases in plugin/opencode/v2/remember.test.ts assert the
// observable differences: unset (the session default) writes
// nothing to storage, until-gone and indefinite each persist the
// picked target under the directory's key, and the scope shown
// to the permission prompt is the setting's value. A run that
// skipped the cases is a run that did not check what this probe
// exists to check, so skips fail it, as in
// TestThePluginSuiteRunsAgainstARealDaemon.
func TestPluginRememberScopePersistsTheDaemonChoiceFromTheEnvironment(t *testing.T) {
	bun := testsupport.RequireBinary(t, "bun")
	recordSettingProbes(t)
	cmd := exec.Command(bun, "test", "plugin/opencode/v2/remember.test.ts")
	cmd.Dir = repoRoot(t)
	out, err := cmd.CombinedOutput()
	if err != nil {
		t.Fatalf("bun test plugin/opencode/v2/remember.test.ts: %v\n%s", err, out)
	}
	if strings.Contains(string(out), " skip") && !strings.Contains(string(out), " 0 skip") {
		t.Fatalf("the plugin.remember probe cases were skipped:\n%s", out)
	}
	if !strings.Contains(string(out), " 0 fail") {
		t.Fatalf("the plugin.remember probe did not report a clean run:\n%s", out)
	}
}

// TestPluginHeadlessSuppressesToastsFromTheEnvironment is the effect
// probe for plugin.headless (#8472; the effect guard is #1971).
//
// The setting's whole effect lives in the opencode plugin host: when
// discovery cannot settle on one daemon, the session plugin warns
// the user with a toast — unless headless is set (MCPX_PLUGIN_HEADLESS
// or the plugin option), because showToast succeeds whether or not
// anyone is watching and a toast nobody can see is a leak, not a
// notification. No Go caller produces the toast, so the probe drives
// the host the way the plugin suite does — bun, the real session
// plugin, two fake daemons on real sockets so boot discovery is
// ambiguous — and the bun cases in
// plugin/opencode/mcpx/headless.test.ts assert the observable
// with/without difference: the warning toast arrives when headless
// is cleared and never when it is set. A run that skipped the cases
// is a run that did not check what this probe exists to check, so
// skips fail it, as in TestThePluginSuiteRunsAgainstARealDaemon.
func TestPluginHeadlessSuppressesToastsFromTheEnvironment(t *testing.T) {
	bun := testsupport.RequireBinary(t, "bun")
	recordSettingProbes(t)
	cmd := exec.Command(bun, "test", "plugin/opencode/mcpx/headless.test.ts")
	cmd.Dir = repoRoot(t)
	out, err := cmd.CombinedOutput()
	if err != nil {
		t.Fatalf("bun test plugin/opencode/mcpx/headless.test.ts: %v\n%s", err, out)
	}
	if strings.Contains(string(out), " skip") && !strings.Contains(string(out), " 0 skip") {
		t.Fatalf("the plugin.headless probe cases were skipped:\n%s", out)
	}
	if !strings.Contains(string(out), " 0 fail") {
		t.Fatalf("the plugin.headless probe did not report a clean run:\n%s", out)
	}
}

// TestPluginInstructionsAppendsTheContextParagraphFromTheEnvironment
// is the effect probe for plugin.instructions (#8472; the effect
// guard is #1971).
//
// The setting's whole effect lives in the opencode plugin host: it
// is the opt-in that adds mcpx's session-context paragraph to the
// model's system prompt — the v2 setup's session "context" hook
// calls appendContextOnce only when the resolved option is on
// (plugin/opencode/v2/index.ts), and the paragraph is paid help, so
// it stays off unless asked for. No Go caller produces the system
// prompt, so the probe drives the host the way the plugin suite
// does — bun, the real setupMcpxV2 with a stub context — and the
// bun cases in plugin/opencode/v2/instructions.test.ts assert the
// observable with/without difference: unset and explicit false
// append nothing; true appends the paragraph, once per system
// array; another truthy spelling appends it too; and the host
// option beats the environment in both directions. The cases set
// the setting the way a user would — MCPX_PLUGIN_INSTRUCTIONS
// through resolveOptions, or the host option. A run that skipped
// the cases is a run that did not check what this probe exists to
// check, so skips fail it, as in
// TestPluginHeadlessSuppressesToastsFromTheEnvironment.
func TestPluginInstructionsAppendsTheContextParagraphFromTheEnvironment(t *testing.T) {
	bun := testsupport.RequireBinary(t, "bun")
	recordSettingProbes(t)
	cmd := exec.Command(bun, "test", "plugin/opencode/v2/instructions.test.ts")
	cmd.Dir = repoRoot(t)
	out, err := cmd.CombinedOutput()
	if err != nil {
		t.Fatalf("bun test plugin/opencode/v2/instructions.test.ts: %v\n%s", err, out)
	}
	if strings.Contains(string(out), " skip") && !strings.Contains(string(out), " 0 skip") {
		t.Fatalf("the plugin.instructions probe cases were skipped:\n%s", out)
	}
	if !strings.Contains(string(out), " 0 fail") {
		t.Fatalf("the plugin.instructions probe did not report a clean run:\n%s", out)
	}
}

// TestPluginDisabledToolsHidesToolsFromTheEnvironment is the effect
// probe for plugin.disabledTools (#8472; the effect guard is #1971).
//
// The setting's whole effect lives in the opencode plugin host: it
// names the model-facing mcpx tools hidden from every session —
// setup records the resolved list as a session's tool-filter policy
// in ctx.storage on the session's first sighting, and the session
// "context" hook deletes the named tools from the per-request tools
// record from the next turn on
// (plugin/opencode/v2/tool-filter.ts). No Go caller builds that
// record, so the probe drives the host the way the plugin suite
// does — bun, the real setupMcpxV2 with a stub context — and the
// bun cases in plugin/opencode/v2/disabled-tools.test.ts assert the
// observable with/without difference: unset hides nothing and seeds
// no policy; a named tool is hidden from the second turn while its
// siblings and the host tools stay; a separated list hides each
// named tool; a name outside the mcpx_ namespace can never hide a
// host tool; and the host option beats the environment in both
// directions. The cases set the setting the way a user would —
// MCPX_PLUGIN_DISABLED_TOOLS through resolveOptions, or the host
// option. A run that skipped the cases is a run that did not check
// what this probe exists to check, so skips fail it, as in
// TestPluginInstructionsAppendsTheContextParagraphFromTheEnvironment.
func TestPluginDisabledToolsHidesToolsFromTheEnvironment(t *testing.T) {
	bun := testsupport.RequireBinary(t, "bun")
	recordSettingProbes(t)
	cmd := exec.Command(bun, "test", "plugin/opencode/v2/disabled-tools.test.ts")
	cmd.Dir = repoRoot(t)
	out, err := cmd.CombinedOutput()
	if err != nil {
		t.Fatalf("bun test plugin/opencode/v2/disabled-tools.test.ts: %v\n%s", err, out)
	}
	if strings.Contains(string(out), " skip") && !strings.Contains(string(out), " 0 skip") {
		t.Fatalf("the plugin.disabledTools probe cases were skipped:\n%s", out)
	}
	if !strings.Contains(string(out), " 0 fail") {
		t.Fatalf("the plugin.disabledTools probe did not report a clean run:\n%s", out)
	}
}
