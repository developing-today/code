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
	"output.color":                       "TestOutputColorIsReachableFromTheEnvironment",
	"plumbing.strictUnknownKeys":         "TestStrictUnknownKeysRefusesATypo",
	"plugin.annotate":                    "TestPluginAnnotateChangesToolResultsFromTheEnvironment",
	"plugin.backend":                     "TestPluginBackendChoosesDaemonOrBinaryFromTheEnvironment",
	"plugin.bin":                         "TestPluginBinInvokesTheConfiguredExecutableFromTheEnvironment",
	"plugin.binArgs":                     "TestPluginBinArgsPrependsTheConfiguredArgumentsFromTheEnvironment",
	"plugin.daemonTools":                 "TestPluginDaemonToolsAreOfferedFromTheEnvironment",
	"plugin.disabledTools":               "TestPluginDisabledToolsHidesToolsFromTheEnvironment",
	"plugin.discoveryRetry":              "TestPluginDiscoveryRetryDelaysRediscoveryFromTheEnvironment",
	"plugin.env":                         "TestPluginEnvLevelChangesShellEnvFromTheEnvironment",
	"plugin.headless":                    "TestPluginHeadlessSuppressesToastsFromTheEnvironment",
	"plugin.instructions":                "TestPluginInstructionsAppendsTheContextParagraphFromTheEnvironment",
	"plugin.tools":                       "TestPluginToolsRegistersTheToolSetFromTheEnvironment",
	"plugin.toolTiming":                  "TestPluginToolTimingRecordsToolCallsFromTheEnvironment",
	"plugin.remember":                    "TestPluginRememberScopePersistsTheDaemonChoiceFromTheEnvironment",
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
	"deps.autoInstall":                   "TestDepsAutoInstallSelectsTheBunInstallPolicyFromTheEnvironment",
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
	"protoTasks.cancelGrace":             "TestProtoTasksCancelGraceExpiresACancelledTaskFromTheEnvironment",
	"events.history":                     "TestEventsHistoryBoundsTheReplay",
	"events.reconnect":                   "TestEventsReconnectWaitsForTheConfiguredDelay",
	"script.checkOnUnsupported":          "TestScriptCheckOnUnsupportedDecidesWhatANonDenoCheckDoes",
	"script.checkOnError":                "TestScriptCheckOnErrorDecidesOnACheckError",
	"daemon.takeoverTimeout":             "TestDaemonTakeoverTimeoutBoundsTheHandoverFromTheEnvironment",
	"daemon.socketProbeTimeout":          "TestDaemonSocketProbeTimeoutDecidesWhetherALiveSocketIsStale",
	"logging.maxAge":                     "TestLoggingMaxAgeRotatesAnAgedFile",
	"logstore.followBacklog":             "TestFollowBacklogCapsWhatOneFollowPollEmits",
	"paths.config":                       "TestPathsConfigReadsAConfigOnlyThatPathHolds",
	"script.importsOnDynamic":            "TestScriptImportsOnDynamicDecidesAComputedImport",
	"script.imports":                     "TestScriptImportsOffSkipsTheImportPipeline",
	"script.importsCrossProject":         "TestScriptImportsCrossProjectDecidesAForeignProjectEdge",
	"script.importsOnMissing":            "TestScriptImportsOnMissingDecidesOnAMissingPackage",
	"script.importsNetwork":              "TestScriptImportsNetworkDecidesWhetherToPinAnUndeclaredPackage",
	"prompt.runTimeout":                  "TestPromptRunTimeoutBoundsARecipeRun",
	"daemon.roots":                       "TestDaemonRootsAnswersAnUpstreamRootsRequest",
	"prompt.sampleTimeout":               "TestPromptSampleTimeoutBoundsASamplingQuestion",
	"prompt.sample":                      "TestPromptSampleAskLetsAnUnmatchedPromptSeekAModel",
	"prompt.catalogBudget":               "TestPromptCatalogBudgetBoundsTheCatalogSentToTheModel",
	"search.semantic":                    "TestSearchSemanticRanksWithTheConfiguredEmbedder",
	"embeddings.url":                     "TestEmbeddingsURLSendsSemanticSearchToTheEndpoint",
	"embeddings.apiKey":                  "TestEmbeddingsAPIKeyIsSentAsABearerToken",
	"embeddings.model":                   "TestEmbeddingsModelNamesTheRequestedModel",
	"embeddings.backend":                 "TestEmbeddingsBackendLocalSkipsTheRemoteEndpoint",
	"proto.samplingPerMinute":            "TestSamplingPerMinuteRefusesAServersSecondSampling",
	"proto.samplingSessionPerMinute":     "TestSamplingSessionPerMinuteRefusesASessionsSecondSampling",
	"proto.elicitationPerMinute":         "TestElicitationPerMinuteRefusesAServersSecondQuestion",
	"sampling.answerLocally":             "TestSamplingAnswerLocallyAnswersAnUpstreamFromTheEnvironment",
	"model.provider":                     "TestModelProviderSelectsTheBackendFromTheEnvironment",
	"model.baseURL":                      "TestModelBaseURLPointsTheProviderAtAnEndpointFromTheEnvironment",
	"model.default":                      "TestModelDefaultNamesTheModelFromTheEnvironment",
	"model.apiKeyEnv":                    "TestModelAPIKeyEnvNamesTheKeyVariableFromTheEnvironment",
	"model.budget":                       "TestModelBudgetCapsAHooksTokensFromTheEnvironment",
	"prompt.maxTokens":                   "TestPromptMaxTokensSetsAHooksTokenCapFromTheEnvironment",
	"model.providers":                    "TestModelProvidersRoutesAProviderToItsEndpointFromTheEnvironment",
	"model.profiles":                     "TestModelProfilesResolvesANamedChainFromTheEnvironment",
	"model.timeout":                      "TestModelTimeoutBoundsAGenerationFromTheEnvironment",
	"session.releaseTimeout":             "TestSessionReleaseTimeoutBoundsTheEndOfRunReleaseFromTheEnvironment",
	"daemon.idleExit":                    "TestDaemonIdleExitStopsAnIdleDaemonFromTheEnvironment",
	"daemon.port":                        "TestDaemonPortBindsTheChosenPortFromTheEnvironment",
	"daemon.saveInterval":                "TestDaemonSaveIntervalWritesTheLeaseTableFromTheEnvironment",
	"autostart.pollInterval":             "TestAutostartPollIntervalPacesTheReadinessPollFromTheEnvironment",
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
	"adapters.allowBare":                 "TestAdaptersAllowBareGatesABareDeclaration",
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
	"retention.stickyWindow":             "TestRetentionStickyWindowExpiresReactiveTools",
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
	"http.idleConns":                     "TestHTTPIdleConnectionPoolSettingsChangeReuse",
	"http.remoteIdleConns":               "TestHTTPIdleConnectionPoolSettingsChangeReuse",
	"http.idleConnTimeout":               "TestHTTPIdleConnTimeoutExpiresAnIdleClientConnection",
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
	"daemon.tsnetUpTimeout": "tsnet bring-up needs a tailnet the harness does not have",
	"daemon.tsnet.hostname": "daemon lifecycle, binding, or timing: its effect is a bound, an interval, or a network identity the isolated harness cannot vary deterministically",
	"http.streamBufferInit": "Scanner allocation-capacity hint only; changing it does not change emitted events or line limits, and allocation size is not a stable e2e behavior oracle",
	"embeddings.wasmPath":   "embeddings backend: needs a compiled wasm model fixture the harness does not provide; the Go wasm runtime (wazero) can compile a module but no embed-exporting model ships with the repo, so the setting's effect (wasm embedder availability) is not observable end to end",
}

// EffectExemptionPin is the ratchet: the exemption count at the time
// the guard landed. It may only ever be lowered, and only in the PR
// that removes an exemption by adding the probe that replaces it.
const EffectExemptionPin = 4 // Shrunk 2026-11-24 (#13595): daemon.inlineStartPoll now has a named e2e probe; deterministic retry-count sensitivity is covered by the CLI harness.  Shrunk 2026-11-23 (#8472): plugin.disabledTools now has an e2e probe that drives the plugin host under bun. Shrunk 2026-11-23 (#8473): protoTasks.cancelGrace now has an e2e probe on the tasks extension path. Shrunk 2026-11-23 (#8472): plugin.instructions now has an e2e probe that drives the plugin host under bun. Shrunk 2026-11-23 (#8472): plugin.headless now has an e2e probe that drives the plugin host under bun. Shrunk 2026-11-23 (#8472): plugin.remember now has an e2e probe that drives the plugin host under bun. Shrunk 2026-11-23 (#8472): plugin.toolTiming now has an e2e probe that drives the plugin host under bun. Shrunk 2026-11-23 (#8472): plugin.discoveryRetry now has an e2e probe that drives the plugin host under bun. Shrunk 2026-11-23 (#8472): plugin.tools now has an e2e probe that drives the plugin host under bun. Shrunk 2026-11-23 (#8472): plugin.daemonTools now has an e2e probe that drives the plugin host under bun. Shrunk 2026-11-23 (#8472): plugin.binArgs now has an e2e probe that drives the plugin host under bun. Shrunk 2026-11-23 (#8472): plugin.env now has an e2e probe that drives the plugin host under bun. Shrunk 2026-11-23 (#8472): plugin.bin now has an e2e probe that drives the plugin host under bun. Shrunk 2026-11-23 (#8472): plugin.backend now has an e2e probe that drives the plugin host under bun. Shrunk 2026-11-23 (#8472): plugin.annotate now has an e2e probe that drives the plugin host under bun. Shrunk 2026-11-22 (#8474): script.importsOnMissing + script.importsNetwork now have e2e probes. Shrunk 2026-11-22 (#8474): script.importsCrossProject + script.checkOnError. Shrunk 2026-11-21 (#8469): idle connection limits and timeout now assert actual connection reuse/expiry through configured clients.  Shrunk 2026-11-21 (#8470): retention.stickyWindow now expires requested tools after its configured window. Earlier shrunk 2026-11-21 (#8470): events.reconnect now asserts retry delay after a dropped connection. Earlier shrunk 2026-11-21 (#8470): deps.autoInstall now asserts Bun's selected install policy. Earlier shrunk 2026-11-19 (#8467): output.color now has a PTY-backed interactive TUI probe. Earlier shrunk 2026-11-18 (#8473 chain 73): script.checkOnUnsupported is now probed under bun. Earlier shrunk 2026-11-18 (#8473 chain 72): daemon.takeoverTimeout gained a probe with a successor whose takeover dial cannot finish. Earlier shrunk 2026-11-18 (#8473 chain 71): daemon.socketProbeTimeout gained a probe with a second daemon started beside a live one. Earlier shrunk 2026-11-18 (#8473 chain 70): logging.maxAge gained a probe with a log file whose first record is two hours old. Earlier shrunk 2026-11-18 (#8473 chain 69): logstore.followBacklog gained a follow probe with a burst between two polls. Earlier shrunk 2026-11-18 (#8473 chain 68): paths.config gained a probe with a config planted only on the named path. Earlier shrunk 2026-11-18 (#8473 chain 67): script.importsOnDynamic and script.imports gained probes with a computed import() in exec source text, which needs no Deno graph. Earlier shrunk 2026-11-18 (#8473 chain 66): prompt.runTimeout gained a probe with a recipe that sleeps past a 1s bound. Earlier shrunk 2026-11-18 (#8473 chain 65): daemon.roots gained a probe through the ask fixture's new need_roots tool. Earlier shrunk 2026-11-18 (#8473 chain 64): prompt.sampleTimeout was orphaned since #2004; it is now the sampling question fallback when model.timeout is unset, and gained a probe. Earlier shrunk 2026-11-18 (#8473 chain 63): prompt.sample and prompt.catalogBudget gained probes on an unmatched prompt, the latter against the fake model endpoint. Earlier shrunk 2026-11-18 (#8473 chain 62): search.semantic and the remote embeddings settings gained probes against a fake OpenAI-compatible embeddings endpoint. Earlier shrunk 2026-11-18 (#8473 chain 61): proto.samplingPerMinute, proto.samplingSessionPerMinute and proto.elicitationPerMinute gained rate-limit probes driven by the ask fixture. Earlier shrunk 2026-11-18 (#8473 chain 60): sampling.answerLocally gained an upstream-sampling probe against the fake model endpoint. Earlier shrunk 2026-11-18 (#8473 chain 59): model.provider, baseURL, default, apiKeyEnv, budget, providers, profiles, timeout and prompt.maxTokens gained prompt-hook probes against a fake model endpoint. Earlier shrunk 2026-11-18 (#8473 chain 58): session.releaseTimeout gained an end-of-run release probe. Earlier shrunk 2026-11-18 (#8473 chain 57): daemon.idleExit gained an idle-daemon probe. Earlier shrunk 2026-11-18 (#8473 chain 56): daemon.port gained a bound-port probe. Earlier shrunk 2026-11-18 (#8473 chain 55): daemon.saveInterval gained a lease-table save probe. Earlier shrunk 2026-11-18 (#8473 chain 54): autostart.pollInterval gained a readiness-poll probe. Earlier shrunk 2026-11-18 (#8473 chain 53): daemon.leaseTTL gained a silent-caller probe. Earlier shrunk 2026-11-18 (#8473 chain 52): elicit.disambiguate and elicit.disambiguateDefault gained probes. Earlier shrunk 2026-11-18 (#8473 chain 51): elicit.ttl and elicit.maxAge gained probes, once upstream questions took their deadline from the broker. Earlier shrunk 2026-11-18 (#8473 chain 50): elicit.pendingLimit gained a probe, once `mcpx elicit list` honoured it. Earlier shrunk 2026-11-18 (#8473 chain 49): elicit.askTimeout gained a held-confirmation probe. Earlier shrunk 2026-11-18 (#8473 chain 48): artifacts.idleTtl and artifacts.maxAge gained touch-on-read probes. Earlier shrunk 2026-11-18 (#8473 chain 47): artifacts.ttl, artifacts.gcInterval, pool.min and autostart.idleExit gained probes. Earlier shrunk 2026-11-18 (#8473 chain 46): daemon.warm, pool.idleTimeout, daemon.reapInterval and daemon.endpoint gained probes. Earlier shrunk 2026-11-18 (#8473 chain 45): upstream.probeTimeout gained a probe, and upstream.eraCache and upstream.protocol were mapped to the era test, whose subtests run each with and without. Earlier shrunk 2026-11-18 (#8473 chain 44): prompt.autonomy, hooks.autonomy and repair.autonomy gained probes. Earlier shrunk 2026-11-18 (#8473 chain 43): policy.write, elicit.confirmDestructive and autonomy.max gained probes. Earlier shrunk 2026-11-18 (#8473 chain 42): daemon.inline, autostart.bin, autostart.args, autostart.logTail and daemon.watchConfig gained probes. Earlier shrunk 2026-11-18 (#8473 chain 41): daemon.inlineStartTimeout and daemon.probeTimeout gained probes. Earlier shrunk 2026-11-18 (#8473 chain 40): daemon.warmTimeout, autostart.connectTimeout and autostart.pingTimeout gained probes. Earlier shrunk 2026-11-18 (#8473 chain 39): pool.callTimeout, pool.startTimeout and daemon.refreshTimeout gained probes. Earlier shrunk 2026-11-18 (#8473 chain 38): doctor.timeout and registry.timeout gained probes that set a bound no request can meet. Earlier shrunk 2026-11-18 (#8473 chain 37): completion.maxValues gained a probe once the fake server could answer completions. Earlier shrunk 2026-11-18 (#8473 chain 36): catalog.instructions gained a probe once the fake server could serve instructions. Earlier shrunk 2026-11-18 (#8473 chain 35): the five plumbing source and launcher switches gained probes. Earlier shrunk 2026-11-18 (#8473 chain 34): artifacts.chunkBytes, script.launcher and paths.placeholders gained probes. Earlier shrunk 2026-11-18 (#8473 chain 33): artifacts.interceptImages and plumbing.indexOnQuery gained probes. Earlier shrunk 2026-11-18 (#8473 chain 32): plumbing.validatePaths and script.language gained probes. Earlier shrunk 2026-11-18 (#8473 chain 31): logging.trace and exec.where gained probes. Earlier shrunk 2026-11-18 (#8473 chain 30): paths.adapters and paths.apis gained declaration-file probes. Earlier shrunk 2026-11-18 (#8473 chain 29): script.runtime, runtimeOrder, permissions, profiles and runtimes gained node-backed probes. Earlier shrunk 2026-11-18 (#8473 chain 28): script.before, onSuccess, onError and suffix gained a run-phase probe. Earlier shrunk 2026-11-18 (#8473 chain 27): hooks, hooks.timeout, daemon.ui and daemon.uiDir gained probes. Earlier shrunk 2026-11-18 (#8473 chain 26): logging.level, format, source, dir, include, maxLines, maxBytes and keep gained durable-log probes. Earlier shrunk 2026-11-18 (#8473 chain 25): artifacts.enabled, maxBytes, quota, delivery, inlineMaxBytes and dir gained script-artifact probes. Earlier shrunk 2026-11-18 (#8473 chain 24): preset and serve.mode gained probes. Earlier shrunk 2026-11-18 (#8473 chain 23): presets gained a --preset probe. Earlier shrunk 2026-11-18 (#8473 chain 22): catalog.budget and catalog.bias gained `mcpx catalog` probes. Earlier shrunk 2026-11-18 (#8473 chain 21): catalog.pinnedTools and retention.maxTools gained reactive-session probes. Earlier shrunk 2026-11-18 (#8473 chain 20): diagnose.history and diagnose.preflight gained schema-churn probes. Earlier shrunk 2026-11-18 (#8473 chain 19): events.history gained a trimmed-replay probe. Earlier shrunk 2026-11-18 (#8473 chain 18): protoTasks.maxTTL, maxPerRequestor and perMinute gained task-store probes. Earlier shrunk 2026-11-18 (#8473 chain 17): tasks.ttl and tasks.resultWait gained /v1 task probes. Earlier shrunk 2026-11-18 (#8473 chain 16): recipes.minScore, matchMargin and limit gained recipe probes. Earlier shrunk 2026-11-18 (#8473 chain 15): logstore.queryLimit gained a /v1/log probe. Earlier shrunk 2026-11-18 (#8473 chain 14): completion.perMinute and completion.serverPerMinute gained 429 probes. Earlier shrunk 2026-11-18 (#8473 chain 13): mcp.pageSize and mcp.passthrough gained tools/list probes. Earlier shrunk 2026-11-18 (#8473 chain 12): exec.timeout (now read by local runs), exec.maxOutputBytes and exec.output gained probes. Earlier shrunk 2026-11-18 (#8473 chain 11): spec.first and spec.lenient mapped to their existing revision-policy probes, spec.precedence gained one. Earlier shrunk 2026-11-18 (#8473 chain 10): protoMessages.taskAfter and protoTasks.pollInterval gained task probes. Earlier shrunk 2026-11-18 (#8473 chain 9): protoMessages.listMaxAge and readMaxAge gained cache-hint probes. Earlier shrunk 2026-11-18 (#8473 chain 8): proto.native gained an asked-client probe. Earlier shrunk 2026-11-18 (#8473 chain 7): proto.mcpPath and proto.serveMCP gained mount probes. Earlier shrunk 2026-11-18 (#8473 chain 6): proto.sessionIdle gained a reaped-session probe. Earlier shrunk 2026-11-18 (#8473 chain 5): proto.stateTTL gained a late-resume probe. Earlier shrunk 2026-11-18 (#8473 chain 4): proto.askTimeout gained a held-request probe. Earlier shrunk 2026-11-18 (#8473 chain 3): proto.askTTL gained a /v1/ask deadline probe. Earlier shrunk 2026-11-18 (#8473 chain 2): proto.askRounds gained a two-round MCP e2e probe. Earlier shrunk 2026-11-18 (#8468 chain 2): daemon.autostart gained its probe. Earlier shrunk 2026-11-17: (#8467 settings core-paths burn-down) paths.plugins gained its probe. Earlier shrunk 2026-11-17: (#8469) 14 http.* and transport.* settings gained their probes. Earlier shrunk 2026-11-17: (#8470 burn-down) stats.top gained its probe. Earlier shrunk 2026-11-17 (#8467 settings core-paths burn-down): paths.cache gained its probe. Earlier shrunk 2026-11-17 (#8467 settings core-paths burn-down): paths.scripts gained its probe. Earlier shrunk 2026-11-17 (#8468 chain 1): daemon.address gained its probe. Earlier shrunk 2026-11-17 (#8473 chain 1): proto.askPoll gained its probe. Deliberate +1 (2026-11-14, #1917 remainder shepherd of PR #7818) for script.language: covered by deterministic internal/runner tests (TestEffectiveScriptLanguage, TestExplicitPythonLanguageRunsInlineSnippet); the e2e harness script fixtures are TypeScript-only. Earlier deliberate +3 (2026-11-09, #2716) for proto.elicitationPerMinute, proto.samplingPerMinute and proto.samplingSessionPerMinute: daemon rate limits whose effect needs an over-limit sampling/elicitation flood no e2e fixture drives deterministically; the limiter is covered by internal/mcpclient and internal/daemon ratelimit tests. Earlier deliberate raises: +1 for daemon.roots and +8 for model.*: model settings are covered by internal/model and daemon unit tests; settings-driven e2e probes are not yet present. Deliberate +1 (2026-11-03, #1925 slice 3) for sampling.answerLocally: same model-layer coverage, daemon unit tests in internal/daemon/sampling_local_test.go. Deliberate +1 (2026-11-03, #5242) for plugin.disabledTools: read by the opencode plugin host (JS side); the Go e2e harness does not drive the plugin host, the same basis as every other plugin.* exemption. Deliberate +5 (2026-11-04, #1915 stage 2): import graph, missing, dynamic, cross-project and network policies are covered by deterministic internal/preflight tests, but the e2e harness has no isolated Deno graph fixture yet. Deliberate +2 (2026-11-05, #1911): checker error and unsupported-runtime policies are covered by deterministic internal/preflight stub-runtime tests; the e2e harness has no isolated Deno checker fixture.

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
