package e2e_test

import (
	"encoding/json"
	"fmt"
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
	"proto.askRounds":            "TestAskRoundsLimitIsReachableFromTheEnvironment",
	"proto.askTTL":               "TestAskTTLBoundsAnInterruptibleCallFromTheEnvironment",
	"daemon.address":             "TestDaemonAddressIsReachableFromTheEnvironment",
	"daemon.autostart":           "TestDaemonAutostartIsReachableFromTheEnvironment",
	"http.bodyLimit":             "TestHTTPBodyLimitsRejectOversizedBodies",
	"http.callBodyLimit":         "TestHTTPBodyLimitsRejectOversizedBodies",
	"http.controlBodyLimit":      "TestHTTPBodyLimitsRejectOversizedBodies",
	"http.ssePing":               "TestHTTPEventStreamSettingsChangeTheStream",
	"http.sseRetry":              "TestHTTPEventStreamSettingsChangeTheStream",
	"http.readHeaderTimeout":     "TestHTTPReadHeaderTimeoutClosesSlowClients",
	"http.shutdownGrace":         "TestHTTPShutdownGraceBoundsHowLongAStopWaits",
	"http.requestTimeout":        "TestHTTPRequestTimeoutBoundsAClientCall",
	"http.streamBufferMax":       "TestHTTPStreamBufferMaxEndsAnOversizedEventLine",
	"transport.bearerToken":      "TestTransportBearerTokenGatesTheDaemon",
	"transport.allowedOrigins":   "TestTransportAllowedOriginsAreServed",
	"transport.sessionMetaKeys":  "TestTransportSessionMetaKeysNameTheSession",
	"transport.sseKeepAlive":     "TestTransportSSEKeepAliveCommentsOnAQuietStream",
	"transport.stdioDrain":       "TestTransportStdioDrainBoundsServeShutdown",
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
	"TestAskRoundsLimitIsReachableFromTheEnvironment":        TestAskRoundsLimitIsReachableFromTheEnvironment,
	"TestAskTTLBoundsAnInterruptibleCallFromTheEnvironment":  TestAskTTLBoundsAnInterruptibleCallFromTheEnvironment,
	"TestDaemonAddressIsReachableFromTheEnvironment":         TestDaemonAddressIsReachableFromTheEnvironment,
	"TestDaemonAutostartIsReachableFromTheEnvironment":       TestDaemonAutostartIsReachableFromTheEnvironment,
	"TestHTTPBodyLimitsRejectOversizedBodies":                TestHTTPBodyLimitsRejectOversizedBodies,
	"TestHTTPEventStreamSettingsChangeTheStream":             TestHTTPEventStreamSettingsChangeTheStream,
	"TestHTTPReadHeaderTimeoutClosesSlowClients":             TestHTTPReadHeaderTimeoutClosesSlowClients,
	"TestHTTPRequestTimeoutBoundsAClientCall":                TestHTTPRequestTimeoutBoundsAClientCall,
	"TestHTTPShutdownGraceBoundsHowLongAStopWaits":           TestHTTPShutdownGraceBoundsHowLongAStopWaits,
	"TestHTTPStreamBufferMaxEndsAnOversizedEventLine":        TestHTTPStreamBufferMaxEndsAnOversizedEventLine,
	"TestTransportAllowedOriginsAreServed":                   TestTransportAllowedOriginsAreServed,
	"TestTransportBearerTokenGatesTheDaemon":                 TestTransportBearerTokenGatesTheDaemon,
	"TestTransportSSEKeepAliveCommentsOnAQuietStream":        TestTransportSSEKeepAliveCommentsOnAQuietStream,
	"TestTransportSessionMetaKeysNameTheSession":             TestTransportSessionMetaKeysNameTheSession,
	"TestTransportStdioDrainBoundsServeShutdown":             TestTransportStdioDrainBoundsServeShutdown,
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

// proto.askRounds limits how many times one MCP request may ask its client
// for input. The fake upstream asks twice in one tool call; default settings
// complete both rounds, while a configured limit of one rejects the second.
// Oracle: the default returns a complete result containing the second answer;
// a limit of one returns the ask-round exhaustion error instead.
func TestAskRoundsLimitIsReachableFromTheEnvironment(t *testing.T) {
	call := func(env ...string) string {
		t.Helper()
		e := askEnv(t)
		e.setenv(env...)
		e.run("refresh")
		ep := e.endpoint(t)
		meta := map[string]any{
			"io.modelcontextprotocol/protocolVersion": "2026-07-28",
			"io.modelcontextprotocol/clientCapabilities": map[string]any{
				"elicitation": map[string]any{"form": map[string]any{}},
			},
		}
		params := map[string]any{
			"name":      "mcpx_call",
			"arguments": map[string]any{"namespace": "ask", "tool": "need_repo_twice"},
			"_meta":     meta,
		}
		for attempt := 0; attempt < 3; attempt++ {
			resp := mcpPost(t, ep, "", map[string]any{
				"jsonrpc": "2.0", "id": attempt + 1, "method": "tools/call", "params": params,
			})
			var frame map[string]any
			if err := json.NewDecoder(resp.Body).Decode(&frame); err != nil {
				_ = resp.Body.Close()
				t.Fatal(err)
			}
			_ = resp.Body.Close()
			if frame["error"] != nil {
				return dumpJSON(t, frame)
			}
			result, _ := frame["result"].(map[string]any)
			if result["resultType"] == "complete" {
				return dumpJSON(t, frame)
			}
			if result["resultType"] != "input_required" {
				t.Fatalf("expected input_required or complete, got %s", dumpJSON(t, frame))
			}
			state, _ := result["requestState"].(string)
			requests, _ := result["inputRequests"].(map[string]any)
			if state == "" || len(requests) != 1 {
				t.Fatalf("expected one resumable elicitation request, got %s", dumpJSON(t, result))
			}
			responses := map[string]any{}
			for id := range requests {
				responses[id] = map[string]any{
					"action":  "accept",
					"content": map[string]any{"repo": fmt.Sprintf("me/round%d", attempt+1)},
				}
			}
			params["requestState"] = state
			params["inputResponses"] = responses
		}
		t.Fatal("two-round call did not complete or hit its configured limit")
		return ""
	}

	defaultResult := call()
	if !strings.Contains(defaultResult, `"resultType":"complete"`) || !strings.Contains(defaultResult, "me/round2") {
		t.Fatalf("default proto.askRounds should complete both rounds, got %s", defaultResult)
	}
	limitedResult := call("MCPX_PROTO_ASK_ROUNDS=1")
	if !strings.Contains(limitedResult, "still asking for input after 1 rounds") {
		t.Fatalf("proto.askRounds=1 should stop the second round, got %s", limitedResult)
	}
}

// proto.askTTL is the lifetime of an interruptible call begun on /v1/ask
// without its own ttl: the daemon cancels the call when it runs out. The
// call is a script that sleeps eight seconds, polled for up to four.
// Oracle: at the default (15m) the call is still running when the polls
// stop; with MCPX_PROTO_ASK_TTL=1s it has ended inside that window, its
// script stopped as timed out after about a second, not eight.
func TestAskTTLBoundsAnInterruptibleCallFromTheEnvironment(t *testing.T) {
	outcome := func(env ...string) map[string]any {
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
		var out map[string]any
		for stop := time.Now().Add(4 * time.Second); time.Now().Before(stop); {
			var st int
			st, out = sockDo(t, c, http.MethodGet, "/v1/ask/"+id+"?waitMs=500", nil)
			if st != http.StatusOK {
				t.Fatalf("poll: %d %v", st, out)
			}
			if out["done"] == true {
				break
			}
		}
		return out
	}

	if def := outcome(); def["done"] == true {
		t.Fatalf("at the default proto.askTTL the call should still be running after four seconds, got %v", def)
	}
	bounded := outcome("MCPX_PROTO_ASK_TTL=1s")
	run, _ := bounded["result"].(map[string]any)
	script, _ := run["result"].(map[string]any)
	took, _ := script["durationMs"].(float64)
	if bounded["done"] != true || script["timedOut"] != true || took <= 0 || took >= 4000 {
		t.Fatalf("proto.askTTL=1s should stop the call at its deadline, got %v", bounded)
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

// TestDaemonAutostartIsReachableFromTheEnvironment is the effect probe
// for daemon.autostart (#8468 chain 2). The exemption it replaced said
// daemon lifecycle cannot be varied deterministically; it can: with no
// daemon running, the same command either starts one and leaves its
// socket behind, or refuses and names the disabled rung.
//
// Oracle: the CLI's own no-daemon diagnosis. With autostart off, `ls`
// fails and names "disabled by daemon.autostart" and no socket
// appears; at the default, `ls` succeeds and leaves a socket.
func TestDaemonAutostartIsReachableFromTheEnvironment(t *testing.T) {
	socketsIn := func(e *env) []string {
		socks, _ := filepath.Glob(filepath.Join(e.dir, "state", "*.sock"))
		return socks
	}

	off := newEnv(t, oneServer)
	off.setenv("MCPX_DAEMON_AUTOSTART=false")
	out, err := off.try("ls")
	if err == nil {
		t.Fatalf("oracle: with daemon.autostart=false and no daemon, ls should fail; got:\n%s", out)
	}
	if !strings.Contains(out, "disabled by daemon.autostart") {
		t.Fatalf("oracle: the refusal should name the disabled autostart rung, got:\n%s", out)
	}
	if socks := socketsIn(off); len(socks) > 0 {
		t.Fatalf("oracle: no daemon socket should appear with autostart off: %v", socks)
	}

	on := newEnv(t, oneServer)
	on.run("ls")
	if socks := socketsIn(on); len(socks) == 0 {
		t.Fatalf("oracle: with the default daemon.autostart, ls should start a daemon and leave its socket")
	}
}
