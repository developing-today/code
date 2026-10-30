package e2e_test

import (
	"os"
	"path/filepath"
	"sort"
	"strings"
	"testing"

	"github.com/dezren39/mcpx/internal/settings"
)

// Effect probes, grouped by surface (#1971, Approach C).
//
// settings.EffectProbes declares, per setting, the e2e test that proves
// its value can change observable behaviour. This file keeps the mirror
// of that declaration and the probes that did not already exist. The
// sync test below fails if the two maps disagree in either direction,
// so a declared probe must really exist here or in unread_test.go /
// registry_test.go, and a probe written here must be declared - a
// setting may not silently lose its guard, and a guard may not silently
// stop guarding.
//
// Every probe sets its setting the way a user would - an MCPX_ variable
// or a config key, never a Go field - drives an ordinary caller, and
// asserts an observable difference with and without. New probes belong
// in the surface group that observes them, next to their neighbours.

var implementedEffectProbes = map[string]string{
	"pool.max":                   "TestPoolDefaultsAreReachableFromTheEnvironment",
	"pool.sharing":               "TestPoolDefaultsAreReachableFromTheEnvironment",
	"pool.scope":                 "TestPoolDefaultsAreReachableFromTheEnvironment",
	"output.json":                "TestOutputJSONIsReachableFromTheEnvironment",
	"script.prefix":              "TestScriptPrefixIsReachableFromTheEnvironment",
	"script.env":                 "TestScriptEnvIsReachableFromTheEnvironment",
	"script.captureConsole":      "TestScriptCaptureConsoleOffDropsConsoleOutput",
	"script.typecheck":           "TestScriptTypecheckOnRefusesATypeError",
	"logging.file":               "TestLoggingFileOffWritesNoDurableLog",
	"plumbing.strictUnknownKeys": "TestStrictUnknownKeysRefusesATypo",
	"paths.state":                "TestStateDirectoryIsReachableAsASetting",
	"paths.configFile":           "TestPathsConfigFileChangesWhichConfigLoads",
	"search.limit":               "TestSearchLimitCapsTheResults",
	"registry.pageSize":          "TestRegistryPageSizeChangesWhatTheCLIAsksFor",
	"registry.limit":             "TestRegistryLimitReachesTheCLI",
	"registry.maxPages":          "TestTheRegistryPageCapSaysItStoppedEarly",
	"registry.url":               "TestRegistryPageSizeChangesWhatTheCLIAsksFor",
}

func TestEffectProbeDeclarationsMatchTheProbesThatExist(t *testing.T) {
	declared := settings.EffectProbes
	var diff []string
	for path, name := range declared {
		if got, ok := implementedEffectProbes[path]; !ok {
			diff = append(diff, path+" is declared in settings.EffectProbes but has no e2e probe")
		} else if got != name {
			diff = append(diff, path+" is declared as "+name+" but implemented as "+got)
		}
	}
	for path := range implementedEffectProbes {
		if _, ok := declared[path]; !ok {
			diff = append(diff, path+" has an e2e probe but is not declared in settings.EffectProbes")
		}
	}
	sort.Strings(diff)
	if len(diff) > 0 {
		t.Fatalf("effect probe declaration drift:\n  %s", strings.Join(diff, "\n  "))
	}
}

// --- Surface: which configuration loads -------------------------------

// paths.configFile (MCPX_CONFIG) decides which file every other setting
// is read from. Point it at a different file and a different server set
// must be what loads - not the env's own .mcpx.json.
func TestPathsConfigFileChangesWhichConfigLoads(t *testing.T) {
	e := newEnv(t, oneServer)

	alt := filepath.Join(e.dir, "alt.json")
	body := "{\n  \"mcpServers\": {\n    \"other\": { \"command\": \"" + e.fake + "\" }\n  }\n}\n"
	if err := os.WriteFile(alt, []byte(body), 0o644); err != nil {
		t.Fatal(err)
	}

	saved := e.envVars
	var kept []string
	for _, kv := range saved {
		if !strings.HasPrefix(kv, "MCPX_CONFIG=") {
			kept = append(kept, kv)
		}
	}
	e.envVars = append(kept, "MCPX_CONFIG="+alt)
	defer func() { e.envVars = saved }()

	rows := e.configRows(t)
	if len(rows) != 1 || rows[0].Name != "other" {
		t.Fatalf("MCPX_CONFIG should have decided which config loads, got %+v", rows)
	}
}

// --- Surface: running a script ----------------------------------------

func TestScriptCaptureConsoleOffDropsConsoleOutput(t *testing.T) {
	// Two envs, not one: the daemon that serves exec resolves this
	// setting when it starts, so flipping the variable mid-env would
	// talk to a daemon that had already decided. Each env gets the
	// value a user would have set before running anything.
	on := newEnv(t, oneServer)
	// The observable is the record form: captured, console.info arrives
	// as a log record (it carries console=info); left alone, it prints
	// raw. Asserting on the marker text alone would pass both ways,
	// because the raw print contains it too.
	out := on.run("exec", `console.info("capture-marker")`)
	if !strings.Contains(out, "console=info") {
		t.Fatalf("console.info should be captured as a record by default:\n%s", out)
	}

	off := newEnv(t, oneServer)
	off.envVars = append(off.envVars, "MCPX_SCRIPT_CAPTURE_CONSOLE=false")
	out = off.run("exec", `console.info("capture-marker")`)
	if strings.Contains(out, "console=info") {
		t.Fatalf("script.captureConsole=false should leave console.* out of the record stream:\n%s", out)
	}
}

func TestScriptTypecheckOnRefusesATypeError(t *testing.T) {
	e := newEnv(t, oneServer)
	script := filepath.Join(e.dir, "bad.ts")
	src := "const n: number = \"not a number\";\n" +
		"export default function () { return n; }\n"
	if err := os.WriteFile(script, []byte(src), 0o644); err != nil {
		t.Fatal(err)
	}

	// Off by default: the annotation is stripped and the script runs.
	if out := e.run("run", script); !strings.Contains(out, "not a number") {
		t.Fatalf("with typecheck off the script should run:\n%s", out)
	}

	saved := e.envVars
	e.envVars = append(append([]string{}, saved...), "MCPX_SCRIPT_TYPECHECK=on")
	defer func() { e.envVars = saved }()

	out, err := e.try("run", script)
	if err == nil {
		t.Fatalf("script.typecheck=on should refuse a type error:\n%s", out)
	}
}

// --- Surface: searching the catalog -----------------------------------

func TestSearchLimitCapsTheResults(t *testing.T) {
	e := newEnv(t, oneServer)

	count := func(out string) int { return strings.Count(out, `"call":`) }

	full := e.run("--json", "search", "e")
	if n := count(full); n < 2 {
		t.Fatalf("the search fixture should match several tools, got %d:\n%s", n, full)
	}

	saved := e.envVars
	e.envVars = append(append([]string{}, saved...), "MCPX_SEARCH_LIMIT=1")
	defer func() { e.envVars = saved }()

	capped := e.run("--json", "search", "e")
	if n := count(capped); n != 1 {
		t.Fatalf("search.limit=1 should cap the results at one, got %d:\n%s", n, capped)
	}
}
