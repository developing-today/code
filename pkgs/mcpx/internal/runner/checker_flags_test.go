package runner

import (
	"bytes"
	"strings"
	"testing"

	"github.com/developing-today/code/pkgs/mcpx/internal/preflight"
)

// TestCheckerDepsFlagsPreservesNoDownloadOnSwitch is the Codex finding
// on PR #6111: switching a Node or Bun script to the Deno checker
// forwarded the EXECUTION runtime's dependency flags, so a
// deps.autoInstall=off set let the checker download uncached
// dependencies -- Bun's --no-install is discarded by ResolutionArgs
// and Node contributes none. The checker must carry the no-download
// policy its own runtime would.
func TestCheckerDepsFlagsPreservesNoDownloadOnSwitch(t *testing.T) {
	off := &Deps{AutoInstall: "off"}
	bunExec := depsFlags(KindBun, off)
	for _, tc := range []struct {
		name          string
		execKind      string
		onUnsupported string
		deps          *Deps
		execFlags     []string
		want          []string
	}{
		{"bun switch to deno", KindBun, "switch", off, bunExec, []string{"--cached-only"}},
		{"node switch to deno", KindNode, "switch", off, nil, []string{"--cached-only"}},
		{"deno keeps its execution flags", KindDeno, "switch", off,
			[]string{"--cached-only", "--config", "/w/deno.json"},
			[]string{"--cached-only", "--config", "/w/deno.json"}},
		{"warn leaves the execution flags", KindBun, "warn", off, bunExec, []string{"--no-install"}},
		{"no deps leaves the execution flags", KindBun, "switch", nil, bunExec, []string{"--no-install"}},
	} {
		t.Run(tc.name, func(t *testing.T) {
			got := checkerDepsFlags(tc.execKind, tc.onUnsupported, tc.deps, tc.execFlags)
			if strings.Join(got, " ") != strings.Join(tc.want, " ") {
				t.Fatalf("checkerDepsFlags = %v, want %v", got, tc.want)
			}
			// The checker's own filtering must keep the guard: this is
			// the argument list deno check is handed.
			resolved := preflight.ResolutionArgs(append([]string{"check"}, got...))
			hasCachedOnly := false
			for _, a := range resolved {
				if a == "--cached-only" {
					hasCachedOnly = true
				}
			}
			wantCached := strings.Contains(strings.Join(tc.want, " "), "--cached-only")
			if hasCachedOnly != wantCached {
				t.Fatalf("deno check args %v contain --cached-only = %v, want %v",
					resolved, hasCachedOnly, wantCached)
			}
		})
	}
}

// TestReportWarningReachesEverySink is the Codex finding on PR #6111:
// a non-fatal check warning must reach not only the diagnostic writer
// (which a daemon reads only on a non-zero exit) but any structured
// warning sink, so a warn-policy diagnostic survives a successful run.
func TestReportWarningReachesEverySink(t *testing.T) {
	var buf bytes.Buffer
	var got []string
	opts := Options{Stderr: &buf, OnWarning: func(m string) { got = append(got, m) }}
	reportWarning(opts, preflight.Problem{What: "the program does not type check", Where: "script.check"})
	if want := "mcpx: script.check: the program does not type check"; !strings.Contains(buf.String(), want) {
		t.Fatalf("diagnostic writer got %q, want it to contain %q", buf.String(), want)
	}
	if len(got) != 1 || !strings.Contains(got[0], "does not type check") {
		t.Fatalf("OnWarning got %v, want one diagnostic", got)
	}
	// No structured sink installed: the writer still gets it, and
	// nothing panics. This is the direct CLI path.
	buf.Reset()
	reportWarning(Options{Stderr: &buf}, preflight.Problem{What: "x"})
	if !strings.Contains(buf.String(), "mcpx: x") {
		t.Fatalf("writer got %q, want it to contain the diagnostic", buf.String())
	}
}
