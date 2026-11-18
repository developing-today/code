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
	"pool.max":                           "TestPoolDefaultsAreReachableFromTheEnvironment",
	"pool.sharing":                       "TestPoolDefaultsAreReachableFromTheEnvironment",
	"pool.scope":                         "TestPoolDefaultsAreReachableFromTheEnvironment",
	"logging.file":                       "TestLoggingFileOffWritesNoDurableLog",
	"script.captureConsole":              "TestScriptCaptureConsoleOffDropsConsoleOutput",
	"script.prefix":                      "TestScriptPrefixIsReachableFromTheEnvironment",
	"script.check":                       "TestScriptTypecheckOnRefusesATypeError",
	"script.env":                         "TestScriptEnvIsReachableFromTheEnvironment",
	"script.devshell":                    "TestScriptDevshellIsReachableFromTheEnvironment",
	"paths.state":                        "TestStateDirectoryIsReachableAsASetting",
	"registry.url":                       "TestRegistryPageSizeChangesWhatTheCLIAsksFor",
	"output.json":                        "TestOutputJSONIsReachableFromTheEnvironment",
	"plumbing.strictUnknownKeys":         "TestStrictUnknownKeysRefusesATypo",
	"search.limit":                       "TestSearchLimitCapsTheResults",
	"registry.limit":                     "TestRegistryLimitReachesTheCLI",
	"registry.pageSize":                  "TestRegistryPageSizeChangesWhatTheCLIAsksFor",
	"registry.maxPages":                  "TestTheRegistryPageCapSaysItStoppedEarly",
	"paths.configFile":                   "TestPathsConfigFileChangesWhichConfigLoads",
	"paths.scripts":                      "TestPathsScriptsFindsRecipesThatLiveOnlyThere",
	"paths.cache":                        "TestPathsCacheChangesTheDoctorDirectory",
	"paths.plugins":                      "TestPathsPluginsDiscoversAPluginThatLivesOnlyThere",
	"tasks.maxAge":                       "TestTasksMaxAgeIsReachableFromEnvironment",
	"stats.top":                          "TestStatsTopLimitsTheRowsTheStatsAPIReports",
	"deps.source":                        "TestDepsSourceNoneDropsTheManifestLayer",
	"deps.manifest":                      "TestDepsManifestNamesTheManifestResolutionReads",
	"deps.packages":                      "TestDepsPackagesPinContradictingTheManifestFailsTheRun",
	"paths.deps":                         "TestPathsDepsExtendsManifestDiscovery",
	"script.cwd":                         "TestScriptCwdIsReachableFromTheEnvironment",
	"script.interpreter":                 "TestScriptInterpreterIsReachableFromTheEnvironment",
	"proxy.timeout":                      "TestProxyUpstreamTimeoutIsReachableFromTheEnvironment",
	"proto.askPoll":                      "TestAskPollCeilingIsReachableFromTheEnvironment",
	"proto.askRounds":                    "TestAskRoundsLimitIsReachableFromTheEnvironment",
	"proto.askTTL":                       "TestAskTTLBoundsAnInterruptibleCallFromTheEnvironment",
	"proto.askTimeout":                   "TestAskTimeoutReleasesAHeldRequestFromTheEnvironment",
	"proto.stateTTL":                     "TestStateTTLExpiresAResumableRequestFromTheEnvironment",
	"proto.sessionIdle":                  "TestSessionIdleReapsAnUnusedSessionFromTheEnvironment",
	"proto.mcpPath":                      "TestMCPPathMovesTheEndpointFromTheEnvironment",
	"proto.native":                       "TestNativeAsksTheCallingClientFromTheEnvironment",
	"protoMessages.listMaxAge":           "TestListMaxAgeSetsTheListCacheHintFromTheEnvironment",
	"protoMessages.readMaxAge":           "TestReadMaxAgeSetsTheReadCacheHintFromTheEnvironment",
	"protoMessages.taskAfter":            "TestTaskAfterHandsASlowCallATaskFromTheEnvironment",
	"protoTasks.pollInterval":            "TestTaskPollIntervalSetsTheTaskPollHintFromTheEnvironment",
	"spec.first":                         "TestTheRevisionPolicyDecidesWhetherProgressKeepsItsMessage",
	"spec.lenient":                       "TestTheRevisionPolicyDecidesWhetherAServerInitiatedRequestIsAnswered",
	"spec.precedence":                    "TestSpecPrecedenceDecidesWhetherProgressKeepsItsMessageFromTheEnvironment",
	"exec.maxOutputBytes":                "TestExecMaxOutputBytesCapsStdoutFromTheEnvironment",
	"exec.timeout":                       "TestExecTimeoutStopsALongScriptFromTheEnvironment",
	"exec.output":                        "TestExecOutputShapesTheResultFromTheEnvironment",
	"mcp.pageSize":                       "TestMCPPageSizePagesToolsListFromTheEnvironment",
	"mcp.passthrough":                    "TestMCPPassthroughServesANamespaceDirectlyFromTheEnvironment",
	"completion.perMinute":               "TestCompletionPerMinuteLimitsOneCallerFromTheEnvironment",
	"completion.serverPerMinute":         "TestCompletionServerPerMinuteLimitsAllCallersFromTheEnvironment",
	"logstore.queryLimit":                "TestLogstoreQueryLimitBoundsALogQueryFromTheEnvironment",
	"recipes.minScore":                   "TestRecipesMinScoreRefusesAWeakMatchFromTheEnvironment",
	"recipes.matchMargin":                "TestRecipesMatchMarginSettlesATieFromTheEnvironment",
	"recipes.limit":                      "TestRecipesLimitBoundsTheRankingFromTheEnvironment",
	"tasks.ttl":                          "TestTasksTTLSetsAnExecTasksRetentionFromTheEnvironment",
	"tasks.resultWait":                   "TestTasksResultWaitBoundsCollectingAResultFromTheEnvironment",
	"protoTasks.maxTTL":                  "TestProtoTasksMaxTTLClampsATasksRetentionFromTheEnvironment",
	"protoTasks.maxPerRequestor":         "TestProtoTasksMaxPerRequestorCapsUnfinishedTasksFromTheEnvironment",
	"protoTasks.perMinute":               "TestProtoTasksPerMinuteWindowsTaskReadsFromTheEnvironment",
	"events.history":                     "TestEventsHistoryBoundsTheReplay",
	"daemon.leaseTTL":                    "TestDaemonLeaseTTLForgetsASilentCallerFromTheEnvironment",
	"elicit.disambiguate":                "TestElicitDisambiguateAsksWhichInstanceFromTheEnvironment",
	"elicit.disambiguateDefault":         "TestElicitDisambiguateDefaultPicksTheRecentInstanceFromTheEnvironment",
	"elicit.ttl":                         "TestElicitTTLSetsAnUpstreamQuestionsDeadlineFromTheEnvironment",
	"elicit.maxAge":                      "TestElicitMaxAgeCapsAnUpstreamQuestionsDeadlineFromTheEnvironment",
	"elicit.pendingLimit":                "TestElicitPendingLimitBoundsTheListingFromTheEnvironment",
	"elicit.askTimeout":                  "TestElicitAskTimeoutBoundsTheWaitForAnAnswerFromTheEnvironment",
	"artifacts.idleTtl":                  "TestArtifactsIdleTTLRenewsAReadArtifactFromTheEnvironment",
	"artifacts.maxAge":                   "TestArtifactsMaxAgeCapsARenewedArtifactFromTheEnvironment",
	"artifacts.ttl":                      "TestArtifactsTTLExpiresAnArtifactFromTheEnvironment",
	"artifacts.gcInterval":               "TestArtifactsGCIntervalSetsWhenExpiredArtifactsAreSweptFromTheEnvironment",
	"pool.min":                           "TestPoolMinKeepsAnInstanceWarmFromTheEnvironment",
	"autostart.idleExit":                 "TestAutostartIdleExitStopsAnIdleDaemonFromTheEnvironment",
	"daemon.warm":                        "TestDaemonWarmReadsTheServersAtStartupFromTheEnvironment",
	"pool.idleTimeout":                   "TestPoolIdleTimeoutReapsAnIdleInstanceFromTheEnvironment",
	"daemon.reapInterval":                "TestDaemonReapIntervalSetsWhenIdleInstancesAreReapedFromTheEnvironment",
	"daemon.endpoint":                    "TestDaemonEndpointNamesTheDaemonToReachFromTheEnvironment",
	"upstream.probeTimeout":              "TestUpstreamProbeTimeoutBoundsTheDiscoverWaitFromTheEnvironment",
	"upstream.eraCache":                  "TestTheEraIsProbedOnceAndRememberedAcrossDaemonRestarts",
	"upstream.protocol":                  "TestTheEraIsProbedOnceAndRememberedAcrossDaemonRestarts",
	"prompt.autonomy":                    "TestPromptAutonomyRunsTheMatchedRecipeFromTheEnvironment",
	"hooks.autonomy":                     "TestHooksAutonomySkipsAConfiguredHookFromTheEnvironment",
	"repair.autonomy":                    "TestRepairAutonomyOffDropsTheDiagnosticsFromTheEnvironment",
	"policy.write":                       "TestPolicyWriteDeniesAnUnannotatedCallFromTheEnvironment",
	"elicit.confirmDestructive":          "TestElicitConfirmDestructiveGatesADestructiveCallFromTheEnvironment",
	"autonomy.max":                       "TestAutonomyMaxClampsARecipeRunFromTheEnvironment",
	"daemon.inline":                      "TestDaemonInlineAlwaysAnswersWithoutASocketFromTheEnvironment",
	"autostart.bin":                      "TestAutostartBinChoosesTheDaemonExecutableFromTheEnvironment",
	"autostart.args":                     "TestAutostartArgsReplaceTheDaemonCommandLineFromTheEnvironment",
	"autostart.logTail":                  "TestAutostartLogTailBoundsTheFailureExcerptFromTheEnvironment",
	"daemon.watchConfig":                 "TestDaemonWatchConfigPicksUpAHandEditFromTheEnvironment",
	"daemon.inlineStartTimeout":          "TestDaemonInlineStartTimeoutBoundsTheInProcessStartFromTheEnvironment",
	"daemon.probeTimeout":                "TestDaemonProbeTimeoutDecidesWhetherASiblingDaemonIsReportedFromTheEnvironment",
	"daemon.warmTimeout":                 "TestDaemonWarmTimeoutBoundsTheStartupWarm",
	"autostart.connectTimeout":           "TestAutostartConnectTimeoutBoundsTheWaitForADaemon",
	"autostart.pingTimeout":              "TestAutostartPingTimeoutBoundsEachReadinessPing",
	"pool.callTimeout":                   "TestPoolCallTimeoutBoundsASlowCall",
	"pool.startTimeout":                  "TestPoolStartTimeoutBoundsServerStart",
	"daemon.refreshTimeout":              "TestDaemonRefreshTimeoutBoundsARefresh",
	"doctor.timeout":                     "TestDoctorTimeoutBoundsTheDaemonCheck",
	"registry.timeout":                   "TestRegistryTimeoutBoundsARegistryRequest",
	"completion.maxValues":               "TestCompletionMaxValuesCapsTheValues",
	"catalog.instructions":               "TestCatalogInstructionsShowsServerGuidance",
	"plumbing.launcherPlaceholderRepeat": "TestPlumbingLauncherPlaceholderRepeatAllowsATwiceUsedName",
	"plumbing.allowTsJsOverlap":          "TestPlumbingAllowTsJsOverlapPicksAmbiguousScripts",
	"plumbing.sourceDirAllowed":          "TestPlumbingSourceDirAllowedTakesADirectoryAsSource",
	"plumbing.sourceDirRecursive":        "TestPlumbingSourceDirRecursiveReadsSubdirectories",
	"plumbing.sourceProbePaths":          "TestPlumbingSourceProbePathsReadsANamedFile",
	"artifacts.chunkBytes":               "TestArtifactsChunkBytesSplitsAStreamedBody",
	"script.launcher":                    "TestScriptLauncherReplacesTheLauncher",
	"paths.placeholders":                 "TestPathsPlaceholdersDeclaresALauncherPlaceholder",
	"artifacts.interceptImages":          "TestArtifactsInterceptImagesStoresAnEmittedImage",
	"plumbing.indexOnQuery":              "TestPlumbingIndexOnQueryIngestsBeforeALogQuery",
	"plumbing.validatePaths":             "TestPlumbingValidatePathsChecksTheLogDirUpFront",
	"script.language":                    "TestScriptLanguageRunsPython",
	"logging.trace":                      "TestLoggingTraceLogsEachCall",
	"exec.where":                         "TestExecWhereRemoteRunsOnTheDaemon",
	"paths.adapters":                     "TestPathsAdaptersFindsADeclarationFile",
	"paths.apis":                         "TestPathsAPIsFindsADeclarationFile",
	"script.runtime":                     "TestScriptRuntimeSelectsTheRuntime",
	"script.runtimeOrder":                "TestScriptRuntimeOrderRanksTheAutoPick",
	"script.permissions":                 "TestScriptPermissionsSandboxesNode",
	"script.profiles":                    "TestScriptProfilesDefinesAProfile",
	"script.runtimes":                    "TestScriptRuntimesDeclaresARuntime",
	"script.before":                      "TestScriptPhasesRunFromTheEnvironment",
	"script.onSuccess":                   "TestScriptPhasesRunFromTheEnvironment",
	"script.onError":                     "TestScriptPhasesRunFromTheEnvironment",
	"script.suffix":                      "TestScriptPhasesRunFromTheEnvironment",
	"hooks":                              "TestHooksRunOnTheirEvent",
	"hooks.timeout":                      "TestHooksTimeoutStopsASlowHook",
	"daemon.ui":                          "TestDaemonUITurnsTheDashboardOff",
	"daemon.uiDir":                       "TestDaemonUIDirOverridesTheDashboard",
	"logging.level":                      "TestLoggingLevelShowsDebugRecords",
	"logging.format":                     "TestLoggingFormatRendersJSON",
	"logging.source":                     "TestLoggingSourceAddsTheCallSite",
	"logging.dir":                        "TestLoggingDirHoldsTheDurableLog",
	"logging.include":                    "TestLoggingIncludeChoosesTheAmbientBlocks",
	"logging.maxLines":                   "TestLoggingMaxLinesRotatesTheFile",
	"logging.maxBytes":                   "TestLoggingMaxBytesRotatesTheFile",
	"logging.keep":                       "TestLoggingKeepPrunesRotatedFiles",
	"artifacts.enabled":                  "TestArtifactsEnabledGatesTheStore",
	"artifacts.maxBytes":                 "TestArtifactsMaxBytesRefusesALargeArtifact",
	"artifacts.quota":                    "TestArtifactsQuotaBoundsTheStore",
	"artifacts.delivery":                 "TestArtifactsDeliveryInlinesTheBody",
	"artifacts.inlineMaxBytes":           "TestArtifactsInlineMaxBytesBoundsAnInlinedBody",
	"artifacts.dir":                      "TestArtifactsDirReceivesTheRunsArtifacts",
	"preset":                             "TestPresetSelectsADefinedBundle",
	"serve.mode":                         "TestServeModeSelectsTheServedSurface",
	"presets":                            "TestPresetsDefinesNamedFlagBundles",
	"catalog.budget":                     "TestCatalogBudgetFitsTheSignatures",
	"catalog.bias":                       "TestCatalogBiasPicksWhatATightBudgetShows",
	"catalog.pinnedTools":                "TestCatalogPinnedToolsAlwaysListInReactiveMode",
	"retention.maxTools":                 "TestRetentionMaxToolsCapsTheReactiveWorkingSet",
	"diagnose.history":                   "TestDiagnoseHistoryCapsRecordedSchemaChanges",
	"diagnose.preflight":                 "TestDiagnosePreflightStopsAMismatchedScript",
	"proto.serveMCP":                     "TestServeMCPUnmountsTheEndpointFromTheEnvironment",
	"daemon.address":                     "TestDaemonAddressIsReachableFromTheEnvironment",
	"daemon.autostart":                   "TestDaemonAutostartIsReachableFromTheEnvironment",
	"http.bodyLimit":                     "TestHTTPBodyLimitsRejectOversizedBodies",
	"http.callBodyLimit":                 "TestHTTPBodyLimitsRejectOversizedBodies",
	"http.controlBodyLimit":              "TestHTTPBodyLimitsRejectOversizedBodies",
	"http.ssePing":                       "TestHTTPEventStreamSettingsChangeTheStream",
	"http.sseRetry":                      "TestHTTPEventStreamSettingsChangeTheStream",
	"http.readHeaderTimeout":             "TestHTTPReadHeaderTimeoutClosesSlowClients",
	"http.shutdownGrace":                 "TestHTTPShutdownGraceBoundsHowLongAStopWaits",
	"http.requestTimeout":                "TestHTTPRequestTimeoutBoundsAClientCall",
	"http.streamBufferMax":               "TestHTTPStreamBufferMaxEndsAnOversizedEventLine",
	"transport.bearerToken":              "TestTransportBearerTokenGatesTheDaemon",
	"transport.allowedOrigins":           "TestTransportAllowedOriginsAreServed",
	"transport.sessionMetaKeys":          "TestTransportSessionMetaKeysNameTheSession",
	"transport.sseKeepAlive":             "TestTransportSSEKeepAliveCommentsOnAQuietStream",
	"transport.stdioDrain":               "TestTransportStdioDrainBoundsServeShutdown",
}

// EffectExemptions maps a setting path to the reason no effect probe
// covers it yet. An empty or missing reason is a violation, and the
// count is pinned by EffectExemptionPin.
var EffectExemptions = map[string]string{
	"logging.maxAge":                 "log pipeline bound or rendering choice: its effect appears only across log volume, rotation, or time, none of which a single harness run produces deterministically",
	"script.imports":                 "static import resolution is covered by deterministic internal/preflight graph tests; the e2e harness has no Deno graph fixture yet",
	"script.importsOnMissing":        "missing-package policies are covered by deterministic internal/preflight resolver tests; the e2e harness has no Deno graph fixture yet",
	"script.importsOnDynamic":        "computed-import policies are covered by deterministic internal/preflight graph tests; the e2e harness has no Deno graph fixture yet",
	"script.importsCrossProject":     "cross-project subset policies are covered by deterministic internal/preflight resolver tests; the e2e harness has no multi-project Deno graph fixture yet",
	"script.importsNetwork":          "registry lookup gating is covered by deterministic internal/preflight resolver tests; the e2e harness has no Deno graph fixture yet",
	"script.checkOnError":            "check failure policy is covered by deterministic internal/preflight stub-runtime tests; the e2e harness has no isolated checker fixture yet",
	"script.checkOnUnsupported":      "unsupported-runtime policy is covered by deterministic internal/preflight stub-runtime tests; the e2e harness has no isolated checker fixture yet",
	"deps.autoInstall":               "a runtime fetch policy: its effect is which --install flag a bun run carries, observable only when a runtime actually fetches a missing package -- a network install the harness does not perform deterministically",
	"paths.config":                   "a discovery search path: its effect appears only when fixture content exists solely on that path, which the harness does not plant per-path",
	"daemon.tsnetUpTimeout":          "tsnet bring-up needs a tailnet the harness does not have",
	"daemon.saveInterval":            "daemon lifecycle, binding, or timing: its effect is a bound, an interval, or a network identity the isolated harness cannot vary deterministically",
	"daemon.port":                    "daemon lifecycle, binding, or timing: its effect is a bound, an interval, or a network identity the isolated harness cannot vary deterministically",
	"daemon.tsnet.hostname":          "daemon lifecycle, binding, or timing: its effect is a bound, an interval, or a network identity the isolated harness cannot vary deterministically",
	"daemon.roots":                   "configured upstream roots only affect an upstream roots/list request; the e2e harness does not yet issue that server request, while daemon hook and mcpclient unit tests cover settings conversion and the response path",
	"output.color":                   "only the interactive browser (tui) renders colour; no non-interactive surface the harness drives can show a with/without difference",
	"proto.elicitationPerMinute":     "daemon proto rate limit: its effect appears only when an upstream server exceeds the per-minute elicitation bound, a flood the e2e harness's fake server does not drive deterministically; the limiter itself is covered by internal/mcpclient and internal/daemon ratelimit tests",
	"proto.samplingPerMinute":        "daemon proto rate limit: its effect appears only when an upstream server exceeds the per-minute sampling bound, a flood the e2e harness's fake server does not drive deterministically; the limiter itself is covered by internal/mcpclient and internal/daemon ratelimit tests",
	"proto.samplingSessionPerMinute": "daemon proto rate limit: its effect appears only when one session exceeds the per-minute sampling bound, a flood the e2e harness's fake server does not drive deterministically; the limiter itself is covered by internal/mcpclient and internal/daemon ratelimit tests",
	"prompt.sample":                  "prompt/sampling behaviour: its effect needs a sampling provider or several candidate actions; no fixture provides one",
	"prompt.catalogBudget":           "prompt/sampling behaviour: its effect needs a sampling provider or several candidate actions; no fixture provides one",
	"prompt.sampleTimeout":           "prompt/sampling behaviour: its effect needs a sampling provider or several candidate actions; no fixture provides one",
	"model.apiKeyEnv":                "model layer is covered by internal/model and daemon unit tests; no settings-driven e2e with/without probe exists yet",
	"model.baseURL":                  "model layer is covered by internal/model and daemon unit tests; no settings-driven e2e with/without probe exists yet",
	"model.budget":                   "model layer is covered by internal/model and daemon unit tests; no settings-driven e2e with/without probe exists yet",
	"model.default":                  "model layer is covered by internal/model and daemon unit tests; no settings-driven e2e with/without probe exists yet",
	"model.profiles":                 "model layer is covered by internal/model and daemon unit tests; no settings-driven e2e with/without probe exists yet",
	"model.provider":                 "model layer is covered by internal/model and daemon unit tests; no settings-driven e2e with/without probe exists yet",
	"model.providers":                "model layer is covered by internal/model and daemon unit tests; no settings-driven e2e with/without probe exists yet",
	"model.timeout":                  "model layer is covered by internal/model and daemon unit tests; no settings-driven e2e with/without probe exists yet",
	"sampling.answerLocally":         "model layer is covered by internal/model and daemon unit tests; no settings-driven e2e with/without probe exists yet",
	"prompt.maxTokens":               "prompt/sampling behaviour: its effect needs a sampling provider or several candidate actions; no fixture provides one",
	"prompt.runTimeout":              "prompt/sampling behaviour: its effect needs a sampling provider or several candidate actions; no fixture provides one",
	"http.streamBufferInit":          "HTTP server bound (body limits, timeouts, buffers): its effect needs oversized bodies, idle connections, or time passage a single run does not produce",
	"http.idleConns":                 "HTTP server bound (body limits, timeouts, buffers): its effect needs oversized bodies, idle connections, or time passage a single run does not produce",
	"http.remoteIdleConns":           "HTTP server bound (body limits, timeouts, buffers): its effect needs oversized bodies, idle connections, or time passage a single run does not produce",
	"http.idleConnTimeout":           "HTTP server bound (body limits, timeouts, buffers): its effect needs oversized bodies, idle connections, or time passage a single run does not produce",
	"autostart.pollInterval":         "client autostart mechanics: the harness itself depends on these to start daemons, so varying them per-test would fight the fixture",
	"search.semantic":                "semantic search needs an embeddings backend fixture the harness does not provide",
	"logstore.followBacklog":         "log store surface: its effect needs stored log volume beyond what one harness run writes",
	"events.reconnect":               "daemon event buffer/reconnect behaviour: needs disconnect/reconnect timing the harness does not drive",
	"daemon.idleExit":                "daemon lifecycle, binding, or timing: its effect is a bound, an interval, or a network identity the isolated harness cannot vary deterministically",
	"retention.stickyWindow":         "retention window: its effect appears across call history and time, not within one run",
	"embeddings.backend":             "embeddings backend: needs a live embeddings service or a wasm model fixture the harness does not provide",
	"embeddings.url":                 "embeddings backend: needs a live embeddings service or a wasm model fixture the harness does not provide",
	"embeddings.apiKey":              "needs a live embeddings service that accepts the key; no fixture serves one",
	"embeddings.model":               "embeddings backend: needs a live embeddings service or a wasm model fixture the harness does not provide",
	"embeddings.wasmPath":            "embeddings backend: needs a live embeddings service or a wasm model fixture the harness does not provide",
	"daemon.socketProbeTimeout":      "daemon lifecycle, binding, or timing: its effect is a bound, an interval, or a network identity the isolated harness cannot vary deterministically",
	"daemon.takeoverTimeout":         "daemon lifecycle, binding, or timing: its effect is a bound, an interval, or a network identity the isolated harness cannot vary deterministically",
	"daemon.inlineStartPoll":         "daemon lifecycle, binding, or timing: its effect is a bound, an interval, or a network identity the isolated harness cannot vary deterministically",
	"session.releaseTimeout":         "session release timing bound: observable only across time passage",
	"plugin.bin":                     "read by the opencode plugin host (JS side); the Go e2e harness does not drive the plugin host",
	"plugin.binArgs":                 "read by the opencode plugin host (JS side); the Go e2e harness does not drive the plugin host",
	"plugin.backend":                 "read by the opencode plugin host (JS side); the Go e2e harness does not drive the plugin host",
	"plugin.discoveryRetry":          "read by the opencode plugin host (JS side); the Go e2e harness does not drive the plugin host",
	"plugin.toolTiming":              "read by the opencode plugin host (JS side); the Go e2e harness does not drive the plugin host",
	"plugin.env":                     "read by the opencode plugin host (JS side); the Go e2e harness does not drive the plugin host",
	"plugin.instructions":            "read by the opencode plugin host (JS side); the Go e2e harness does not drive the plugin host",
	"plugin.tools":                   "read by the opencode plugin host (JS side); the Go e2e harness does not drive the plugin host",
	"plugin.remember":                "read by the opencode plugin host (JS side); the Go e2e harness does not drive the plugin host",
	"plugin.annotate":                "read by the opencode plugin host (JS side); the Go e2e harness does not drive the plugin host",
	"plugin.headless":                "read by the opencode plugin host (JS side); the Go e2e harness does not drive the plugin host",
	"plugin.daemonTools":             "read by the opencode plugin host (JS side); the Go e2e harness does not drive the plugin host",
	"plugin.disabledTools":           "read by the opencode plugin host (JS side); the Go e2e harness does not drive the plugin host",
	"protoTasks.cancelGrace":         "task-store cancellation timing: covered by internal/tasks store tests, not by a settings with/without probe",
}

// EffectExemptionPin is the ratchet: the exemption count at the time
// the guard landed. It may only ever be lowered, and only in the PR
// that removes an exemption by adding the probe that replaces it.
const EffectExemptionPin = 66 // Shrunk 2026-11-18 (#8473 chain 53): daemon.leaseTTL gained a silent-caller probe. Earlier shrunk 2026-11-18 (#8473 chain 52): elicit.disambiguate and elicit.disambiguateDefault gained probes. Earlier shrunk 2026-11-18 (#8473 chain 51): elicit.ttl and elicit.maxAge gained probes, once upstream questions took their deadline from the broker. Earlier shrunk 2026-11-18 (#8473 chain 50): elicit.pendingLimit gained a probe, once `mcpx elicit list` honoured it. Earlier shrunk 2026-11-18 (#8473 chain 49): elicit.askTimeout gained a held-confirmation probe. Earlier shrunk 2026-11-18 (#8473 chain 48): artifacts.idleTtl and artifacts.maxAge gained touch-on-read probes. Earlier shrunk 2026-11-18 (#8473 chain 47): artifacts.ttl, artifacts.gcInterval, pool.min and autostart.idleExit gained probes. Earlier shrunk 2026-11-18 (#8473 chain 46): daemon.warm, pool.idleTimeout, daemon.reapInterval and daemon.endpoint gained probes. Earlier shrunk 2026-11-18 (#8473 chain 45): upstream.probeTimeout gained a probe, and upstream.eraCache and upstream.protocol were mapped to the era test, whose subtests run each with and without. Earlier shrunk 2026-11-18 (#8473 chain 44): prompt.autonomy, hooks.autonomy and repair.autonomy gained probes. Earlier shrunk 2026-11-18 (#8473 chain 43): policy.write, elicit.confirmDestructive and autonomy.max gained probes. Earlier shrunk 2026-11-18 (#8473 chain 42): daemon.inline, autostart.bin, autostart.args, autostart.logTail and daemon.watchConfig gained probes. Earlier shrunk 2026-11-18 (#8473 chain 41): daemon.inlineStartTimeout and daemon.probeTimeout gained probes. Earlier shrunk 2026-11-18 (#8473 chain 40): daemon.warmTimeout, autostart.connectTimeout and autostart.pingTimeout gained probes. Earlier shrunk 2026-11-18 (#8473 chain 39): pool.callTimeout, pool.startTimeout and daemon.refreshTimeout gained probes. Earlier shrunk 2026-11-18 (#8473 chain 38): doctor.timeout and registry.timeout gained probes that set a bound no request can meet. Earlier shrunk 2026-11-18 (#8473 chain 37): completion.maxValues gained a probe once the fake server could answer completions. Earlier shrunk 2026-11-18 (#8473 chain 36): catalog.instructions gained a probe once the fake server could serve instructions. Earlier shrunk 2026-11-18 (#8473 chain 35): the five plumbing source and launcher switches gained probes. Earlier shrunk 2026-11-18 (#8473 chain 34): artifacts.chunkBytes, script.launcher and paths.placeholders gained probes. Earlier shrunk 2026-11-18 (#8473 chain 33): artifacts.interceptImages and plumbing.indexOnQuery gained probes. Earlier shrunk 2026-11-18 (#8473 chain 32): plumbing.validatePaths and script.language gained probes. Earlier shrunk 2026-11-18 (#8473 chain 31): logging.trace and exec.where gained probes. Earlier shrunk 2026-11-18 (#8473 chain 30): paths.adapters and paths.apis gained declaration-file probes. Earlier shrunk 2026-11-18 (#8473 chain 29): script.runtime, runtimeOrder, permissions, profiles and runtimes gained node-backed probes. Earlier shrunk 2026-11-18 (#8473 chain 28): script.before, onSuccess, onError and suffix gained a run-phase probe. Earlier shrunk 2026-11-18 (#8473 chain 27): hooks, hooks.timeout, daemon.ui and daemon.uiDir gained probes. Earlier shrunk 2026-11-18 (#8473 chain 26): logging.level, format, source, dir, include, maxLines, maxBytes and keep gained durable-log probes. Earlier shrunk 2026-11-18 (#8473 chain 25): artifacts.enabled, maxBytes, quota, delivery, inlineMaxBytes and dir gained script-artifact probes. Earlier shrunk 2026-11-18 (#8473 chain 24): preset and serve.mode gained probes. Earlier shrunk 2026-11-18 (#8473 chain 23): presets gained a --preset probe. Earlier shrunk 2026-11-18 (#8473 chain 22): catalog.budget and catalog.bias gained `mcpx catalog` probes. Earlier shrunk 2026-11-18 (#8473 chain 21): catalog.pinnedTools and retention.maxTools gained reactive-session probes. Earlier shrunk 2026-11-18 (#8473 chain 20): diagnose.history and diagnose.preflight gained schema-churn probes. Earlier shrunk 2026-11-18 (#8473 chain 19): events.history gained a trimmed-replay probe. Earlier shrunk 2026-11-18 (#8473 chain 18): protoTasks.maxTTL, maxPerRequestor and perMinute gained task-store probes. Earlier shrunk 2026-11-18 (#8473 chain 17): tasks.ttl and tasks.resultWait gained /v1 task probes. Earlier shrunk 2026-11-18 (#8473 chain 16): recipes.minScore, matchMargin and limit gained recipe probes. Earlier shrunk 2026-11-18 (#8473 chain 15): logstore.queryLimit gained a /v1/log probe. Earlier shrunk 2026-11-18 (#8473 chain 14): completion.perMinute and completion.serverPerMinute gained 429 probes. Earlier shrunk 2026-11-18 (#8473 chain 13): mcp.pageSize and mcp.passthrough gained tools/list probes. Earlier shrunk 2026-11-18 (#8473 chain 12): exec.timeout (now read by local runs), exec.maxOutputBytes and exec.output gained probes. Earlier shrunk 2026-11-18 (#8473 chain 11): spec.first and spec.lenient mapped to their existing revision-policy probes, spec.precedence gained one. Earlier shrunk 2026-11-18 (#8473 chain 10): protoMessages.taskAfter and protoTasks.pollInterval gained task probes. Earlier shrunk 2026-11-18 (#8473 chain 9): protoMessages.listMaxAge and readMaxAge gained cache-hint probes. Earlier shrunk 2026-11-18 (#8473 chain 8): proto.native gained an asked-client probe. Earlier shrunk 2026-11-18 (#8473 chain 7): proto.mcpPath and proto.serveMCP gained mount probes. Earlier shrunk 2026-11-18 (#8473 chain 6): proto.sessionIdle gained a reaped-session probe. Earlier shrunk 2026-11-18 (#8473 chain 5): proto.stateTTL gained a late-resume probe. Earlier shrunk 2026-11-18 (#8473 chain 4): proto.askTimeout gained a held-request probe. Earlier shrunk 2026-11-18 (#8473 chain 3): proto.askTTL gained a /v1/ask deadline probe. Earlier shrunk 2026-11-18 (#8473 chain 2): proto.askRounds gained a two-round MCP e2e probe. Earlier shrunk 2026-11-18 (#8468 chain 2): daemon.autostart gained its probe. Earlier shrunk 2026-11-17: (#8467 settings core-paths burn-down) paths.plugins gained its probe. Earlier shrunk 2026-11-17: (#8469) 14 http.* and transport.* settings gained their probes. Earlier shrunk 2026-11-17: (#8470 burn-down) stats.top gained its probe. Earlier shrunk 2026-11-17 (#8467 settings core-paths burn-down): paths.cache gained its probe. Earlier shrunk 2026-11-17 (#8467 settings core-paths burn-down): paths.scripts gained its probe. Earlier shrunk 2026-11-17 (#8468 chain 1): daemon.address gained its probe. Earlier shrunk 2026-11-17 (#8473 chain 1): proto.askPoll gained its probe. Deliberate +1 (2026-11-14, #1917 remainder shepherd of PR #7818) for script.language: covered by deterministic internal/runner tests (TestEffectiveScriptLanguage, TestExplicitPythonLanguageRunsInlineSnippet); the e2e harness script fixtures are TypeScript-only. Earlier deliberate +3 (2026-11-09, #2716) for proto.elicitationPerMinute, proto.samplingPerMinute and proto.samplingSessionPerMinute: daemon rate limits whose effect needs an over-limit sampling/elicitation flood no e2e fixture drives deterministically; the limiter is covered by internal/mcpclient and internal/daemon ratelimit tests. Earlier deliberate raises: +1 for daemon.roots and +8 for model.*: model settings are covered by internal/model and daemon unit tests; settings-driven e2e probes are not yet present. Deliberate +1 (2026-11-03, #1925 slice 3) for sampling.answerLocally: same model-layer coverage, daemon unit tests in internal/daemon/sampling_local_test.go. Deliberate +1 (2026-11-03, #5242) for plugin.disabledTools: read by the opencode plugin host (JS side); the Go e2e harness does not drive the plugin host, the same basis as every other plugin.* exemption. Deliberate +5 (2026-11-04, #1915 stage 2): import graph, missing, dynamic, cross-project and network policies are covered by deterministic internal/preflight tests, but the e2e harness has no isolated Deno graph fixture yet. Deliberate +2 (2026-11-05, #1911): checker error and unsupported-runtime policies are covered by deterministic internal/preflight stub-runtime tests; the e2e harness has no isolated Deno checker fixture.

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
