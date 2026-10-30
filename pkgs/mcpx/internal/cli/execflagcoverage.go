// Package cli: the exec/run flag probe contract (#2188).
package cli

// ExecFlagProbes and ExecFlagExemptions are the contract between the
// two guards for the fault this repository has shipped three times —
// a flag defined on the shared exec/run flag set, described in the
// help, and read on only one of the paths that share it:
//
//   - TestEveryExecFlagChangesSomething (internal/e2e) holds one probe
//     row per name in ExecFlagProbes: the flag is run with and without,
//     and must change an observable outcome both locally and on the
//     daemon (--remote) path.
//   - TestExecFlagSetIsFullyProbed (internal/cli) enumerates the real
//     exec/run FlagSet and fails when a hand-written flag (one defined
//     in runScript, as opposed to a settings-registry flag bound by
//     parseFlags) is in neither list.
//
// Settings-registry flags are not listed here: their reach — a flag
// must change the resolved value, not merely parse — is guarded for
// every command by TestEverySettingFlagReachesTheResolvedValue.
// This contract covers only the hand-written flags, whose effects are
// run outcomes, not resolved settings.
//
// The lists live in a non-test file (the handFlagsMeaningOther
// precedent in settings.go) so both test packages read one copy:
// a probe row added in e2e without a contract update fails the e2e
// match test, and a flag added in runScript without a row or a
// reasoned exemption fails the cli completeness test.
//
// ExecFlagExemptions is a ratchet: it may only shrink, or change with
// a deliberate, reasoned update in the same commit. An exemption is a
// claim that the flag cannot be probed by this guard's shape (one
// run, with and without, asserting on the run's output) — never a
// claim that the flag does not matter.

// ExecFlagProbes names the hand-written exec/run flags that have a
// probe row in TestEveryExecFlagChangesSomething, without the dashes.
var ExecFlagProbes = []string{
	"allow-repeat",
	"before",
	"env",
	"export",
	"launcher",
	"no-capture-console",
	"no-launcher",
	"on-error",
	"on-success",
	"prefix",
	"suffix",
	"typecheck",
	"validate",
}

// ExecFlagExemptions names the hand-written exec/run flags with no
// probe row, each with the reason a with/without output row cannot
// probe it.
var ExecFlagExemptions = map[string]string{
	"format":      "rendering only: it changes how the record stream is printed, never the run's outcome, so an emit-asserting row has nothing to compare",
	"keep":        "its effect is the generated client and script left on disk for inspection — a filesystem artifact, not a run outcome",
	"local":       "the default path itself: every probe row's local half is the --local outcome, and its counterpart --remote is the rows' remote half",
	"log-level":   "rendering only, like format: it filters which record lines print, it does not change what the run does",
	"log-source":  "rendering only, like format: it annotates record lines with call sites, it does not change what the run does",
	"ns":          "namespace restriction is observable only through generated-client tool visibility, which needs a multi-server env; this guard's env has one server — probe rows await a multi-namespace env (follow-on on #2188)",
	"permissions": "a denial is observable only through a per-runtime permission error whose shape differs across deno/bun/node; the rows assert runtime-stable output",
	"prompt":      "synthesizes the script through a configured model; the e2e env has no model, so there is no run to probe",
	"remote":      "probed by construction: every probe row runs twice, and the second half leads with --remote on the daemon path (#192)",
	"repair":      "model-backed repair of a failing script; the e2e env has no model, so a repaired run cannot be produced",
	"runtime":     "the without half is environment-dependent — auto picks the first installed runtime, so the same row would assert different outcomes on different machines",
	"session":     "names the session key; its effect is continuity ACROSS two runs sharing a key, which this guard's single with/without run cannot express",
	"timeout":     "a with/without row needs a script that sleeps: with the flag it is killed, without it the guard itself waits out the sleep — the probe would make the guard slow and timing-flaky by construction",
}
