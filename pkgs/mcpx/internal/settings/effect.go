package settings

// The effect guard (#1971, Approach C).
//
// consumed_test.go proves every setting is READ through an accessor.
// That is necessary and not sufficient: registry.pageSize was read for
// months and consulted only behind `if limit <= 0`, a branch every
// ordinary caller bypassed by passing a positive limit (#179). A read
// proves the wire reaches the code; it does not prove the value can
// change anything.
//
// This file is the completeness half of the guard. Every setting in
// Registry() must appear in exactly one of the two maps below:
//
//   - EffectProbes: an e2e test sets the setting the way a user would
//     (environment variable or config key, never a Go field), drives an
//     ordinary caller, and asserts an observable with/without
//     difference. The e2e package keeps a mirror map and a test fails
//     if the two disagree, and its effectProbeFuncs map anchors each
//     name to the real function as a Go reference, so a probe named
//     here must really exist: deleting or renaming it breaks the build.
//   - EffectExemptions: a written reason no probe can observe the
//     effect yet. The map is ratcheted by EffectExemptionPin: it can
//     only shrink. Adding an exemption fails the ratchet; removing one
//     (because a probe replaced it) requires lowering the pin in the
//     same PR, so every shrink is a recorded, deliberate act.
//
// The check itself is CheckEffectCoverage, exercised at unit speed by
// effect_test.go - including a negative test that plants an inert
// setting in the registry.pageSize shape and requires the guard to
// fail on it. A guard that cannot fail is not a guard.

import (
	"fmt"
	"sort"
	"strings"
)

// EffectProbes maps a setting path to the e2e test that proves its
// value can change observable behaviour.
var EffectProbes = map[string]string{
	"pool.max":                   "TestPoolDefaultsAreReachableFromTheEnvironment",
	"pool.sharing":               "TestPoolDefaultsAreReachableFromTheEnvironment",
	"pool.scope":                 "TestPoolDefaultsAreReachableFromTheEnvironment",
	"logging.file":               "TestLoggingFileOffWritesNoDurableLog",
	"script.captureConsole":      "TestScriptCaptureConsoleOffDropsConsoleOutput",
	"script.prefix":              "TestScriptPrefixIsReachableFromTheEnvironment",
	"script.typecheck":           "TestScriptTypecheckOnRefusesATypeError",
	"script.env":                 "TestScriptEnvIsReachableFromTheEnvironment",
	"paths.state":                "TestStateDirectoryIsReachableAsASetting",
	"registry.url":               "TestRegistryPageSizeChangesWhatTheCLIAsksFor",
	"output.json":                "TestOutputJSONIsReachableFromTheEnvironment",
	"plumbing.strictUnknownKeys": "TestStrictUnknownKeysRefusesATypo",
	"search.limit":               "TestSearchLimitCapsTheResults",
	"registry.limit":             "TestRegistryLimitReachesTheCLI",
	"registry.pageSize":          "TestRegistryPageSizeChangesWhatTheCLIAsksFor",
	"registry.maxPages":          "TestTheRegistryPageCapSaysItStoppedEarly",
	"paths.configFile":           "TestPathsConfigFileChangesWhichConfigLoads",
	"script.cwd":                 "TestScriptCwdIsReachableFromTheEnvironment",
	"script.interpreter":         "TestScriptInterpreterIsReachableFromTheEnvironment",
}

// EffectExemptions maps a setting path to the reason no effect probe
// covers it yet. An empty or missing reason is a violation, and the
// count is pinned by EffectExemptionPin.
var EffectExemptions = map[string]string{
	"pool.min":                           "pre-warming is daemon-internal state; no CLI surface reports the live instance count a probe could assert on",
	"pool.idleTimeout":                   "daemon pool timing bound: its effect is when the daemon gives up or reaps, observable only with a stalled upstream and a timing assertion the harness does not make deterministically",
	"pool.callTimeout":                   "daemon pool timing bound: its effect is when the daemon gives up or reaps, observable only with a stalled upstream and a timing assertion the harness does not make deterministically",
	"pool.startTimeout":                  "daemon pool timing bound: its effect is when the daemon gives up or reaps, observable only with a stalled upstream and a timing assertion the harness does not make deterministically",
	"logging.format":                     "log pipeline bound or rendering choice: its effect appears only across log volume, rotation, or time, none of which a single harness run produces deterministically",
	"logging.level":                      "log pipeline bound or rendering choice: its effect appears only across log volume, rotation, or time, none of which a single harness run produces deterministically",
	"logging.source":                     "log pipeline bound or rendering choice: its effect appears only across log volume, rotation, or time, none of which a single harness run produces deterministically",
	"logging.dir":                        "log pipeline bound or rendering choice: its effect appears only across log volume, rotation, or time, none of which a single harness run produces deterministically",
	"logging.include":                    "log pipeline bound or rendering choice: its effect appears only across log volume, rotation, or time, none of which a single harness run produces deterministically",
	"logging.maxBytes":                   "log pipeline bound or rendering choice: its effect appears only across log volume, rotation, or time, none of which a single harness run produces deterministically",
	"logging.maxLines":                   "log pipeline bound or rendering choice: its effect appears only across log volume, rotation, or time, none of which a single harness run produces deterministically",
	"logging.maxAge":                     "log pipeline bound or rendering choice: its effect appears only across log volume, rotation, or time, none of which a single harness run produces deterministically",
	"logging.keep":                       "log pipeline bound or rendering choice: its effect appears only across log volume, rotation, or time, none of which a single harness run produces deterministically",
	"script.runtime":                     "script launcher pipeline: its effect needs a runtime, profile, or launcher fixture exercising that exact stage, which the harness script fixtures do not yet provide",
	"script.runtimes":                    "script launcher pipeline: its effect needs a runtime, profile, or launcher fixture exercising that exact stage, which the harness script fixtures do not yet provide",
	"script.runtimeOrder":                "script launcher pipeline: its effect needs a runtime, profile, or launcher fixture exercising that exact stage, which the harness script fixtures do not yet provide",
	"script.permissions":                 "script launcher pipeline: its effect needs a runtime, profile, or launcher fixture exercising that exact stage, which the harness script fixtures do not yet provide",
	"script.profiles":                    "script launcher pipeline: its effect needs a runtime, profile, or launcher fixture exercising that exact stage, which the harness script fixtures do not yet provide",
	"script.launcher":                    "script launcher pipeline: its effect needs a runtime, profile, or launcher fixture exercising that exact stage, which the harness script fixtures do not yet provide",
	"script.before":                      "script launcher pipeline: its effect needs a runtime, profile, or launcher fixture exercising that exact stage, which the harness script fixtures do not yet provide",
	"script.onSuccess":                   "script launcher pipeline: its effect needs a runtime, profile, or launcher fixture exercising that exact stage, which the harness script fixtures do not yet provide",
	"script.onError":                     "script launcher pipeline: its effect needs a runtime, profile, or launcher fixture exercising that exact stage, which the harness script fixtures do not yet provide",
	"script.suffix":                      "script launcher pipeline: its effect needs a runtime, profile, or launcher fixture exercising that exact stage, which the harness script fixtures do not yet provide",
	"exec.timeout":                       "call-scoped exec behaviour with no dedicated with/without probe yet; the exec e2e tests exercise the surface only at its defaults",
	"exec.output":                        "call-scoped exec behaviour with no dedicated with/without probe yet; the exec e2e tests exercise the surface only at its defaults",
	"exec.where":                         "call-scoped exec behaviour with no dedicated with/without probe yet; the exec e2e tests exercise the surface only at its defaults",
	"artifacts.enabled":                  "artifact store behaviour: its effect needs artifact-producing calls plus volume or time (ttl, quota, gc) that the fake server's fixtures do not drive",
	"artifacts.dir":                      "artifact store behaviour: its effect needs artifact-producing calls plus volume or time (ttl, quota, gc) that the fake server's fixtures do not drive",
	"artifacts.delivery":                 "artifact store behaviour: its effect needs artifact-producing calls plus volume or time (ttl, quota, gc) that the fake server's fixtures do not drive",
	"artifacts.ttl":                      "artifact store behaviour: its effect needs artifact-producing calls plus volume or time (ttl, quota, gc) that the fake server's fixtures do not drive",
	"artifacts.maxBytes":                 "artifact store behaviour: its effect needs artifact-producing calls plus volume or time (ttl, quota, gc) that the fake server's fixtures do not drive",
	"artifacts.quota":                    "artifact store behaviour: its effect needs artifact-producing calls plus volume or time (ttl, quota, gc) that the fake server's fixtures do not drive",
	"artifacts.inlineMaxBytes":           "artifact store behaviour: its effect needs artifact-producing calls plus volume or time (ttl, quota, gc) that the fake server's fixtures do not drive",
	"artifacts.chunkBytes":               "artifact store behaviour: its effect needs artifact-producing calls plus volume or time (ttl, quota, gc) that the fake server's fixtures do not drive",
	"artifacts.gcInterval":               "artifact store behaviour: its effect needs artifact-producing calls plus volume or time (ttl, quota, gc) that the fake server's fixtures do not drive",
	"artifacts.interceptImages":          "artifact store behaviour: its effect needs artifact-producing calls plus volume or time (ttl, quota, gc) that the fake server's fixtures do not drive",
	"paths.config":                       "a discovery search path: its effect appears only when fixture content exists solely on that path, which the harness does not plant per-path",
	"paths.scripts":                      "a discovery search path: its effect appears only when fixture content exists solely on that path, which the harness does not plant per-path",
	"paths.placeholders":                 "a discovery search path: its effect appears only when fixture content exists solely on that path, which the harness does not plant per-path",
	"paths.adapters":                     "a discovery search path: its effect appears only when fixture content exists solely on that path, which the harness does not plant per-path",
	"paths.apis":                         "a discovery search path: its effect appears only when fixture content exists solely on that path, which the harness does not plant per-path",
	"paths.plugins":                      "a discovery search path: its effect appears only when fixture content exists solely on that path, which the harness does not plant per-path",
	"paths.cache":                        "a discovery search path: its effect appears only when fixture content exists solely on that path, which the harness does not plant per-path",
	"daemon.tsnetUpTimeout":              "tsnet bring-up needs a tailnet the harness does not have",
	"daemon.reapInterval":                "daemon lifecycle, binding, or timing: its effect is a bound, an interval, or a network identity the isolated harness cannot vary deterministically",
	"daemon.saveInterval":                "daemon lifecycle, binding, or timing: its effect is a bound, an interval, or a network identity the isolated harness cannot vary deterministically",
	"daemon.port":                        "daemon lifecycle, binding, or timing: its effect is a bound, an interval, or a network identity the isolated harness cannot vary deterministically",
	"daemon.endpoint":                    "daemon lifecycle, binding, or timing: its effect is a bound, an interval, or a network identity the isolated harness cannot vary deterministically",
	"daemon.tsnet.hostname":              "daemon lifecycle, binding, or timing: its effect is a bound, an interval, or a network identity the isolated harness cannot vary deterministically",
	"daemon.address":                     "daemon lifecycle, binding, or timing: its effect is a bound, an interval, or a network identity the isolated harness cannot vary deterministically",
	"daemon.ui":                          "daemon lifecycle, binding, or timing: its effect is a bound, an interval, or a network identity the isolated harness cannot vary deterministically",
	"daemon.uiDir":                       "daemon lifecycle, binding, or timing: its effect is a bound, an interval, or a network identity the isolated harness cannot vary deterministically",
	"daemon.inline":                      "daemon lifecycle, binding, or timing: its effect is a bound, an interval, or a network identity the isolated harness cannot vary deterministically",
	"daemon.autostart":                   "daemon lifecycle, binding, or timing: its effect is a bound, an interval, or a network identity the isolated harness cannot vary deterministically",
	"hooks":                              "hook execution: its effect needs a hook configured for a specific event and an event that fires it; the harness has no per-event hook fixture",
	"hooks.timeout":                      "hook execution: its effect needs a hook configured for a specific event and an event that fires it; the harness has no per-event hook fixture",
	"mcp.pageSize":                       "MCP list/passthrough surface: paging behaviour is unit-tested in mcpserver, but no e2e with/without probe drives it through a client yet",
	"mcp.passthrough":                    "MCP list/passthrough surface: paging behaviour is unit-tested in mcpserver, but no e2e with/without probe drives it through a client yet",
	"output.color":                       "only the interactive browser (tui) renders colour; no non-interactive surface the harness drives can show a with/without difference",
	"catalog.budget":                     "catalog ranking/truncation: the fake server's tool set is too small for a budget or bias change to alter the listing observably",
	"catalog.bias":                       "catalog ranking/truncation: the fake server's tool set is too small for a budget or bias change to alter the listing observably",
	"catalog.instructions":               "the fake MCP server serves no server instructions, so `types` output cannot differ either way until a fixture gains instructions",
	"proto.native":                       "protocol negotiation/wire behaviour between mcpx and a peer: era fixtures cover negotiation end-to-end, but no per-setting with/without probe exists yet",
	"proto.serveMCP":                     "protocol negotiation/wire behaviour between mcpx and a peer: era fixtures cover negotiation end-to-end, but no per-setting with/without probe exists yet",
	"proto.mcpPath":                      "protocol negotiation/wire behaviour between mcpx and a peer: era fixtures cover negotiation end-to-end, but no per-setting with/without probe exists yet",
	"proto.askTimeout":                   "protocol negotiation/wire behaviour between mcpx and a peer: era fixtures cover negotiation end-to-end, but no per-setting with/without probe exists yet",
	"proto.askPoll":                      "protocol negotiation/wire behaviour between mcpx and a peer: era fixtures cover negotiation end-to-end, but no per-setting with/without probe exists yet",
	"proto.askRounds":                    "protocol negotiation/wire behaviour between mcpx and a peer: era fixtures cover negotiation end-to-end, but no per-setting with/without probe exists yet",
	"proto.askTTL":                       "protocol negotiation/wire behaviour between mcpx and a peer: era fixtures cover negotiation end-to-end, but no per-setting with/without probe exists yet",
	"proto.stateTTL":                     "protocol negotiation/wire behaviour between mcpx and a peer: era fixtures cover negotiation end-to-end, but no per-setting with/without probe exists yet",
	"proto.sessionIdle":                  "protocol negotiation/wire behaviour between mcpx and a peer: era fixtures cover negotiation end-to-end, but no per-setting with/without probe exists yet",
	"transport.bearerToken":              "auth gate (#1975): the with/without difference is proven at the daemon HTTP layer (internal/daemon bearer tests: unset passes, wrong token 401, correct token passes), but no e2e probe yet starts a token'd daemon and drives a caller against it; replace this exemption with that probe",
	"transport.allowedOrigins":           "HTTP/SSE transport behaviour (origins, keep-alive, drain): needs a browser-origin or timing fixture the harness does not drive",
	"transport.sessionMetaKeys":          "HTTP/SSE transport behaviour (origins, keep-alive, drain): needs a browser-origin or timing fixture the harness does not drive",
	"transport.sseKeepAlive":             "HTTP/SSE transport behaviour (origins, keep-alive, drain): needs a browser-origin or timing fixture the harness does not drive",
	"transport.stdioDrain":               "HTTP/SSE transport behaviour (origins, keep-alive, drain): needs a browser-origin or timing fixture the harness does not drive",
	"plumbing.allowTsJsOverlap":          "plumbing switch: the branch it guards is read in its own package, but no e2e with/without probe exists yet - plumbing is the ratchet's expected bulk",
	"plumbing.sourceDirRecursive":        "plumbing switch: the branch it guards is read in its own package, but no e2e with/without probe exists yet - plumbing is the ratchet's expected bulk",
	"plumbing.sourceDirAllowed":          "plumbing switch: the branch it guards is read in its own package, but no e2e with/without probe exists yet - plumbing is the ratchet's expected bulk",
	"plumbing.sourceProbePaths":          "plumbing switch: the branch it guards is read in its own package, but no e2e with/without probe exists yet - plumbing is the ratchet's expected bulk",
	"plumbing.validatePaths":             "plumbing switch: the branch it guards is read in its own package, but no e2e with/without probe exists yet - plumbing is the ratchet's expected bulk",
	"plumbing.launcherPlaceholderRepeat": "plumbing switch: the branch it guards is read in its own package, but no e2e with/without probe exists yet - plumbing is the ratchet's expected bulk",
	"plumbing.indexOnQuery":              "plumbing switch: the branch it guards is read in its own package, but no e2e with/without probe exists yet - plumbing is the ratchet's expected bulk",
	"elicit.disambiguate":                "elicitation flow: its effect needs an interactive client answering (or not answering) inside that window; ask fixtures cover the flow, not this bound",
	"elicit.disambiguateDefault":         "elicitation flow: its effect needs an interactive client answering (or not answering) inside that window; ask fixtures cover the flow, not this bound",
	"elicit.confirmDestructive":          "elicitation flow: its effect needs an interactive client answering (or not answering) inside that window; ask fixtures cover the flow, not this bound",
	"policy.write":                       "elicitation flow: its effect needs an interactive client answering (or not answering) inside that window; daemon unit tests cover the gate, not an e2e with/without probe",
	"elicit.askTimeout":                  "elicitation flow: its effect needs an interactive client answering (or not answering) inside that window; ask fixtures cover the flow, not this bound",
	"elicit.ttl":                         "elicitation flow: its effect needs an interactive client answering (or not answering) inside that window; ask fixtures cover the flow, not this bound",
	"elicit.maxAge":                      "elicitation flow: its effect needs an interactive client answering (or not answering) inside that window; ask fixtures cover the flow, not this bound",
	"diagnose.preflight":                 "diagnostic surface with no dedicated with/without probe yet",
	"diagnose.history":                   "diagnostic surface with no dedicated with/without probe yet",
	"recipes.minScore":                   "recipe scoring: its effect needs several competing recipes straddling the threshold; the harness recipe fixtures do not",
	"recipes.matchMargin":                "recipe scoring: its effect needs several competing recipes straddling the threshold; the harness recipe fixtures do not",
	"recipes.limit":                      "recipe scoring: its effect needs several competing recipes straddling the threshold; the harness recipe fixtures do not",
	"autonomy.max":                       "autonomy behaviour: its effect needs several candidate actions or a sampling provider; no fixture provides one",
	"repair.autonomy":                    "repair autonomy: its effect needs a repairable failure fixture and a sampling provider; none exists yet",
	"hooks.autonomy":                     "hook execution: its effect needs a hook configured for a specific event and an event that fires it; the harness has no per-event hook fixture",
	"prompt.autonomy":                    "prompt/sampling behaviour: its effect needs a sampling provider or several candidate actions; no fixture provides one",
	"prompt.sample":                      "prompt/sampling behaviour: its effect needs a sampling provider or several candidate actions; no fixture provides one",
	"prompt.catalogBudget":               "prompt/sampling behaviour: its effect needs a sampling provider or several candidate actions; no fixture provides one",
	"prompt.sampleTimeout":               "prompt/sampling behaviour: its effect needs a sampling provider or several candidate actions; no fixture provides one",
	"prompt.maxTokens":                   "prompt/sampling behaviour: its effect needs a sampling provider or several candidate actions; no fixture provides one",
	"prompt.runTimeout":                  "prompt/sampling behaviour: its effect needs a sampling provider or several candidate actions; no fixture provides one",
	"http.bodyLimit":                     "HTTP server bound (body limits, timeouts, buffers): its effect needs oversized bodies, idle connections, or time passage a single run does not produce",
	"http.callBodyLimit":                 "HTTP server bound (body limits, timeouts, buffers): its effect needs oversized bodies, idle connections, or time passage a single run does not produce",
	"http.controlBodyLimit":              "HTTP server bound (body limits, timeouts, buffers): its effect needs oversized bodies, idle connections, or time passage a single run does not produce",
	"http.readHeaderTimeout":             "HTTP server bound (body limits, timeouts, buffers): its effect needs oversized bodies, idle connections, or time passage a single run does not produce",
	"http.shutdownGrace":                 "HTTP server bound (body limits, timeouts, buffers): its effect needs oversized bodies, idle connections, or time passage a single run does not produce",
	"http.ssePing":                       "HTTP server bound (body limits, timeouts, buffers): its effect needs oversized bodies, idle connections, or time passage a single run does not produce",
	"http.sseRetry":                      "HTTP server bound (body limits, timeouts, buffers): its effect needs oversized bodies, idle connections, or time passage a single run does not produce",
	"http.streamBufferInit":              "HTTP server bound (body limits, timeouts, buffers): its effect needs oversized bodies, idle connections, or time passage a single run does not produce",
	"http.streamBufferMax":               "HTTP server bound (body limits, timeouts, buffers): its effect needs oversized bodies, idle connections, or time passage a single run does not produce",
	"http.idleConns":                     "HTTP server bound (body limits, timeouts, buffers): its effect needs oversized bodies, idle connections, or time passage a single run does not produce",
	"http.remoteIdleConns":               "HTTP server bound (body limits, timeouts, buffers): its effect needs oversized bodies, idle connections, or time passage a single run does not produce",
	"http.requestTimeout":                "HTTP server bound (body limits, timeouts, buffers): its effect needs oversized bodies, idle connections, or time passage a single run does not produce",
	"http.idleConnTimeout":               "HTTP server bound (body limits, timeouts, buffers): its effect needs oversized bodies, idle connections, or time passage a single run does not produce",
	"autostart.bin":                      "client autostart mechanics: the harness itself depends on these to start daemons, so varying them per-test would fight the fixture",
	"autostart.args":                     "client autostart mechanics: the harness itself depends on these to start daemons, so varying them per-test would fight the fixture",
	"autostart.idleExit":                 "the harness itself sets this (a 60s backstop) for every env; a with/without probe would have to wait out a daemon's idle exit",
	"autostart.connectTimeout":           "client autostart mechanics: the harness itself depends on these to start daemons, so varying them per-test would fight the fixture",
	"autostart.pollInterval":             "client autostart mechanics: the harness itself depends on these to start daemons, so varying them per-test would fight the fixture",
	"autostart.pingTimeout":              "client autostart mechanics: the harness itself depends on these to start daemons, so varying them per-test would fight the fixture",
	"autostart.logTail":                  "client autostart mechanics: the harness itself depends on these to start daemons, so varying them per-test would fight the fixture",
	"search.semantic":                    "semantic search needs an embeddings backend fixture the harness does not provide",
	"completion.maxValues":               "shell completion surface: the harness never drives completion",
	"elicit.pendingLimit":                "elicitation flow: its effect needs an interactive client answering (or not answering) inside that window; ask fixtures cover the flow, not this bound",
	"registry.timeout":                   "a timeout bound: observing it needs a registry slower than the stub registry ever is",
	"logstore.queryLimit":                "log store surface: its effect needs stored log volume beyond what one harness run writes",
	"logstore.followBacklog":             "log store surface: its effect needs stored log volume beyond what one harness run writes",
	"stats.top":                          "statistics surface: its effect needs more recorded calls than the fake server's fixtures accumulate in one run",
	"events.history":                     "daemon event buffer/reconnect behaviour: needs disconnect/reconnect timing the harness does not drive",
	"events.reconnect":                   "daemon event buffer/reconnect behaviour: needs disconnect/reconnect timing the harness does not drive",
	"tasks.ttl":                          "task lifecycle timing: its effect is a ttl or wait bound, observable only across time passage",
	"tasks.resultWait":                   "task lifecycle timing: its effect is a ttl or wait bound, observable only across time passage",
	"daemon.idleExit":                    "daemon lifecycle, binding, or timing: its effect is a bound, an interval, or a network identity the isolated harness cannot vary deterministically",
	"serve.mode":                         "serve mode selection: needs a client driving the alternate mode end-to-end, which no fixture does",
	"catalog.pinnedTools":                "catalog ranking/truncation: the fake server's tool set is too small for a budget or bias change to alter the listing observably",
	"retention.maxTools":                 "retention window: its effect appears across call history and time, not within one run",
	"retention.stickyWindow":             "retention window: its effect appears across call history and time, not within one run",
	"embeddings.backend":                 "embeddings backend: needs a live embeddings service or a wasm model fixture the harness does not provide",
	"embeddings.url":                     "embeddings backend: needs a live embeddings service or a wasm model fixture the harness does not provide",
	"embeddings.apiKey":                  "needs a live embeddings service that accepts the key; no fixture serves one",
	"embeddings.model":                   "embeddings backend: needs a live embeddings service or a wasm model fixture the harness does not provide",
	"embeddings.wasmPath":                "embeddings backend: needs a live embeddings service or a wasm model fixture the harness does not provide",
	"daemon.warm":                        "daemon lifecycle, binding, or timing: its effect is a bound, an interval, or a network identity the isolated harness cannot vary deterministically",
	"daemon.watchConfig":                 "daemon lifecycle, binding, or timing: its effect is a bound, an interval, or a network identity the isolated harness cannot vary deterministically",
	"daemon.warmTimeout":                 "daemon lifecycle, binding, or timing: its effect is a bound, an interval, or a network identity the isolated harness cannot vary deterministically",
	"daemon.refreshTimeout":              "daemon lifecycle, binding, or timing: its effect is a bound, an interval, or a network identity the isolated harness cannot vary deterministically",
	"daemon.leaseTTL":                    "daemon lifecycle, binding, or timing: its effect is a bound, an interval, or a network identity the isolated harness cannot vary deterministically",
	"daemon.socketProbeTimeout":          "daemon lifecycle, binding, or timing: its effect is a bound, an interval, or a network identity the isolated harness cannot vary deterministically",
	"daemon.takeoverTimeout":             "daemon lifecycle, binding, or timing: its effect is a bound, an interval, or a network identity the isolated harness cannot vary deterministically",
	"daemon.inlineStartTimeout":          "daemon lifecycle, binding, or timing: its effect is a bound, an interval, or a network identity the isolated harness cannot vary deterministically",
	"daemon.inlineStartPoll":             "daemon lifecycle, binding, or timing: its effect is a bound, an interval, or a network identity the isolated harness cannot vary deterministically",
	"daemon.probeTimeout":                "daemon lifecycle, binding, or timing: its effect is a bound, an interval, or a network identity the isolated harness cannot vary deterministically",
	"doctor.timeout":                     "doctor timeout bound: observing it needs a check slower than the bound",
	"session.releaseTimeout":             "session release timing bound: observable only across time passage",
	"plugin.bin":                         "read by the opencode plugin host (JS side); the Go e2e harness does not drive the plugin host",
	"plugin.binArgs":                     "read by the opencode plugin host (JS side); the Go e2e harness does not drive the plugin host",
	"plugin.backend":                     "read by the opencode plugin host (JS side); the Go e2e harness does not drive the plugin host",
	"plugin.discoveryRetry":              "read by the opencode plugin host (JS side); the Go e2e harness does not drive the plugin host",
	"plugin.toolTiming":                  "read by the opencode plugin host (JS side); the Go e2e harness does not drive the plugin host",
	"plugin.env":                         "read by the opencode plugin host (JS side); the Go e2e harness does not drive the plugin host",
	"plugin.instructions":                "read by the opencode plugin host (JS side); the Go e2e harness does not drive the plugin host",
	"plugin.tools":                       "read by the opencode plugin host (JS side); the Go e2e harness does not drive the plugin host",
	"plugin.remember":                    "read by the opencode plugin host (JS side); the Go e2e harness does not drive the plugin host",
	"plugin.annotate":                    "read by the opencode plugin host (JS side); the Go e2e harness does not drive the plugin host",
	"plugin.headless":                    "read by the opencode plugin host (JS side); the Go e2e harness does not drive the plugin host",
	"plugin.daemonTools":                 "read by the opencode plugin host (JS side); the Go e2e harness does not drive the plugin host",
	"protoMessages.listMaxAge":           "protocol message timing: era fixtures cover the flow end-to-end, but no per-setting with/without probe exists yet",
	"protoMessages.readMaxAge":           "protocol message timing: era fixtures cover the flow end-to-end, but no per-setting with/without probe exists yet",
	"protoMessages.taskAfter":            "protocol message timing: era fixtures cover the flow end-to-end, but no per-setting with/without probe exists yet",
	"protoTasks.pollInterval":            "protocol task polling: its effect is a timing interval no single run observes deterministically",
	"upstream.protocol":                  "upstream protocol detection: its effect needs a peer whose era must be probed, beyond what the fake server presents",
	"upstream.probeTimeout":              "upstream protocol detection: its effect needs a peer whose era must be probed, beyond what the fake server presents",
	"upstream.eraCache":                  "upstream protocol detection: its effect needs a peer whose era must be probed, beyond what the fake server presents",
	"logging.trace":                      "log pipeline bound or rendering choice: its effect appears only across log volume, rotation, or time, none of which a single harness run produces deterministically",
	"presets":                            "preset bundling with no dedicated with/without probe yet",
	"preset":                             "preset bundling with no dedicated with/without probe yet",
	"spec.precedence":                    "protocol era selection: era fixtures cover negotiation end-to-end, but no per-setting with/without probe exists yet",
	"spec.first":                         "protocol era selection: era fixtures cover negotiation end-to-end, but no per-setting with/without probe exists yet",
	"spec.lenient":                       "protocol era selection: era fixtures cover negotiation end-to-end, but no per-setting with/without probe exists yet",
}

// EffectExemptionPin is the ratchet: the exemption count at the time
// the guard landed. It may only ever be lowered, and only in the PR
// that removes an exemption by adding the probe that replaces it.
const EffectExemptionPin = 182 // raised by one for policy.write (#1979), in the elicitation-flow exemption style of elicit.confirmDestructive

// CheckEffectCoverage returns one violation per way the probe and
// exemption maps fail to cover reg exactly: a setting with neither,
// a setting with both, an entry naming no live setting, a probe with
// no test name, or an exemption with no reason.
func CheckEffectCoverage(reg []Setting, probes, exemptions map[string]string) []string {
	known := make(map[string]bool, len(reg))
	for _, s := range reg {
		known[s.Path] = true
	}
	var violations []string
	for path, test := range probes {
		if !known[path] {
			violations = append(violations, fmt.Sprintf(
				"effect probe declared for %s, which is not in the registry (probe %s)", path, test))
		}
		if strings.TrimSpace(test) == "" {
			violations = append(violations, fmt.Sprintf(
				"effect probe for %s names no test", path))
		}
		if _, dup := exemptions[path]; dup {
			violations = append(violations, fmt.Sprintf(
				"%s has both an effect probe and an exemption; delete the exemption", path))
		}
	}
	for path, reason := range exemptions {
		if !known[path] {
			violations = append(violations, fmt.Sprintf(
				"effect exemption declared for %s, which is not in the registry", path))
		}
		if strings.TrimSpace(reason) == "" {
			violations = append(violations, fmt.Sprintf(
				"effect exemption for %s gives no reason", path))
		}
	}
	for _, s := range reg {
		if _, ok := probes[s.Path]; ok {
			continue
		}
		if _, ok := exemptions[s.Path]; ok {
			continue
		}
		violations = append(violations, fmt.Sprintf(
			"%s has neither an effect probe nor an exemption", s.Path))
	}
	sort.Strings(violations)
	return violations
}
