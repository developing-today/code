package e2e_test

import (
	"bufio"
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
	"proto.askTimeout":           "TestAskTimeoutReleasesAHeldRequestFromTheEnvironment",
	"proto.stateTTL":             "TestStateTTLExpiresAResumableRequestFromTheEnvironment",
	"proto.sessionIdle":          "TestSessionIdleReapsAnUnusedSessionFromTheEnvironment",
	"proto.mcpPath":              "TestMCPPathMovesTheEndpointFromTheEnvironment",
	"proto.native":               "TestNativeAsksTheCallingClientFromTheEnvironment",
	"protoMessages.listMaxAge":   "TestListMaxAgeSetsTheListCacheHintFromTheEnvironment",
	"protoMessages.readMaxAge":   "TestReadMaxAgeSetsTheReadCacheHintFromTheEnvironment",
	"protoMessages.taskAfter":    "TestTaskAfterHandsASlowCallATaskFromTheEnvironment",
	"protoTasks.pollInterval":    "TestTaskPollIntervalSetsTheTaskPollHintFromTheEnvironment",
	"spec.first":                 "TestTheRevisionPolicyDecidesWhetherProgressKeepsItsMessage",
	"spec.lenient":               "TestTheRevisionPolicyDecidesWhetherAServerInitiatedRequestIsAnswered",
	"spec.precedence":            "TestSpecPrecedenceDecidesWhetherProgressKeepsItsMessageFromTheEnvironment",
	"exec.maxOutputBytes":        "TestExecMaxOutputBytesCapsStdoutFromTheEnvironment",
	"exec.timeout":               "TestExecTimeoutStopsALongScriptFromTheEnvironment",
	"exec.output":                "TestExecOutputShapesTheResultFromTheEnvironment",
	"mcp.pageSize":               "TestMCPPageSizePagesToolsListFromTheEnvironment",
	"mcp.passthrough":            "TestMCPPassthroughServesANamespaceDirectlyFromTheEnvironment",
	"completion.perMinute":       "TestCompletionPerMinuteLimitsOneCallerFromTheEnvironment",
	"completion.serverPerMinute": "TestCompletionServerPerMinuteLimitsAllCallersFromTheEnvironment",
	"logstore.queryLimit":        "TestLogstoreQueryLimitBoundsALogQueryFromTheEnvironment",
	"proto.serveMCP":             "TestServeMCPUnmountsTheEndpointFromTheEnvironment",
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
	"TestLoggingFileOffWritesNoDurableLog":                                      TestLoggingFileOffWritesNoDurableLog,
	"TestOutputJSONIsReachableFromTheEnvironment":                               TestOutputJSONIsReachableFromTheEnvironment,
	"TestPathsConfigFileChangesWhichConfigLoads":                                TestPathsConfigFileChangesWhichConfigLoads,
	"TestPathsScriptsFindsRecipesThatLiveOnlyThere":                             TestPathsScriptsFindsRecipesThatLiveOnlyThere,
	"TestPathsCacheChangesTheDoctorDirectory":                                   TestPathsCacheChangesTheDoctorDirectory,
	"TestPathsPluginsDiscoversAPluginThatLivesOnlyThere":                        TestPathsPluginsDiscoversAPluginThatLivesOnlyThere,
	"TestProxyUpstreamTimeoutIsReachableFromTheEnvironment":                     TestProxyUpstreamTimeoutIsReachableFromTheEnvironment,
	"TestPoolDefaultsAreReachableFromTheEnvironment":                            TestPoolDefaultsAreReachableFromTheEnvironment,
	"TestRegistryLimitReachesTheCLI":                                            TestRegistryLimitReachesTheCLI,
	"TestRegistryPageSizeChangesWhatTheCLIAsksFor":                              TestRegistryPageSizeChangesWhatTheCLIAsksFor,
	"TestScriptCaptureConsoleOffDropsConsoleOutput":                             TestScriptCaptureConsoleOffDropsConsoleOutput,
	"TestScriptCwdIsReachableFromTheEnvironment":                                TestScriptCwdIsReachableFromTheEnvironment,
	"TestScriptInterpreterIsReachableFromTheEnvironment":                        TestScriptInterpreterIsReachableFromTheEnvironment,
	"TestScriptEnvIsReachableFromTheEnvironment":                                TestScriptEnvIsReachableFromTheEnvironment,
	"TestScriptDevshellIsReachableFromTheEnvironment":                           TestScriptDevshellIsReachableFromTheEnvironment,
	"TestScriptPrefixIsReachableFromTheEnvironment":                             TestScriptPrefixIsReachableFromTheEnvironment,
	"TestScriptTypecheckOnRefusesATypeError":                                    TestScriptTypecheckOnRefusesATypeError,
	"TestSearchLimitCapsTheResults":                                             TestSearchLimitCapsTheResults,
	"TestStateDirectoryIsReachableAsASetting":                                   TestStateDirectoryIsReachableAsASetting,
	"TestStrictUnknownKeysRefusesATypo":                                         TestStrictUnknownKeysRefusesATypo,
	"TestTheRegistryPageCapSaysItStoppedEarly":                                  TestTheRegistryPageCapSaysItStoppedEarly,
	"TestDepsSourceNoneDropsTheManifestLayer":                                   TestDepsSourceNoneDropsTheManifestLayer,
	"TestDepsManifestNamesTheManifestResolutionReads":                           TestDepsManifestNamesTheManifestResolutionReads,
	"TestDepsPackagesPinContradictingTheManifestFailsTheRun":                    TestDepsPackagesPinContradictingTheManifestFailsTheRun,
	"TestPathsDepsExtendsManifestDiscovery":                                     TestPathsDepsExtendsManifestDiscovery,
	"TestTasksMaxAgeIsReachableFromEnvironment":                                 TestTasksMaxAgeIsReachableFromEnvironment,
	"TestStatsTopLimitsTheRowsTheStatsAPIReports":                               TestStatsTopLimitsTheRowsTheStatsAPIReports,
	"TestAskPollCeilingIsReachableFromTheEnvironment":                           TestAskPollCeilingIsReachableFromTheEnvironment,
	"TestAskRoundsLimitIsReachableFromTheEnvironment":                           TestAskRoundsLimitIsReachableFromTheEnvironment,
	"TestAskTTLBoundsAnInterruptibleCallFromTheEnvironment":                     TestAskTTLBoundsAnInterruptibleCallFromTheEnvironment,
	"TestAskTimeoutReleasesAHeldRequestFromTheEnvironment":                      TestAskTimeoutReleasesAHeldRequestFromTheEnvironment,
	"TestStateTTLExpiresAResumableRequestFromTheEnvironment":                    TestStateTTLExpiresAResumableRequestFromTheEnvironment,
	"TestSessionIdleReapsAnUnusedSessionFromTheEnvironment":                     TestSessionIdleReapsAnUnusedSessionFromTheEnvironment,
	"TestMCPPathMovesTheEndpointFromTheEnvironment":                             TestMCPPathMovesTheEndpointFromTheEnvironment,
	"TestNativeAsksTheCallingClientFromTheEnvironment":                          TestNativeAsksTheCallingClientFromTheEnvironment,
	"TestListMaxAgeSetsTheListCacheHintFromTheEnvironment":                      TestListMaxAgeSetsTheListCacheHintFromTheEnvironment,
	"TestReadMaxAgeSetsTheReadCacheHintFromTheEnvironment":                      TestReadMaxAgeSetsTheReadCacheHintFromTheEnvironment,
	"TestTaskAfterHandsASlowCallATaskFromTheEnvironment":                        TestTaskAfterHandsASlowCallATaskFromTheEnvironment,
	"TestTaskPollIntervalSetsTheTaskPollHintFromTheEnvironment":                 TestTaskPollIntervalSetsTheTaskPollHintFromTheEnvironment,
	"TestTheRevisionPolicyDecidesWhetherProgressKeepsItsMessage":                TestTheRevisionPolicyDecidesWhetherProgressKeepsItsMessage,
	"TestTheRevisionPolicyDecidesWhetherAServerInitiatedRequestIsAnswered":      TestTheRevisionPolicyDecidesWhetherAServerInitiatedRequestIsAnswered,
	"TestSpecPrecedenceDecidesWhetherProgressKeepsItsMessageFromTheEnvironment": TestSpecPrecedenceDecidesWhetherProgressKeepsItsMessageFromTheEnvironment,
	"TestExecMaxOutputBytesCapsStdoutFromTheEnvironment":                        TestExecMaxOutputBytesCapsStdoutFromTheEnvironment,
	"TestExecTimeoutStopsALongScriptFromTheEnvironment":                         TestExecTimeoutStopsALongScriptFromTheEnvironment,
	"TestExecOutputShapesTheResultFromTheEnvironment":                           TestExecOutputShapesTheResultFromTheEnvironment,
	"TestMCPPageSizePagesToolsListFromTheEnvironment":                           TestMCPPageSizePagesToolsListFromTheEnvironment,
	"TestMCPPassthroughServesANamespaceDirectlyFromTheEnvironment":              TestMCPPassthroughServesANamespaceDirectlyFromTheEnvironment,
	"TestCompletionPerMinuteLimitsOneCallerFromTheEnvironment":                  TestCompletionPerMinuteLimitsOneCallerFromTheEnvironment,
	"TestCompletionServerPerMinuteLimitsAllCallersFromTheEnvironment":           TestCompletionServerPerMinuteLimitsAllCallersFromTheEnvironment,
	"TestLogstoreQueryLimitBoundsALogQueryFromTheEnvironment":                   TestLogstoreQueryLimitBoundsALogQueryFromTheEnvironment,
	"TestServeMCPUnmountsTheEndpointFromTheEnvironment":                         TestServeMCPUnmountsTheEndpointFromTheEnvironment,
	"TestDaemonAddressIsReachableFromTheEnvironment":                            TestDaemonAddressIsReachableFromTheEnvironment,
	"TestDaemonAutostartIsReachableFromTheEnvironment":                          TestDaemonAutostartIsReachableFromTheEnvironment,
	"TestHTTPBodyLimitsRejectOversizedBodies":                                   TestHTTPBodyLimitsRejectOversizedBodies,
	"TestHTTPEventStreamSettingsChangeTheStream":                                TestHTTPEventStreamSettingsChangeTheStream,
	"TestHTTPReadHeaderTimeoutClosesSlowClients":                                TestHTTPReadHeaderTimeoutClosesSlowClients,
	"TestHTTPRequestTimeoutBoundsAClientCall":                                   TestHTTPRequestTimeoutBoundsAClientCall,
	"TestHTTPShutdownGraceBoundsHowLongAStopWaits":                              TestHTTPShutdownGraceBoundsHowLongAStopWaits,
	"TestHTTPStreamBufferMaxEndsAnOversizedEventLine":                           TestHTTPStreamBufferMaxEndsAnOversizedEventLine,
	"TestTransportAllowedOriginsAreServed":                                      TestTransportAllowedOriginsAreServed,
	"TestTransportBearerTokenGatesTheDaemon":                                    TestTransportBearerTokenGatesTheDaemon,
	"TestTransportSSEKeepAliveCommentsOnAQuietStream":                           TestTransportSSEKeepAliveCommentsOnAQuietStream,
	"TestTransportSessionMetaKeysNameTheSession":                                TestTransportSessionMetaKeysNameTheSession,
	"TestTransportStdioDrainBoundsServeShutdown":                                TestTransportStdioDrainBoundsServeShutdown,
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

// proto.askTimeout bounds how long the daemon's /mcp holds one request
// while the question it put to its client goes unanswered. A 2025-11-25
// client that declared form elicitation calls a tool that asks; it is sent
// elicitation/create on the stream and never answers.
// Oracle: at the default (10m) the stream carries no final frame within
// six seconds; with MCPX_PROTO_ASK_TIMEOUT=1s it ends inside that window
// with the stopped-waiting error.
func TestAskTimeoutReleasesAHeldRequestFromTheEnvironment(t *testing.T) {
	final := func(env ...string) string {
		t.Helper()
		e := askEnv(t)
		e.setenv(env...)
		e.run("refresh")
		ep := e.endpoint(t)
		init := mcpPost(t, ep, "", map[string]any{
			"jsonrpc": "2.0", "id": 1, "method": "initialize",
			"params": map[string]any{
				"protocolVersion": "2025-11-25",
				"capabilities":    map[string]any{"elicitation": map[string]any{"form": map[string]any{}}},
			},
		})
		session := init.Header.Get("Mcp-Session-Id")
		_ = init.Body.Close()
		if session == "" {
			t.Fatal("a legacy client needs a session")
		}
		resp := mcpPost(t, ep, session, map[string]any{
			"jsonrpc": "2.0", "id": 2, "method": "tools/call",
			"params": map[string]any{"name": "mcpx_call",
				"arguments": map[string]any{"namespace": "ask", "tool": "need_repo"}},
		})
		frames := make(chan map[string]any, 8)
		go func() {
			defer close(frames)
			sc := bufio.NewScanner(resp.Body)
			for sc.Scan() {
				var frame map[string]any
				if line := sc.Text(); strings.HasPrefix(line, "data: ") &&
					json.Unmarshal([]byte(strings.TrimPrefix(line, "data: ")), &frame) == nil {
					frames <- frame
				}
			}
		}()
		defer func() { _ = resp.Body.Close() }()
		asked := false
		stop := time.After(6 * time.Second)
		for {
			select {
			case frame, open := <-frames:
				if !open {
					t.Fatalf("the stream ended without a final frame (asked: %v)", asked)
				}
				if frame["method"] == "elicitation/create" {
					asked = true // and never answered
					continue
				}
				if frame["id"] != nil && (frame["result"] != nil || frame["error"] != nil) {
					if !asked {
						t.Fatalf("the call ended before its client was asked: %s", dumpJSON(t, frame))
					}
					return dumpJSON(t, frame)
				}
			case <-stop:
				if !asked {
					t.Fatal("the client was never asked")
				}
				return ""
			}
		}
	}

	if got := final(); got != "" {
		t.Fatalf("at the default proto.askTimeout the request should still be held after six seconds, got %s", got)
	}
	if got := final("MCPX_PROTO_ASK_TIMEOUT=1s"); !strings.Contains(got, "did not finish before mcpx stopped waiting") {
		t.Fatalf("proto.askTimeout=1s should end the request with the stopped-waiting error, got %q", got)
	}
}

// proto.stateTTL is how long a requestState handed to a modern client
// stays resumable. The client is asked for a repository, waits three
// seconds, then resumes with its answer.
// Oracle: at the default (30m) the resumed call completes with the answer;
// with MCPX_PROTO_STATE_TTL=1s the resume is refused as expired.
func TestStateTTLExpiresAResumableRequestFromTheEnvironment(t *testing.T) {
	resume := func(env ...string) string {
		t.Helper()
		e := askEnv(t)
		e.setenv(env...)
		e.run("refresh")
		ep := e.endpoint(t)
		params := map[string]any{
			"name":      "mcpx_call",
			"arguments": map[string]any{"namespace": "ask", "tool": "need_repo"},
			"_meta": map[string]any{
				"io.modelcontextprotocol/protocolVersion": "2026-07-28",
				"io.modelcontextprotocol/clientCapabilities": map[string]any{
					"elicitation": map[string]any{"form": map[string]any{}},
				},
			},
		}
		post := func(id int) map[string]any {
			resp := mcpPost(t, ep, "", map[string]any{"jsonrpc": "2.0", "id": id, "method": "tools/call", "params": params})
			defer func() { _ = resp.Body.Close() }()
			var frame map[string]any
			if err := json.NewDecoder(resp.Body).Decode(&frame); err != nil {
				t.Fatal(err)
			}
			return frame
		}
		first := post(1)
		result, _ := first["result"].(map[string]any)
		state, _ := result["requestState"].(string)
		requests, _ := result["inputRequests"].(map[string]any)
		if result["resultType"] != "input_required" || state == "" || len(requests) != 1 {
			t.Fatalf("expected one resumable question, got %s", dumpJSON(t, first))
		}
		answers := map[string]any{}
		for id := range requests {
			answers[id] = map[string]any{"action": "accept", "content": map[string]any{"repo": "me/late"}}
		}
		time.Sleep(3 * time.Second)
		params["requestState"] = state
		params["inputResponses"] = answers
		return dumpJSON(t, post(2))
	}

	if got := resume(); !strings.Contains(got, `"resultType":"complete"`) || !strings.Contains(got, "me/late") {
		t.Fatalf("at the default proto.stateTTL a resume after three seconds should complete, got %s", got)
	}
	if got := resume("MCPX_PROTO_STATE_TTL=1s"); !strings.Contains(got, "requestState expired") {
		t.Fatalf("proto.stateTTL=1s should refuse a resume after three seconds, got %s", got)
	}
}

// proto.sessionIdle is how long an unused legacy Streamable HTTP session
// is kept. Idle sessions are reaped when a new one is opened, so the probe
// opens a session, leaves it alone for three seconds, opens a second one
// and then asks to end the first.
// Oracle: at the default (30m) the first session still exists and DELETE
// ends it (204); with MCPX_PROTO_SESSION_IDLE=1s it was reaped and DELETE
// answers 404.
func TestSessionIdleReapsAnUnusedSessionFromTheEnvironment(t *testing.T) {
	endStatus := func(env ...string) int {
		t.Helper()
		e := newEnv(t, oneServer)
		e.setenv(env...)
		e.run("ls")
		ep := endpointOf(t, e)
		first := mcpInit(t, ep)
		time.Sleep(3 * time.Second)
		_ = mcpInit(t, ep)
		req, err := http.NewRequest(http.MethodDelete, ep+"/mcp", nil)
		if err != nil {
			t.Fatal(err)
		}
		req.Header.Set("Mcp-Session-Id", first)
		resp, err := (&http.Client{Timeout: 30 * time.Second}).Do(req)
		if err != nil {
			t.Fatal(err)
		}
		_ = resp.Body.Close()
		return resp.StatusCode
	}

	if got := endStatus(); got != http.StatusNoContent {
		t.Fatalf("at the default proto.sessionIdle a session idle for three seconds should still end with 204, got %d", got)
	}
	if got := endStatus("MCPX_PROTO_SESSION_IDLE=1s"); got != http.StatusNotFound {
		t.Fatalf("proto.sessionIdle=1s should reap a session idle for three seconds, so DELETE should answer 404, got %d", got)
	}
}

// initializeStatus posts an initialize to path on the daemon and returns
// the HTTP status, so a probe can tell where MCP is mounted.
func initializeStatus(t *testing.T, ep, path string) int {
	t.Helper()
	body := `{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"t","version":"1"}}}`
	req, err := http.NewRequest(http.MethodPost, ep+path, strings.NewReader(body))
	if err != nil {
		t.Fatal(err)
	}
	req.Header.Set("Content-Type", "application/json")
	req.Header.Set("Accept", "application/json, text/event-stream")
	resp, err := (&http.Client{Timeout: 30 * time.Second}).Do(req)
	if err != nil {
		t.Fatal(err)
	}
	_ = resp.Body.Close()
	return resp.StatusCode
}

// proto.mcpPath is where the daemon mounts MCP.
// Oracle: at the default an initialize to /mcp succeeds and one to
// /tools/mcp does not; with MCPX_PROTO_MCP_PATH=/tools/mcp it is the
// other way round.
func TestMCPPathMovesTheEndpointFromTheEnvironment(t *testing.T) {
	statuses := func(env ...string) (atMCP, atMoved int) {
		t.Helper()
		e := newEnv(t, oneServer)
		e.setenv(env...)
		e.run("ls")
		ep := endpointOf(t, e)
		return initializeStatus(t, ep, "/mcp"), initializeStatus(t, ep, "/tools/mcp")
	}

	if atMCP, atMoved := statuses(); atMCP != http.StatusOK || atMoved == http.StatusOK {
		t.Fatalf("at the default proto.mcpPath MCP should answer at /mcp only, got /mcp=%d /tools/mcp=%d", atMCP, atMoved)
	}
	if atMCP, atMoved := statuses("MCPX_PROTO_MCP_PATH=/tools/mcp"); atMoved != http.StatusOK || atMCP == http.StatusOK {
		t.Fatalf("proto.mcpPath=/tools/mcp should move MCP there, got /mcp=%d /tools/mcp=%d", atMCP, atMoved)
	}
}

// proto.serveMCP mounts mcpx's own MCP server on the daemon.
// Oracle: at the default an initialize to /mcp succeeds; with
// MCPX_PROTO_SERVE_MCP=false the daemon still answers /v1 but not /mcp.
func TestServeMCPUnmountsTheEndpointFromTheEnvironment(t *testing.T) {
	status := func(env ...string) int {
		t.Helper()
		e := newEnv(t, oneServer)
		e.setenv(env...)
		e.run("ls")
		got := initializeStatus(t, endpointOf(t, e), "/mcp")
		t.Logf("serveMCP env=%v status=%d", env, got)
		return got
	}

	if got := status(); got != http.StatusOK {
		t.Fatalf("at the default proto.serveMCP the daemon should answer MCP at /mcp, got %d", got)
	}
	if got := status("MCPX_PROTO_SERVE_MCP=false"); got == http.StatusOK {
		t.Fatalf("proto.serveMCP=false should leave /mcp unmounted, got %d", got)
	}
}

// proto.native puts an upstream server's question to the client that made
// the call. A modern client that declared elicitation calls a tool whose
// server asks for a repository.
// Oracle: at the default the call comes back input_required with the
// question for the client to answer; with MCPX_PROTO_NATIVE=false the
// question goes to the broker instead, which has no answerer in the
// harness, so the call fails with the broker's refusal rather than coming
// back input_required.
func TestNativeAsksTheCallingClientFromTheEnvironment(t *testing.T) {
	first := func(env ...string) string {
		t.Helper()
		e := askEnv(t)
		e.setenv(env...)
		e.run("refresh")
		ep := e.endpoint(t)
		resp := mcpPost(t, ep, "", map[string]any{"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": map[string]any{
			"name":      "mcpx_call",
			"arguments": map[string]any{"namespace": "ask", "tool": "need_repo"},
			"_meta": map[string]any{
				"io.modelcontextprotocol/protocolVersion": "2026-07-28",
				"io.modelcontextprotocol/clientCapabilities": map[string]any{
					"elicitation": map[string]any{"form": map[string]any{}},
				},
			},
		}})
		defer func() { _ = resp.Body.Close() }()
		var frame map[string]any
		if err := json.NewDecoder(resp.Body).Decode(&frame); err != nil {
			t.Fatal(err)
		}
		return dumpJSON(t, frame)
	}

	if got := first(); !strings.Contains(got, `"resultType":"input_required"`) {
		t.Fatalf("at the default proto.native the client should be asked, got %s", got)
	}
	if got := first("MCPX_PROTO_NATIVE=false"); strings.Contains(got, `"resultType":"input_required"`) || !strings.Contains(got, "no agent answerer") {
		t.Fatalf("proto.native=false should send the question to the broker, not the client, got %s", got)
	}
}

// modernTTL sends one 2026-07-28 request to the daemon and returns the
// ttlMs cache hint on its result.
func modernTTL(t *testing.T, method string, params map[string]any, env ...string) any {
	t.Helper()
	e := newEnv(t, oneServer)
	e.setenv(env...)
	e.run("refresh")
	ep := e.endpoint(t)
	p := map[string]any{"_meta": modernMeta()}
	for k, v := range params {
		p[k] = v
	}
	r := mcpPost(t, ep, "", map[string]any{"jsonrpc": "2.0", "id": 1, "method": method, "params": p})
	doc := decodeJSON(t, r.Body)
	_ = r.Body.Close()
	res, _ := doc["result"].(map[string]any)
	if res == nil {
		t.Fatalf("%s returned no result:\n%s", method, dumpJSON(t, doc))
	}
	return res["ttlMs"]
}

// protoMessages.listMaxAge is the ttlMs on list results sent to a
// 2026-07-28 client.
// Oracle: resources/list carries ttlMs 60000 at the default (1m) and 7000
// with MCPX_PROTO_MESSAGES_LIST_MAX_AGE=7s.
func TestListMaxAgeSetsTheListCacheHintFromTheEnvironment(t *testing.T) {
	if got := modernTTL(t, "resources/list", nil); got != float64(60000) {
		t.Fatalf("at the default protoMessages.listMaxAge resources/list should carry ttlMs 60000, got %v", got)
	}
	if got := modernTTL(t, "resources/list", nil, "MCPX_PROTO_MESSAGES_LIST_MAX_AGE=7s"); got != float64(7000) {
		t.Fatalf("protoMessages.listMaxAge=7s should carry ttlMs 7000, got %v", got)
	}
}

// protoMessages.readMaxAge is the ttlMs on resources/read results sent to
// a 2026-07-28 client.
// Oracle: reading one of mcpx's own skill resources carries ttlMs 0 at the
// default and 9000 with MCPX_PROTO_MESSAGES_READ_MAX_AGE=9s.
func TestReadMaxAgeSetsTheReadCacheHintFromTheEnvironment(t *testing.T) {
	read := map[string]any{"uri": "skill://mcpx/mcpx-basics/SKILL.md"}
	if got := modernTTL(t, "resources/read", read); got != float64(0) {
		t.Fatalf("at the default protoMessages.readMaxAge resources/read should carry ttlMs 0, got %v", got)
	}
	if got := modernTTL(t, "resources/read", read, "MCPX_PROTO_MESSAGES_READ_MAX_AGE=9s"); got != float64(9000) {
		t.Fatalf("protoMessages.readMaxAge=9s should carry ttlMs 9000, got %v", got)
	}
}

// protoMessages.taskAfter is how long a tools/call runs in line before a
// 2026-07-28 client that declared the tasks extension is handed a task.
// The client runs a script that takes one second.
// Oracle: at the default (2s) the call is answered directly with
// resultType complete; with MCPX_PROTO_MESSAGES_TASK_AFTER=200ms it comes
// back as a task (resultType task with a taskId).
func TestTaskAfterHandsASlowCallATaskFromTheEnvironment(t *testing.T) {
	call := func(env ...string) map[string]any { t.Helper(); return slowTaskCall(t, env...) }

	if got := call(); got["resultType"] != "complete" || !strings.Contains(dumpJSON(t, got), "slept") {
		t.Fatalf("at the default protoMessages.taskAfter a one-second call should be answered in line, got %s", dumpJSON(t, got))
	}
	if got := call("MCPX_PROTO_MESSAGES_TASK_AFTER=200ms"); got["resultType"] != "task" || got["taskId"] == nil {
		t.Fatalf("protoMessages.taskAfter=200ms should hand a one-second call a task, got %s", dumpJSON(t, got))
	}
}

// protoTasks.pollInterval is the pollIntervalMs a task mcpx hands out
// carries. A one-second call is made into a task (taskAfter 200ms).
// Oracle: the task carries pollIntervalMs 1000 at the default (1s) and 250
// with MCPX_PROTO_TASKS_POLL_INTERVAL=250ms.
func TestTaskPollIntervalSetsTheTaskPollHintFromTheEnvironment(t *testing.T) {
	poll := func(env ...string) any {
		t.Helper()
		got := slowTaskCall(t, append([]string{"MCPX_PROTO_MESSAGES_TASK_AFTER=200ms"}, env...)...)
		if got["resultType"] != "task" {
			t.Fatalf("a one-second call past a 200ms threshold should be a task, got %s", dumpJSON(t, got))
		}
		return got["pollIntervalMs"]
	}

	if got := poll(); got != float64(1000) {
		t.Fatalf("at the default protoTasks.pollInterval a task should carry pollIntervalMs 1000, got %v", got)
	}
	if got := poll("MCPX_PROTO_TASKS_POLL_INTERVAL=250ms"); got != float64(250) {
		t.Fatalf("protoTasks.pollInterval=250ms should carry pollIntervalMs 250, got %v", got)
	}
}

// slowTaskCall has a 2026-07-28 client that declared the tasks extension
// run a script that takes one second, and returns the tools/call result.
func slowTaskCall(t *testing.T, env ...string) map[string]any {
	t.Helper()
	e := newEnv(t, oneServer)
	e.setenv(env...)
	e.run("refresh")
	ep := e.endpoint(t)
	resp := mcpPost(t, ep, "", map[string]any{"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": map[string]any{
		"name":      "mcpx_exec",
		"arguments": map[string]any{"source": "await new Promise((r) => setTimeout(r, 1000)); console.log(\"slept\")"},
		"_meta": map[string]any{
			"io.modelcontextprotocol/protocolVersion": "2026-07-28",
			"io.modelcontextprotocol/clientCapabilities": map[string]any{
				"extensions": map[string]any{"io.modelcontextprotocol/tasks": map[string]any{}},
			},
		},
	}})
	defer func() { _ = resp.Body.Close() }()
	doc := decodeJSON(t, resp.Body)
	res, _ := doc["result"].(map[string]any)
	if res == nil {
		t.Fatalf("no result: %s", dumpJSON(t, doc))
	}
	return res
}

// spec.precedence orders the MCP revisions whose rules win a conflict. A
// 2024-11-05 client is relayed an upstream's progress, which carries a
// message 2024-11-05 does not define.
// Oracle: at the default (2026-07-28 first) the message is kept; with
// MCPX_SPEC_PRECEDENCE=2024-11-05 that revision's rule governs and the
// message is stripped. TestTheRevisionPolicyDecidesWhetherProgressKeepsItsMessage
// makes the same decision through spec.first.
func TestSpecPrecedenceDecidesWhetherProgressKeepsItsMessageFromTheEnvironment(t *testing.T) {
	kept := func(env ...string) bool {
		t.Helper()
		e := newEnv(t, oneServer)
		e.setenv(env...)
		e.run("refresh")
		notes, res := relaySessionAs(t, e, "2024-11-05", nil,
			map[string]any{}, "info", map[string]any{"progressToken": "precedence-tok"})
		if res["result"] == nil {
			t.Fatalf("the call should still answer:\n%s", dumpJSON(t, res))
		}
		progress, withMessage := 0, 0
		for _, n := range notes {
			if n["method"] != "notifications/progress" {
				continue
			}
			progress++
			if p, _ := n["params"].(map[string]any); p["message"] != nil {
				withMessage++
			}
		}
		if progress == 0 {
			t.Fatalf("no progress was relayed: %v", notes)
		}
		return withMessage == progress
	}

	if !kept() {
		t.Fatal("at the default spec.precedence a relayed progress message should be kept for a 2024-11-05 client")
	}
	if kept("MCPX_SPEC_PRECEDENCE=2024-11-05") {
		t.Fatal("spec.precedence=2024-11-05 should strip the progress message for a 2024-11-05 client")
	}
}

// exec.maxOutputBytes caps the stdout a run returns, with a marker. A
// script prints 2000 bytes and the run is returned as one structured
// document (text output streams to the terminal as it is printed, so the
// returned stdout is what the cap bounds).
// Oracle: at the default (64KiB) all of it comes back unmarked; with
// MCPX_EXEC_MAX_OUTPUT_BYTES=500 it is cut, marked and flagged
// stdoutTruncated.
func TestExecMaxOutputBytesCapsStdoutFromTheEnvironment(t *testing.T) {
	result := func(env ...string) map[string]any {
		t.Helper()
		e := newEnv(t, oneServer)
		e.setenv(append([]string{"MCPX_EXEC_OUTPUT=structured"}, env...)...)
		got := e.run("exec", `console.log("x".repeat(2000))`)
		var doc map[string]any
		if err := json.Unmarshal([]byte(strings.TrimSpace(got)), &doc); err != nil {
			t.Fatalf("a structured run should print one JSON document, got %v:\n%s", err, firstN(got, 300))
		}
		return doc
	}

	if doc := result(); strings.Count(doc["stdout"].(string), "x") < 2000 || doc["stdoutTruncated"] == true {
		t.Fatalf("at the default exec.maxOutputBytes 2000 bytes should come back whole:\n%s", firstN(dumpJSON(t, doc), 300))
	}
	doc := result("MCPX_EXEC_MAX_OUTPUT_BYTES=500")
	if out, _ := doc["stdout"].(string); len(out) > 500 || !strings.Contains(out, "stdout truncated") || doc["stdoutTruncated"] != true {
		t.Fatalf("exec.maxOutputBytes=500 should cut the stdout to 500 bytes and mark it:\n%s", firstN(dumpJSON(t, doc), 800))
	}
}

// exec.timeout kills a script after this long. A script sleeps four
// seconds and then prints.
// Oracle: at the default (120s) it prints and succeeds; with
// MCPX_EXEC_TIMEOUT=1s it is stopped before printing and the run fails.
func TestExecTimeoutStopsALongScriptFromTheEnvironment(t *testing.T) {
	run := func(env ...string) (string, error) {
		t.Helper()
		e := newEnv(t, oneServer)
		e.setenv(env...)
		return e.try("exec", `await new Promise((r) => setTimeout(r, 4000)); console.log("woke")`)
	}

	if got, err := run(); err != nil || !strings.Contains(got, "woke") {
		t.Fatalf("at the default exec.timeout a four-second script should finish, got err=%v:\n%s", err, got)
	}
	if got, err := run("MCPX_EXEC_TIMEOUT=1s"); err == nil || strings.Contains(got, "woke") {
		t.Fatalf("exec.timeout=1s should stop a four-second script before it prints, got err=%v:\n%s", err, got)
	}
}

// exec.output picks the shape of what a run returns. A script prints one
// line.
// Oracle: at the default (text) the CLI prints the line bare; with
// MCPX_EXEC_OUTPUT=structured it prints one JSON document whose stdout
// field holds the line.
func TestExecOutputShapesTheResultFromTheEnvironment(t *testing.T) {
	out := func(env ...string) string {
		t.Helper()
		e := newEnv(t, oneServer)
		e.setenv(env...)
		return e.run("exec", `console.log("shaped")`)
	}

	if got := strings.TrimSpace(out()); got != "shaped" {
		t.Fatalf("at the default exec.output the CLI should print the line bare, got:\n%s", got)
	}
	got := out("MCPX_EXEC_OUTPUT=structured")
	var doc map[string]any
	if err := json.Unmarshal([]byte(strings.TrimSpace(got)), &doc); err != nil {
		t.Fatalf("exec.output=structured should print one JSON document, got %v:\n%s", err, got)
	}
	if s, _ := doc["stdout"].(string); !strings.Contains(s, "shaped") {
		t.Fatalf("the structured document should carry the line in stdout, got:\n%s", got)
	}
}

// daemonToolsList returns the first tools/list page the daemon's /mcp
// gives a 2025-11-25 session, under the given environment.
func daemonToolsList(t *testing.T, env ...string) map[string]any {
	t.Helper()
	e := newEnv(t, oneServer)
	e.setenv(env...)
	e.run("refresh")
	ep := e.endpoint(t)
	sid := mcpInit(t, ep)
	frame, _ := mcpRoundTrip(t, ep, `{"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}`, sid)
	res, _ := frame["result"].(map[string]any)
	if res == nil {
		t.Fatalf("tools/list returned no result:\n%s", dumpJSON(t, frame))
	}
	return res
}

// mcp.pageSize is how many tools one tools/list reply carries.
// Oracle: at the default (100) mcpx's handful of tools fit on one page
// with no nextCursor; with MCPX_MCP_PAGE_SIZE=2 the page holds two tools
// and a nextCursor.
func TestMCPPageSizePagesToolsListFromTheEnvironment(t *testing.T) {
	page := func(env ...string) (int, bool) {
		t.Helper()
		res := daemonToolsList(t, env...)
		tools, _ := res["tools"].([]any)
		cursor, _ := res["nextCursor"].(string)
		return len(tools), cursor != ""
	}

	if n, more := page(); n <= 2 || more {
		t.Fatalf("at the default mcp.pageSize every tool should fit on one page, got %d tools, nextCursor=%v", n, more)
	}
	if n, more := page("MCPX_MCP_PAGE_SIZE=2"); n != 2 || !more {
		t.Fatalf("mcp.pageSize=2 should give a two-tool page with a nextCursor, got %d tools, nextCursor=%v", n, more)
	}
}

// mcp.passthrough lists the namespaces whose tools mcpx serves directly
// rather than only behind its gateway tools.
// Oracle: at the default the fake server's echo tool is not on the
// daemon's tools/list; with MCPX_MCP_PASSTHROUGH=demo it is.
func TestMCPPassthroughServesANamespaceDirectlyFromTheEnvironment(t *testing.T) {
	listsEcho := func(env ...string) bool {
		t.Helper()
		res := daemonToolsList(t, env...)
		tools, _ := res["tools"].([]any)
		for _, tool := range tools {
			if m, _ := tool.(map[string]any); m["name"] == "echo" {
				return true
			}
		}
		return false
	}

	if listsEcho() {
		t.Fatal("at the default mcp.passthrough the demo server's echo tool should not be listed directly")
	}
	if !listsEcho("MCPX_MCP_PASSTHROUGH=demo") {
		t.Fatal("mcp.passthrough=demo should list the demo server's echo tool directly")
	}
}

// completionStatuses sends one /v1/complete per session name, in order,
// to the fake server, and returns each HTTP status.
func completionStatuses(t *testing.T, env []string, sessions ...string) []int {
	t.Helper()
	e := newEnv(t, oneServer)
	e.setenv(env...)
	e.run("refresh")
	client := e.socketClient(t)
	var out []int
	for _, sess := range sessions {
		body := fmt.Sprintf(`{"server":"demo","ref":{"type":"ref/prompt","name":"s"},"argument":{"name":"x","value":"sum"},"session":%q}`, sess)
		resp, err := client.Post("http://mcpx/v1/complete", "application/json", strings.NewReader(body))
		if err != nil {
			t.Fatal(err)
		}
		_ = resp.Body.Close()
		out = append(out, resp.StatusCode)
	}
	return out
}

// completion.perMinute bounds completions per caller and server.
// Oracle: two completions from one session both succeed at the default;
// with MCPX_COMPLETION_PER_MINUTE=1 the second is refused with 429.
func TestCompletionPerMinuteLimitsOneCallerFromTheEnvironment(t *testing.T) {
	if got := completionStatuses(t, nil, "one", "one"); got[0] != http.StatusOK || got[1] != http.StatusOK {
		t.Fatalf("at the default completion.perMinute two completions should both succeed, got %v", got)
	}
	if got := completionStatuses(t, []string{"MCPX_COMPLETION_PER_MINUTE=1"}, "one", "one"); got[0] != http.StatusOK || got[1] != http.StatusTooManyRequests {
		t.Fatalf("completion.perMinute=1 should refuse the same caller's second completion with 429, got %v", got)
	}
}

// completion.serverPerMinute bounds completions per server across callers.
// Oracle: completions from two different sessions both succeed at the
// default; with MCPX_COMPLETION_SERVER_PER_MINUTE=1 the second is refused
// with 429 although it is another caller's first.
func TestCompletionServerPerMinuteLimitsAllCallersFromTheEnvironment(t *testing.T) {
	if got := completionStatuses(t, nil, "one", "two"); got[0] != http.StatusOK || got[1] != http.StatusOK {
		t.Fatalf("at the default completion.serverPerMinute two callers' completions should both succeed, got %v", got)
	}
	if got := completionStatuses(t, []string{"MCPX_COMPLETION_SERVER_PER_MINUTE=1"}, "one", "two"); got[0] != http.StatusOK || got[1] != http.StatusTooManyRequests {
		t.Fatalf("completion.serverPerMinute=1 should refuse a second caller's completion with 429, got %v", got)
	}
}

// logstore.queryLimit is how many records GET /v1/log returns when the
// caller names no limit. Two calls leave well over two records.
// Oracle: at the default (100) the query returns more than two records;
// with MCPX_LOGSTORE_QUERY_LIMIT=2 it returns exactly two.
func TestLogstoreQueryLimitBoundsALogQueryFromTheEnvironment(t *testing.T) {
	records := func(env ...string) int {
		t.Helper()
		e := newEnv(t, oneServer)
		e.setenv(env...)
		e.run("refresh")
		e.run("call", "demo.echo", `{"message":"a"}`)
		e.run("call", "demo.echo", `{"message":"b"}`)
		resp, err := e.socketClient(t).Get("http://mcpx/v1/log")
		if err != nil {
			t.Fatal(err)
		}
		defer func() { _ = resp.Body.Close() }()
		doc := decodeJSON(t, resp.Body)
		recs, _ := doc["records"].([]any)
		return len(recs)
	}

	if n := records(); n <= 2 {
		t.Fatalf("at the default logstore.queryLimit the query should return every record of two calls (more than two), got %d", n)
	}
	if n := records("MCPX_LOGSTORE_QUERY_LIMIT=2"); n != 2 {
		t.Fatalf("logstore.queryLimit=2 should return exactly two records, got %d", n)
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
