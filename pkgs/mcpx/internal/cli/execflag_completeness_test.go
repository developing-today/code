package cli

import (
	"context"
	"errors"
	"flag"
	"os"
	"path/filepath"
	"testing"

	"github.com/dezren39/mcpx/internal/settings"
)

// TestExecFlagSetIsFullyProbed is the completeness half of #2188, and
// it lives here — unit-speed, milliseconds — instead of inside the
// e2e guard, so a new flag with no disposition fails locally at once.
//
// It enumerates the REAL exec and run FlagSets (the parseFlagsHook
// stops each handler the moment its flags are folded in, with the
// fully built FlagSet in hand), partitions the names into
// settings-registry flags (bound by parseFlags; their reach is
// guarded by TestEverySettingFlagReachesTheResolvedValue) and
// hand-written flags (defined in runScript), and requires every
// hand-written flag to be dispositioned in execflagcoverage.go:
// a probe row in TestEveryExecFlagChangesSomething, or a reasoned
// exemption. A flag bound by some third path lands in the
// hand-written bucket and fails here until it is dispositioned —
// that is the point: no flag joins exec/run silently again.
func TestExecFlagSetIsFullyProbed(t *testing.T) {
	dir := t.TempDir()
	t.Setenv("HOME", dir)
	t.Setenv("XDG_CONFIG_HOME", filepath.Join(dir, "xdg"))
	t.Setenv("MCPX_STATE_DIR", filepath.Join(dir, "state"))
	t.Setenv("MCPX_CACHE_DIR", filepath.Join(dir, "cache"))
	cfg := filepath.Join(dir, "mcpx.json")
	if err := os.WriteFile(cfg, []byte(`{"mcpServers":{}}`), 0o600); err != nil {
		t.Fatal(err)
	}

	sch, err := settings.New(settings.Registry())
	if err != nil {
		t.Fatal(err)
	}

	enumerate := func(cmd string) map[string]bool {
		var names map[string]bool
		parseFlagsHook = func(fs *flag.FlagSet) error {
			names = map[string]bool{}
			fs.VisitAll(func(f *flag.Flag) { names[f.Name] = true })
			return errStopAfterFlags
		}
		a := &App{ConfigPath: cfg}
		err := a.Handlers()[cmd](context.Background(), nil)
		parseFlagsHook = nil
		if !errors.Is(err, errStopAfterFlags) {
			t.Fatalf("%s: want the hook to stop the handler, got %v", cmd, err)
		}
		return names
	}

	probed := map[string]bool{}
	for _, name := range ExecFlagProbes {
		if probed[name] {
			t.Errorf("ExecFlagProbes lists %q twice", name)
		}
		if _, clash := ExecFlagExemptions[name]; clash {
			t.Errorf("%q is both probed and exempt; pick one", name)
		}
		probed[name] = true
	}
	for name, reason := range ExecFlagExemptions {
		if reason == "" {
			t.Errorf("exemption for %q has no reason", name)
		}
	}

	var prevHand map[string]bool
	for _, cmd := range []string{"exec", "run"} {
		all := enumerate(cmd)
		if len(all) < 100 {
			t.Fatalf("%s: only %d flags enumerated; the probe is not reaching the real FlagSet", cmd, len(all))
		}
		settingNames := map[string]bool{}
		for _, s := range sch.ForCommand(cmd) {
			settingNames[s.FlagName()] = true
			for _, alias := range s.FlagAliases {
				settingNames[alias] = true
			}
		}
		// A hand flag that shares a setting's name is tee-bound to
		// that setting by BindExcept, so its resolution is guarded by
		// flagreach — EXCEPT the handFlagsMeaningOther names, whose
		// hand meaning is the only one. Hand-written for this guard
		// means: not bound from the registry, or meaning-other.
		hand := map[string]bool{}
		for name := range all {
			if !settingNames[name] || handFlagsMeaningOther[cmd][name] {
				hand[name] = true
			}
		}
		// exec and run share runScript, so their hand sets must agree:
		// a flag on only one of them is exactly the shipped fault.
		if prevHand != nil {
			for name := range hand {
				if !prevHand[name] {
					t.Errorf("%s has hand flag %q that the other command lacks", cmd, name)
				}
			}
			for name := range prevHand {
				if !hand[name] {
					t.Errorf("%s lacks hand flag %q that the other command has", cmd, name)
				}
			}
		}
		prevHand = hand

		for name := range hand {
			if !probed[name] {
				if _, exempt := ExecFlagExemptions[name]; !exempt {
					t.Errorf("%s --%s: hand-written flag with no probe row and no exemption — add a row to TestEveryExecFlagChangesSomething or a reasoned exemption in execflagcoverage.go", cmd, name)
				}
			}
		}
		for name := range probed {
			if !all[name] {
				t.Errorf("ExecFlagProbes names %q, which is not an %s flag at all (renamed or removed?)", name, cmd)
			}
		}
		for name := range ExecFlagExemptions {
			if !all[name] {
				t.Errorf("ExecFlagExemptions names %q, which is not an %s flag at all (renamed or removed?)", name, cmd)
			}
		}
	}
}
