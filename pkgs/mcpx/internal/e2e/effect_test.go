package e2e_test

import (
	"net"
	"net/http"
	"net/url"
	"os"
	"path/filepath"
	"sort"
	"strings"
	"testing"
	"time"

	"github.com/developing-today/code/pkgs/mcpx/internal/settings"
)

// Effect probes, grouped by surface (#1971, Approach C).
//
// settings.EffectProbes declares, per setting, the e2e test that proves
// its value can change observable behaviour. This file keeps the mirror
// of that declaration and the probes that did not already exist. The
// sync test below fails if the two maps disagree in either direction,
// and effectProbeFuncs anchors each declared name to the real function
// as a Go reference, so a declared probe must really exist here or in
// unread_test.go / registry_test.go -- deleting or renaming one breaks
// the build -- and a probe written here must be declared: a setting may
// not silently lose its guard, and a guard may not silently stop
// guarding.
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
	"script.cwd":                 "TestScriptCwdIsReachableFromTheEnvironment",
	"script.interpreter":         "TestScriptInterpreterIsReachableFromTheEnvironment",
	"script.env":                 "TestScriptEnvIsReachableFromTheEnvironment",
	"script.devshell":            "TestScriptDevshellIsReachableFromTheEnvironment",
	"script.captureConsole":      "TestScriptCaptureConsoleOffDropsConsoleOutput",
	"script.check":               "TestScriptTypecheckOnRefusesATypeError",
	"logging.file":               "TestLoggingFileOffWritesNoDurableLog",
	"plumbing.strictUnknownKeys": "TestStrictUnknownKeysRefusesATypo",
	"paths.state":                "TestStateDirectoryIsReachableAsASetting",
	"paths.configFile":           "TestPathsConfigFileChangesWhichConfigLoads",
	"paths.scripts":              "TestPathsScriptsFindsRecipesThatLiveOnlyThere",
	"paths.cache":                "TestPathsCacheChangesTheDoctorDirectory",
	"paths.plugins":              "TestPathsPluginsDiscoversAPluginThatLivesOnlyThere",
	"search.limit":               "TestSearchLimitCapsTheResults",
	"registry.pageSize":          "TestRegistryPageSizeChangesWhatTheCLIAsksFor",
	"registry.limit":             "TestRegistryLimitReachesTheCLI",
	"registry.maxPages":          "TestTheRegistryPageCapSaysItStoppedEarly",
	"registry.url":               "TestRegistryPageSizeChangesWhatTheCLIAsksFor",
	"deps.source":                "TestDepsSourceNoneDropsTheManifestLayer",
	"deps.manifest":              "TestDepsManifestNamesTheManifestResolutionReads",
	"deps.packages":              "TestDepsPackagesPinContradictingTheManifestFailsTheRun",
	"paths.deps":                 "TestPathsDepsExtendsManifestDiscovery",
	"proxy.timeout":              "TestProxyUpstreamTimeoutIsReachableFromTheEnvironment",
	"tasks.maxAge":               "TestTasksMaxAgeIsReachableFromEnvironment",
	"stats.top":                  "TestStatsTopLimitsTheRowsTheStatsAPIReports",
	"proto.askPoll":              "TestAskPollCeilingIsReachableFromTheEnvironment",
	"daemon.address":             "TestDaemonAddressIsReachableFromTheEnvironment",
}

// effectProbeFuncs anchors every declared probe name to the actual
// test function, as a Go reference. The maps above are strings, and a
// string cannot notice when the function it names is deleted or
// renamed; this map can, because deleting or renaming a probe then
// breaks compilation of this package. The sync test requires every
// implemented probe name to appear here, so a declared probe must
// really exist -- not just be spelled the same in two maps.
var effectProbeFuncs = map[string]func(*testing.T){
	"TestLoggingFileOffWritesNoDurableLog":                   TestLoggingFileOffWritesNoDurableLog,
	"TestOutputJSONIsReachableFromTheEnvironment":            TestOutputJSONIsReachableFromTheEnvironment,
	"TestPathsConfigFileChangesWhichConfigLoads":             TestPathsConfigFileChangesWhichConfigLoads,
	"TestPathsScriptsFindsRecipesThatLiveOnlyThere":          TestPathsScriptsFindsRecipesThatLiveOnlyThere,
	"TestPathsCacheChangesTheDoctorDirectory":                TestPathsCacheChangesTheDoctorDirectory,
	"TestPathsPluginsDiscoversAPluginThatLivesOnlyThere":     TestPathsPluginsDiscoversAPluginThatLivesOnlyThere,
	"TestProxyUpstreamTimeoutIsReachableFromTheEnvironment":  TestProxyUpstreamTimeoutIsReachableFromTheEnvironment,
	"TestPoolDefaultsAreReachableFromTheEnvironment":         TestPoolDefaultsAreReachableFromTheEnvironment,
	"TestRegistryLimitReachesTheCLI":                         TestRegistryLimitReachesTheCLI,
	"TestRegistryPageSizeChangesWhatTheCLIAsksFor":           TestRegistryPageSizeChangesWhatTheCLIAsksFor,
	"TestScriptCaptureConsoleOffDropsConsoleOutput":          TestScriptCaptureConsoleOffDropsConsoleOutput,
	"TestScriptCwdIsReachableFromTheEnvironment":             TestScriptCwdIsReachableFromTheEnvironment,
	"TestScriptInterpreterIsReachableFromTheEnvironment":     TestScriptInterpreterIsReachableFromTheEnvironment,
	"TestScriptEnvIsReachableFromTheEnvironment":             TestScriptEnvIsReachableFromTheEnvironment,
	"TestScriptDevshellIsReachableFromTheEnvironment":        TestScriptDevshellIsReachableFromTheEnvironment,
	"TestScriptPrefixIsReachableFromTheEnvironment":          TestScriptPrefixIsReachableFromTheEnvironment,
	"TestScriptTypecheckOnRefusesATypeError":                 TestScriptTypecheckOnRefusesATypeError,
	"TestSearchLimitCapsTheResults":                          TestSearchLimitCapsTheResults,
	"TestStateDirectoryIsReachableAsASetting":                TestStateDirectoryIsReachableAsASetting,
	"TestStrictUnknownKeysRefusesATypo":                      TestStrictUnknownKeysRefusesATypo,
	"TestTheRegistryPageCapSaysItStoppedEarly":               TestTheRegistryPageCapSaysItStoppedEarly,
	"TestDepsSourceNoneDropsTheManifestLayer":                TestDepsSourceNoneDropsTheManifestLayer,
	"TestDepsManifestNamesTheManifestResolutionReads":        TestDepsManifestNamesTheManifestResolutionReads,
	"TestDepsPackagesPinContradictingTheManifestFailsTheRun": TestDepsPackagesPinContradictingTheManifestFailsTheRun,
	"TestPathsDepsExtendsManifestDiscovery":                  TestPathsDepsExtendsManifestDiscovery,
	"TestTasksMaxAgeIsReachableFromEnvironment":              TestTasksMaxAgeIsReachableFromEnvironment,
	"TestStatsTopLimitsTheRowsTheStatsAPIReports":            TestStatsTopLimitsTheRowsTheStatsAPIReports,
	"TestAskPollCeilingIsReachableFromTheEnvironment":        TestAskPollCeilingIsReachableFromTheEnvironment,
	"TestDaemonAddressIsReachableFromTheEnvironment":         TestDaemonAddressIsReachableFromTheEnvironment,
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
	seen := map[string]bool{}
	for _, name := range implementedEffectProbes {
		if seen[name] {
			continue
		}
		seen[name] = true
		if _, ok := effectProbeFuncs[name]; !ok {
			diff = append(diff, name+" is declared as a probe but is not anchored in effectProbeFuncs; anchor it so deleting the test breaks the build")
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

	saved := e.envVars
	defer func() { e.envVars = saved }()

	// Off: the annotation is stripped and the script runs. run checks by
	// default (#1868 point 4), so off has to be asked for.
	e.envVars = append(append([]string{}, saved...), "MCPX_SCRIPT_TYPECHECK=off")
	if out := e.run("run", script); !strings.Contains(out, "not a number") {
		t.Fatalf("with typecheck off the script should run:\n%s", out)
	}

	e.envVars = append(append([]string{}, saved...), "MCPX_SCRIPT_TYPECHECK=on")

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

// --- Surface: waiting on a question --------------------------------------

// proto.askPoll is the ceiling on how long one GET /v1/ask/{id} holds
// while a live call has nothing new to report. A script run that is
// still working and asks nothing is such a call, so with no waitMs the
// poll holds for exactly the ceiling: the hold time is the observable.
func TestAskPollCeilingIsReachableFromTheEnvironment(t *testing.T) {
	holdFor := func(env ...string) time.Duration {
		t.Helper()
		e := newEnv(t, oneServer)
		e.setenv(env...)
		e.run("ls")
		c := e.longSocketClient(t)
		_, begun := sockDo(t, c, http.MethodPost, "/v1/ask",
			map[string]any{"kind": "exec", "source": "await new Promise((r) => setTimeout(r, 8000));"})
		id, _ := begun["callId"].(string)
		if id == "" {
			t.Fatalf("no call id: %v", begun)
		}
		defer sockDo(t, c, http.MethodPost, "/v1/ask/"+id+"/abandon", nil)
		start := time.Now()
		st, out := sockDo(t, c, http.MethodGet, "/v1/ask/"+id, nil)
		took := time.Since(start)
		if st != http.StatusOK || out["done"] == true {
			t.Fatalf("the poll should return while the call is still running: %d %v", st, out)
		}
		return took
	}

	def := holdFor()
	raised := holdFor("MCPX_PROTO_ASK_POLL=2s")
	if raised < def+time.Second {
		t.Fatalf("proto.askPoll=2s should hold a poll about two seconds, got %v against %v at the default", raised, def)
	}
}

// TestDaemonAddressIsReachableFromTheEnvironment is the effect probe
// for daemon.address (#8468 chain 1). The exemption it replaced said
// a bind address is a network identity the harness cannot vary
// deterministically; it can: the daemon port is ephemeral (default
// 0), so two harness daemons bind different addresses without
// colliding, and the daemon itself reports the endpoint it bound.
//
// Oracle: the endpoint from `mcpx --json status` (endpointOf). At
// the default the reported host is loopback; with
// MCPX_DAEMON_ADDRESS=0.0.0.0 the reported host is the unspecified
// wildcard (0.0.0.0, reported as [::] on a dual-stack host) -- and
// the widened endpoint really answers an HTTP request, so the
// difference is the bind itself, not a string the daemon prints.
func TestDaemonAddressIsReachableFromTheEnvironment(t *testing.T) {
	endpointFor := func(env ...string) string {
		t.Helper()
		e := newEnv(t, oneServer)
		e.setenv(env...)
		e.run("ls")
		return endpointOf(t, e)
	}

	def := endpointFor()
	if !strings.HasPrefix(def, "http://127.0.0.1:") {
		t.Fatalf("oracle: default daemon.address should report a loopback endpoint, got %s", def)
	}

	wide := endpointFor("MCPX_DAEMON_ADDRESS=0.0.0.0")
	u, err := url.Parse(wide)
	if err != nil {
		t.Fatalf("oracle: unparseable widened endpoint %q: %v", wide, err)
	}
	host, port, err := net.SplitHostPort(u.Host)
	if err != nil {
		t.Fatalf("oracle: widened endpoint %q has no host:port: %v", wide, err)
	}
	if ip := net.ParseIP(host); ip == nil || !ip.IsUnspecified() {
		t.Fatalf("oracle: daemon.address=0.0.0.0 should report a wildcard endpoint, got %s (default was %s)", wide, def)
	}
	resp, err := http.Get("http://" + net.JoinHostPort("127.0.0.1", port) + "/v1/health")
	if err != nil {
		t.Fatalf("oracle: the widened endpoint should answer: %v", err)
	}
	defer func() { _ = resp.Body.Close() }()
	if resp.StatusCode != http.StatusOK {
		t.Fatalf("oracle: the widened endpoint answered %d, want 200", resp.StatusCode)
	}
}
