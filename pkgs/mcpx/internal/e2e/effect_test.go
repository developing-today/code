package e2e_test

import (
	"bufio"
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"net"
	"net/http"
	"net/http/httptest"
	"net/url"
	"os"
	"os/exec"
	"path/filepath"
	"sort"
	"strconv"
	"strings"
	"sync"
	"testing"
	"time"

	"github.com/developing-today/code/pkgs/mcpx/internal/cli"
	"github.com/developing-today/code/pkgs/mcpx/internal/daemon"
	"github.com/developing-today/code/pkgs/mcpx/internal/events"
	"github.com/developing-today/code/pkgs/mcpx/internal/settings"
	"github.com/developing-today/code/pkgs/mcpx/internal/testsupport"
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
	"pool.max":                           "TestPoolDefaultsAreReachableFromTheEnvironment",
	"pool.sharing":                       "TestPoolDefaultsAreReachableFromTheEnvironment",
	"pool.scope":                         "TestPoolDefaultsAreReachableFromTheEnvironment",
	"output.json":                        "TestOutputJSONIsReachableFromTheEnvironment",
	"output.color":                       "TestOutputColorIsReachableFromTheEnvironment",
	"script.prefix":                      "TestScriptPrefixIsReachableFromTheEnvironment",
	"script.cwd":                         "TestScriptCwdIsReachableFromTheEnvironment",
	"script.interpreter":                 "TestScriptInterpreterIsReachableFromTheEnvironment",
	"script.env":                         "TestScriptEnvIsReachableFromTheEnvironment",
	"script.devshell":                    "TestScriptDevshellIsReachableFromTheEnvironment",
	"script.captureConsole":              "TestScriptCaptureConsoleOffDropsConsoleOutput",
	"script.check":                       "TestScriptTypecheckOnRefusesATypeError",
	"script.checkOnError":                "TestScriptCheckOnErrorDecidesOnACheckError",
	"script.importsCrossProject":         "TestScriptImportsCrossProjectDecidesAForeignProjectEdge",
	"script.importsOnMissing":            "TestScriptImportsOnMissingDecidesOnAMissingPackage",
	"script.importsNetwork":              "TestScriptImportsNetworkDecidesWhetherToPinAnUndeclaredPackage",
	"logging.file":                       "TestLoggingFileOffWritesNoDurableLog",
	"plumbing.strictUnknownKeys":         "TestStrictUnknownKeysRefusesATypo",
	"plugin.annotate":                    "TestPluginAnnotateChangesToolResultsFromTheEnvironment",
	"plugin.backend":                     "TestPluginBackendChoosesDaemonOrBinaryFromTheEnvironment",
	"plugin.bin":                         "TestPluginBinInvokesTheConfiguredExecutableFromTheEnvironment",
	"plugin.binArgs":                     "TestPluginBinArgsPrependsTheConfiguredArgumentsFromTheEnvironment",
	"plugin.daemonTools":                 "TestPluginDaemonToolsAreOfferedFromTheEnvironment",
	"plugin.disabledTools":               "TestPluginDisabledToolsHidesToolsFromTheEnvironment",
	"plugin.discoveryRetry":              "TestPluginDiscoveryRetryDelaysRediscoveryFromTheEnvironment",
	"plugin.env":                         "TestPluginEnvLevelChangesShellEnvFromTheEnvironment",
	"plugin.tools":                       "TestPluginToolsRegistersTheToolSetFromTheEnvironment",
	"plugin.toolTiming":                  "TestPluginToolTimingRecordsToolCallsFromTheEnvironment",
	"plugin.remember":                    "TestPluginRememberScopePersistsTheDaemonChoiceFromTheEnvironment",
	"plugin.headless":                    "TestPluginHeadlessSuppressesToastsFromTheEnvironment",
	"plugin.instructions":                "TestPluginInstructionsAppendsTheContextParagraphFromTheEnvironment",
	"paths.state":                        "TestStateDirectoryIsReachableAsASetting",
	"paths.configFile":                   "TestPathsConfigFileChangesWhichConfigLoads",
	"paths.scripts":                      "TestPathsScriptsFindsRecipesThatLiveOnlyThere",
	"paths.cache":                        "TestPathsCacheChangesTheDoctorDirectory",
	"paths.plugins":                      "TestPathsPluginsDiscoversAPluginThatLivesOnlyThere",
	"search.limit":                       "TestSearchLimitCapsTheResults",
	"registry.pageSize":                  "TestRegistryPageSizeChangesWhatTheCLIAsksFor",
	"registry.limit":                     "TestRegistryLimitReachesTheCLI",
	"registry.maxPages":                  "TestTheRegistryPageCapSaysItStoppedEarly",
	"registry.url":                       "TestRegistryPageSizeChangesWhatTheCLIAsksFor",
	"deps.source":                        "TestDepsSourceNoneDropsTheManifestLayer",
	"deps.autoInstall":                   "TestDepsAutoInstallSelectsTheBunInstallPolicyFromTheEnvironment",
	"deps.manifest":                      "TestDepsManifestNamesTheManifestResolutionReads",
	"deps.packages":                      "TestDepsPackagesPinContradictingTheManifestFailsTheRun",
	"paths.deps":                         "TestPathsDepsExtendsManifestDiscovery",
	"proxy.timeout":                      "TestProxyUpstreamTimeoutIsReachableFromTheEnvironment",
	"tasks.maxAge":                       "TestTasksMaxAgeIsReachableFromEnvironment",
	"stats.top":                          "TestStatsTopLimitsTheRowsTheStatsAPIReports",
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
	"daemon.takeoverTimeout":             "TestDaemonTakeoverTimeoutBoundsTheHandoverFromTheEnvironment",
	"daemon.socketProbeTimeout":          "TestDaemonSocketProbeTimeoutDecidesWhetherALiveSocketIsStale",
	"logging.maxAge":                     "TestLoggingMaxAgeRotatesAnAgedFile",
	"logstore.followBacklog":             "TestFollowBacklogCapsWhatOneFollowPollEmits",
	"paths.config":                       "TestPathsConfigReadsAConfigOnlyThatPathHolds",
	"script.imports":                     "TestScriptImportsOffSkipsTheImportPipeline",
	"script.importsOnDynamic":            "TestScriptImportsOnDynamicDecidesAComputedImport",
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

// effectProbeFuncs anchors every declared probe name to the actual
// test function, as a Go reference. The maps above are strings, and a
// string cannot notice when the function it names is deleted or
// renamed; this map can, because deleting or renaming a probe then
// breaks compilation of this package. The sync test requires every
// implemented probe name to appear here, so a declared probe must
// really exist -- not just be spelled the same in two maps.
var effectProbeFuncs = map[string]func(*testing.T){
	"TestLoggingFileOffWritesNoDurableLog":                                           TestLoggingFileOffWritesNoDurableLog,
	"TestOutputJSONIsReachableFromTheEnvironment":                                    TestOutputJSONIsReachableFromTheEnvironment,
	"TestOutputColorIsReachableFromTheEnvironment":                                   TestOutputColorIsReachableFromTheEnvironment,
	"TestPathsConfigFileChangesWhichConfigLoads":                                     TestPathsConfigFileChangesWhichConfigLoads,
	"TestPathsScriptsFindsRecipesThatLiveOnlyThere":                                  TestPathsScriptsFindsRecipesThatLiveOnlyThere,
	"TestPathsCacheChangesTheDoctorDirectory":                                        TestPathsCacheChangesTheDoctorDirectory,
	"TestPathsPluginsDiscoversAPluginThatLivesOnlyThere":                             TestPathsPluginsDiscoversAPluginThatLivesOnlyThere,
	"TestProxyUpstreamTimeoutIsReachableFromTheEnvironment":                          TestProxyUpstreamTimeoutIsReachableFromTheEnvironment,
	"TestPluginAnnotateChangesToolResultsFromTheEnvironment":                         TestPluginAnnotateChangesToolResultsFromTheEnvironment,
	"TestPluginBackendChoosesDaemonOrBinaryFromTheEnvironment":                       TestPluginBackendChoosesDaemonOrBinaryFromTheEnvironment,
	"TestPluginDaemonToolsAreOfferedFromTheEnvironment":                              TestPluginDaemonToolsAreOfferedFromTheEnvironment,
	"TestPluginDiscoveryRetryDelaysRediscoveryFromTheEnvironment":                    TestPluginDiscoveryRetryDelaysRediscoveryFromTheEnvironment,
	"TestPluginBinInvokesTheConfiguredExecutableFromTheEnvironment":                  TestPluginBinInvokesTheConfiguredExecutableFromTheEnvironment,
	"TestPluginBinArgsPrependsTheConfiguredArgumentsFromTheEnvironment":              TestPluginBinArgsPrependsTheConfiguredArgumentsFromTheEnvironment,
	"TestPluginEnvLevelChangesShellEnvFromTheEnvironment":                            TestPluginEnvLevelChangesShellEnvFromTheEnvironment,
	"TestPluginToolsRegistersTheToolSetFromTheEnvironment":                           TestPluginToolsRegistersTheToolSetFromTheEnvironment,
	"TestPluginToolTimingRecordsToolCallsFromTheEnvironment":                         TestPluginToolTimingRecordsToolCallsFromTheEnvironment,
	"TestPluginRememberScopePersistsTheDaemonChoiceFromTheEnvironment":               TestPluginRememberScopePersistsTheDaemonChoiceFromTheEnvironment,
	"TestPluginHeadlessSuppressesToastsFromTheEnvironment":                           TestPluginHeadlessSuppressesToastsFromTheEnvironment,
	"TestPluginInstructionsAppendsTheContextParagraphFromTheEnvironment":             TestPluginInstructionsAppendsTheContextParagraphFromTheEnvironment,
	"TestPluginDisabledToolsHidesToolsFromTheEnvironment":                            TestPluginDisabledToolsHidesToolsFromTheEnvironment,
	"TestPoolDefaultsAreReachableFromTheEnvironment":                                 TestPoolDefaultsAreReachableFromTheEnvironment,
	"TestRegistryLimitReachesTheCLI":                                                 TestRegistryLimitReachesTheCLI,
	"TestRegistryPageSizeChangesWhatTheCLIAsksFor":                                   TestRegistryPageSizeChangesWhatTheCLIAsksFor,
	"TestScriptCaptureConsoleOffDropsConsoleOutput":                                  TestScriptCaptureConsoleOffDropsConsoleOutput,
	"TestScriptCwdIsReachableFromTheEnvironment":                                     TestScriptCwdIsReachableFromTheEnvironment,
	"TestScriptInterpreterIsReachableFromTheEnvironment":                             TestScriptInterpreterIsReachableFromTheEnvironment,
	"TestScriptEnvIsReachableFromTheEnvironment":                                     TestScriptEnvIsReachableFromTheEnvironment,
	"TestScriptDevshellIsReachableFromTheEnvironment":                                TestScriptDevshellIsReachableFromTheEnvironment,
	"TestScriptPrefixIsReachableFromTheEnvironment":                                  TestScriptPrefixIsReachableFromTheEnvironment,
	"TestScriptTypecheckOnRefusesATypeError":                                         TestScriptTypecheckOnRefusesATypeError,
	"TestScriptCheckOnErrorDecidesOnACheckError":                                     TestScriptCheckOnErrorDecidesOnACheckError,
	"TestScriptImportsCrossProjectDecidesAForeignProjectEdge":                        TestScriptImportsCrossProjectDecidesAForeignProjectEdge,
	"TestScriptImportsOnMissingDecidesOnAMissingPackage":                             TestScriptImportsOnMissingDecidesOnAMissingPackage,
	"TestScriptImportsNetworkDecidesWhetherToPinAnUndeclaredPackage":                 TestScriptImportsNetworkDecidesWhetherToPinAnUndeclaredPackage,
	"TestSearchLimitCapsTheResults":                                                  TestSearchLimitCapsTheResults,
	"TestStateDirectoryIsReachableAsASetting":                                        TestStateDirectoryIsReachableAsASetting,
	"TestStrictUnknownKeysRefusesATypo":                                              TestStrictUnknownKeysRefusesATypo,
	"TestTheRegistryPageCapSaysItStoppedEarly":                                       TestTheRegistryPageCapSaysItStoppedEarly,
	"TestDepsSourceNoneDropsTheManifestLayer":                                        TestDepsSourceNoneDropsTheManifestLayer,
	"TestDepsAutoInstallSelectsTheBunInstallPolicyFromTheEnvironment":                TestDepsAutoInstallSelectsTheBunInstallPolicyFromTheEnvironment,
	"TestDepsManifestNamesTheManifestResolutionReads":                                TestDepsManifestNamesTheManifestResolutionReads,
	"TestDepsPackagesPinContradictingTheManifestFailsTheRun":                         TestDepsPackagesPinContradictingTheManifestFailsTheRun,
	"TestPathsDepsExtendsManifestDiscovery":                                          TestPathsDepsExtendsManifestDiscovery,
	"TestTasksMaxAgeIsReachableFromEnvironment":                                      TestTasksMaxAgeIsReachableFromEnvironment,
	"TestStatsTopLimitsTheRowsTheStatsAPIReports":                                    TestStatsTopLimitsTheRowsTheStatsAPIReports,
	"TestAskPollCeilingIsReachableFromTheEnvironment":                                TestAskPollCeilingIsReachableFromTheEnvironment,
	"TestAskRoundsLimitIsReachableFromTheEnvironment":                                TestAskRoundsLimitIsReachableFromTheEnvironment,
	"TestAskTTLBoundsAnInterruptibleCallFromTheEnvironment":                          TestAskTTLBoundsAnInterruptibleCallFromTheEnvironment,
	"TestAskTimeoutReleasesAHeldRequestFromTheEnvironment":                           TestAskTimeoutReleasesAHeldRequestFromTheEnvironment,
	"TestStateTTLExpiresAResumableRequestFromTheEnvironment":                         TestStateTTLExpiresAResumableRequestFromTheEnvironment,
	"TestSessionIdleReapsAnUnusedSessionFromTheEnvironment":                          TestSessionIdleReapsAnUnusedSessionFromTheEnvironment,
	"TestMCPPathMovesTheEndpointFromTheEnvironment":                                  TestMCPPathMovesTheEndpointFromTheEnvironment,
	"TestNativeAsksTheCallingClientFromTheEnvironment":                               TestNativeAsksTheCallingClientFromTheEnvironment,
	"TestListMaxAgeSetsTheListCacheHintFromTheEnvironment":                           TestListMaxAgeSetsTheListCacheHintFromTheEnvironment,
	"TestReadMaxAgeSetsTheReadCacheHintFromTheEnvironment":                           TestReadMaxAgeSetsTheReadCacheHintFromTheEnvironment,
	"TestTaskAfterHandsASlowCallATaskFromTheEnvironment":                             TestTaskAfterHandsASlowCallATaskFromTheEnvironment,
	"TestTaskPollIntervalSetsTheTaskPollHintFromTheEnvironment":                      TestTaskPollIntervalSetsTheTaskPollHintFromTheEnvironment,
	"TestTheRevisionPolicyDecidesWhetherProgressKeepsItsMessage":                     TestTheRevisionPolicyDecidesWhetherProgressKeepsItsMessage,
	"TestTheRevisionPolicyDecidesWhetherAServerInitiatedRequestIsAnswered":           TestTheRevisionPolicyDecidesWhetherAServerInitiatedRequestIsAnswered,
	"TestSpecPrecedenceDecidesWhetherProgressKeepsItsMessageFromTheEnvironment":      TestSpecPrecedenceDecidesWhetherProgressKeepsItsMessageFromTheEnvironment,
	"TestExecMaxOutputBytesCapsStdoutFromTheEnvironment":                             TestExecMaxOutputBytesCapsStdoutFromTheEnvironment,
	"TestExecTimeoutStopsALongScriptFromTheEnvironment":                              TestExecTimeoutStopsALongScriptFromTheEnvironment,
	"TestExecOutputShapesTheResultFromTheEnvironment":                                TestExecOutputShapesTheResultFromTheEnvironment,
	"TestMCPPageSizePagesToolsListFromTheEnvironment":                                TestMCPPageSizePagesToolsListFromTheEnvironment,
	"TestMCPPassthroughServesANamespaceDirectlyFromTheEnvironment":                   TestMCPPassthroughServesANamespaceDirectlyFromTheEnvironment,
	"TestCompletionPerMinuteLimitsOneCallerFromTheEnvironment":                       TestCompletionPerMinuteLimitsOneCallerFromTheEnvironment,
	"TestCompletionServerPerMinuteLimitsAllCallersFromTheEnvironment":                TestCompletionServerPerMinuteLimitsAllCallersFromTheEnvironment,
	"TestLogstoreQueryLimitBoundsALogQueryFromTheEnvironment":                        TestLogstoreQueryLimitBoundsALogQueryFromTheEnvironment,
	"TestRecipesMinScoreRefusesAWeakMatchFromTheEnvironment":                         TestRecipesMinScoreRefusesAWeakMatchFromTheEnvironment,
	"TestRecipesMatchMarginSettlesATieFromTheEnvironment":                            TestRecipesMatchMarginSettlesATieFromTheEnvironment,
	"TestRecipesLimitBoundsTheRankingFromTheEnvironment":                             TestRecipesLimitBoundsTheRankingFromTheEnvironment,
	"TestTasksTTLSetsAnExecTasksRetentionFromTheEnvironment":                         TestTasksTTLSetsAnExecTasksRetentionFromTheEnvironment,
	"TestTasksResultWaitBoundsCollectingAResultFromTheEnvironment":                   TestTasksResultWaitBoundsCollectingAResultFromTheEnvironment,
	"TestProtoTasksMaxTTLClampsATasksRetentionFromTheEnvironment":                    TestProtoTasksMaxTTLClampsATasksRetentionFromTheEnvironment,
	"TestProtoTasksMaxPerRequestorCapsUnfinishedTasksFromTheEnvironment":             TestProtoTasksMaxPerRequestorCapsUnfinishedTasksFromTheEnvironment,
	"TestProtoTasksPerMinuteWindowsTaskReadsFromTheEnvironment":                      TestProtoTasksPerMinuteWindowsTaskReadsFromTheEnvironment,
	"TestProtoTasksCancelGraceExpiresACancelledTaskFromTheEnvironment":               TestProtoTasksCancelGraceExpiresACancelledTaskFromTheEnvironment,
	"TestEventsHistoryBoundsTheReplay":                                               TestEventsHistoryBoundsTheReplay,
	"TestEventsReconnectWaitsForTheConfiguredDelay":                                  TestEventsReconnectWaitsForTheConfiguredDelay,
	"TestScriptCheckOnUnsupportedDecidesWhatANonDenoCheckDoes":                       TestScriptCheckOnUnsupportedDecidesWhatANonDenoCheckDoes,
	"TestDaemonTakeoverTimeoutBoundsTheHandoverFromTheEnvironment":                   TestDaemonTakeoverTimeoutBoundsTheHandoverFromTheEnvironment,
	"TestDaemonSocketProbeTimeoutDecidesWhetherALiveSocketIsStale":                   TestDaemonSocketProbeTimeoutDecidesWhetherALiveSocketIsStale,
	"TestLoggingMaxAgeRotatesAnAgedFile":                                             TestLoggingMaxAgeRotatesAnAgedFile,
	"TestFollowBacklogCapsWhatOneFollowPollEmits":                                    TestFollowBacklogCapsWhatOneFollowPollEmits,
	"TestPathsConfigReadsAConfigOnlyThatPathHolds":                                   TestPathsConfigReadsAConfigOnlyThatPathHolds,
	"TestScriptImportsOffSkipsTheImportPipeline":                                     TestScriptImportsOffSkipsTheImportPipeline,
	"TestScriptImportsOnDynamicDecidesAComputedImport":                               TestScriptImportsOnDynamicDecidesAComputedImport,
	"TestPromptRunTimeoutBoundsARecipeRun":                                           TestPromptRunTimeoutBoundsARecipeRun,
	"TestDaemonRootsAnswersAnUpstreamRootsRequest":                                   TestDaemonRootsAnswersAnUpstreamRootsRequest,
	"TestPromptSampleTimeoutBoundsASamplingQuestion":                                 TestPromptSampleTimeoutBoundsASamplingQuestion,
	"TestPromptSampleAskLetsAnUnmatchedPromptSeekAModel":                             TestPromptSampleAskLetsAnUnmatchedPromptSeekAModel,
	"TestPromptCatalogBudgetBoundsTheCatalogSentToTheModel":                          TestPromptCatalogBudgetBoundsTheCatalogSentToTheModel,
	"TestSearchSemanticRanksWithTheConfiguredEmbedder":                               TestSearchSemanticRanksWithTheConfiguredEmbedder,
	"TestEmbeddingsURLSendsSemanticSearchToTheEndpoint":                              TestEmbeddingsURLSendsSemanticSearchToTheEndpoint,
	"TestEmbeddingsAPIKeyIsSentAsABearerToken":                                       TestEmbeddingsAPIKeyIsSentAsABearerToken,
	"TestEmbeddingsModelNamesTheRequestedModel":                                      TestEmbeddingsModelNamesTheRequestedModel,
	"TestEmbeddingsBackendLocalSkipsTheRemoteEndpoint":                               TestEmbeddingsBackendLocalSkipsTheRemoteEndpoint,
	"TestSamplingPerMinuteRefusesAServersSecondSampling":                             TestSamplingPerMinuteRefusesAServersSecondSampling,
	"TestSamplingSessionPerMinuteRefusesASessionsSecondSampling":                     TestSamplingSessionPerMinuteRefusesASessionsSecondSampling,
	"TestElicitationPerMinuteRefusesAServersSecondQuestion":                          TestElicitationPerMinuteRefusesAServersSecondQuestion,
	"TestSamplingAnswerLocallyAnswersAnUpstreamFromTheEnvironment":                   TestSamplingAnswerLocallyAnswersAnUpstreamFromTheEnvironment,
	"TestModelProviderSelectsTheBackendFromTheEnvironment":                           TestModelProviderSelectsTheBackendFromTheEnvironment,
	"TestModelBaseURLPointsTheProviderAtAnEndpointFromTheEnvironment":                TestModelBaseURLPointsTheProviderAtAnEndpointFromTheEnvironment,
	"TestModelDefaultNamesTheModelFromTheEnvironment":                                TestModelDefaultNamesTheModelFromTheEnvironment,
	"TestModelAPIKeyEnvNamesTheKeyVariableFromTheEnvironment":                        TestModelAPIKeyEnvNamesTheKeyVariableFromTheEnvironment,
	"TestModelBudgetCapsAHooksTokensFromTheEnvironment":                              TestModelBudgetCapsAHooksTokensFromTheEnvironment,
	"TestPromptMaxTokensSetsAHooksTokenCapFromTheEnvironment":                        TestPromptMaxTokensSetsAHooksTokenCapFromTheEnvironment,
	"TestModelProvidersRoutesAProviderToItsEndpointFromTheEnvironment":               TestModelProvidersRoutesAProviderToItsEndpointFromTheEnvironment,
	"TestModelProfilesResolvesANamedChainFromTheEnvironment":                         TestModelProfilesResolvesANamedChainFromTheEnvironment,
	"TestModelTimeoutBoundsAGenerationFromTheEnvironment":                            TestModelTimeoutBoundsAGenerationFromTheEnvironment,
	"TestSessionReleaseTimeoutBoundsTheEndOfRunReleaseFromTheEnvironment":            TestSessionReleaseTimeoutBoundsTheEndOfRunReleaseFromTheEnvironment,
	"TestDaemonIdleExitStopsAnIdleDaemonFromTheEnvironment":                          TestDaemonIdleExitStopsAnIdleDaemonFromTheEnvironment,
	"TestDaemonPortBindsTheChosenPortFromTheEnvironment":                             TestDaemonPortBindsTheChosenPortFromTheEnvironment,
	"TestDaemonSaveIntervalWritesTheLeaseTableFromTheEnvironment":                    TestDaemonSaveIntervalWritesTheLeaseTableFromTheEnvironment,
	"TestAutostartPollIntervalPacesTheReadinessPollFromTheEnvironment":               TestAutostartPollIntervalPacesTheReadinessPollFromTheEnvironment,
	"TestDaemonLeaseTTLForgetsASilentCallerFromTheEnvironment":                       TestDaemonLeaseTTLForgetsASilentCallerFromTheEnvironment,
	"TestElicitDisambiguateAsksWhichInstanceFromTheEnvironment":                      TestElicitDisambiguateAsksWhichInstanceFromTheEnvironment,
	"TestElicitDisambiguateDefaultPicksTheRecentInstanceFromTheEnvironment":          TestElicitDisambiguateDefaultPicksTheRecentInstanceFromTheEnvironment,
	"TestElicitTTLSetsAnUpstreamQuestionsDeadlineFromTheEnvironment":                 TestElicitTTLSetsAnUpstreamQuestionsDeadlineFromTheEnvironment,
	"TestElicitMaxAgeCapsAnUpstreamQuestionsDeadlineFromTheEnvironment":              TestElicitMaxAgeCapsAnUpstreamQuestionsDeadlineFromTheEnvironment,
	"TestElicitPendingLimitBoundsTheListingFromTheEnvironment":                       TestElicitPendingLimitBoundsTheListingFromTheEnvironment,
	"TestElicitAskTimeoutBoundsTheWaitForAnAnswerFromTheEnvironment":                 TestElicitAskTimeoutBoundsTheWaitForAnAnswerFromTheEnvironment,
	"TestArtifactsIdleTTLRenewsAReadArtifactFromTheEnvironment":                      TestArtifactsIdleTTLRenewsAReadArtifactFromTheEnvironment,
	"TestArtifactsMaxAgeCapsARenewedArtifactFromTheEnvironment":                      TestArtifactsMaxAgeCapsARenewedArtifactFromTheEnvironment,
	"TestArtifactsTTLExpiresAnArtifactFromTheEnvironment":                            TestArtifactsTTLExpiresAnArtifactFromTheEnvironment,
	"TestArtifactsGCIntervalSetsWhenExpiredArtifactsAreSweptFromTheEnvironment":      TestArtifactsGCIntervalSetsWhenExpiredArtifactsAreSweptFromTheEnvironment,
	"TestPoolMinKeepsAnInstanceWarmFromTheEnvironment":                               TestPoolMinKeepsAnInstanceWarmFromTheEnvironment,
	"TestAutostartIdleExitStopsAnIdleDaemonFromTheEnvironment":                       TestAutostartIdleExitStopsAnIdleDaemonFromTheEnvironment,
	"TestDaemonWarmReadsTheServersAtStartupFromTheEnvironment":                       TestDaemonWarmReadsTheServersAtStartupFromTheEnvironment,
	"TestPoolIdleTimeoutReapsAnIdleInstanceFromTheEnvironment":                       TestPoolIdleTimeoutReapsAnIdleInstanceFromTheEnvironment,
	"TestDaemonReapIntervalSetsWhenIdleInstancesAreReapedFromTheEnvironment":         TestDaemonReapIntervalSetsWhenIdleInstancesAreReapedFromTheEnvironment,
	"TestDaemonEndpointNamesTheDaemonToReachFromTheEnvironment":                      TestDaemonEndpointNamesTheDaemonToReachFromTheEnvironment,
	"TestUpstreamProbeTimeoutBoundsTheDiscoverWaitFromTheEnvironment":                TestUpstreamProbeTimeoutBoundsTheDiscoverWaitFromTheEnvironment,
	"TestTheEraIsProbedOnceAndRememberedAcrossDaemonRestarts":                        TestTheEraIsProbedOnceAndRememberedAcrossDaemonRestarts,
	"TestPromptAutonomyRunsTheMatchedRecipeFromTheEnvironment":                       TestPromptAutonomyRunsTheMatchedRecipeFromTheEnvironment,
	"TestHooksAutonomySkipsAConfiguredHookFromTheEnvironment":                        TestHooksAutonomySkipsAConfiguredHookFromTheEnvironment,
	"TestRepairAutonomyOffDropsTheDiagnosticsFromTheEnvironment":                     TestRepairAutonomyOffDropsTheDiagnosticsFromTheEnvironment,
	"TestPolicyWriteDeniesAnUnannotatedCallFromTheEnvironment":                       TestPolicyWriteDeniesAnUnannotatedCallFromTheEnvironment,
	"TestElicitConfirmDestructiveGatesADestructiveCallFromTheEnvironment":            TestElicitConfirmDestructiveGatesADestructiveCallFromTheEnvironment,
	"TestAutonomyMaxClampsARecipeRunFromTheEnvironment":                              TestAutonomyMaxClampsARecipeRunFromTheEnvironment,
	"TestDaemonInlineAlwaysAnswersWithoutASocketFromTheEnvironment":                  TestDaemonInlineAlwaysAnswersWithoutASocketFromTheEnvironment,
	"TestAutostartBinChoosesTheDaemonExecutableFromTheEnvironment":                   TestAutostartBinChoosesTheDaemonExecutableFromTheEnvironment,
	"TestAutostartArgsReplaceTheDaemonCommandLineFromTheEnvironment":                 TestAutostartArgsReplaceTheDaemonCommandLineFromTheEnvironment,
	"TestAutostartLogTailBoundsTheFailureExcerptFromTheEnvironment":                  TestAutostartLogTailBoundsTheFailureExcerptFromTheEnvironment,
	"TestDaemonWatchConfigPicksUpAHandEditFromTheEnvironment":                        TestDaemonWatchConfigPicksUpAHandEditFromTheEnvironment,
	"TestDaemonInlineStartTimeoutBoundsTheInProcessStartFromTheEnvironment":          TestDaemonInlineStartTimeoutBoundsTheInProcessStartFromTheEnvironment,
	"TestDaemonProbeTimeoutDecidesWhetherASiblingDaemonIsReportedFromTheEnvironment": TestDaemonProbeTimeoutDecidesWhetherASiblingDaemonIsReportedFromTheEnvironment,
	"TestDaemonWarmTimeoutBoundsTheStartupWarm":                                      TestDaemonWarmTimeoutBoundsTheStartupWarm,
	"TestAutostartConnectTimeoutBoundsTheWaitForADaemon":                             TestAutostartConnectTimeoutBoundsTheWaitForADaemon,
	"TestAutostartPingTimeoutBoundsEachReadinessPing":                                TestAutostartPingTimeoutBoundsEachReadinessPing,
	"TestPoolCallTimeoutBoundsASlowCall":                                             TestPoolCallTimeoutBoundsASlowCall,
	"TestPoolStartTimeoutBoundsServerStart":                                          TestPoolStartTimeoutBoundsServerStart,
	"TestDaemonRefreshTimeoutBoundsARefresh":                                         TestDaemonRefreshTimeoutBoundsARefresh,
	"TestDoctorTimeoutBoundsTheDaemonCheck":                                          TestDoctorTimeoutBoundsTheDaemonCheck,
	"TestRegistryTimeoutBoundsARegistryRequest":                                      TestRegistryTimeoutBoundsARegistryRequest,
	"TestCompletionMaxValuesCapsTheValues":                                           TestCompletionMaxValuesCapsTheValues,
	"TestCatalogInstructionsShowsServerGuidance":                                     TestCatalogInstructionsShowsServerGuidance,
	"TestPlumbingLauncherPlaceholderRepeatAllowsATwiceUsedName":                      TestPlumbingLauncherPlaceholderRepeatAllowsATwiceUsedName,
	"TestPlumbingAllowTsJsOverlapPicksAmbiguousScripts":                              TestPlumbingAllowTsJsOverlapPicksAmbiguousScripts,
	"TestPlumbingSourceDirAllowedTakesADirectoryAsSource":                            TestPlumbingSourceDirAllowedTakesADirectoryAsSource,
	"TestPlumbingSourceDirRecursiveReadsSubdirectories":                              TestPlumbingSourceDirRecursiveReadsSubdirectories,
	"TestPlumbingSourceProbePathsReadsANamedFile":                                    TestPlumbingSourceProbePathsReadsANamedFile,
	"TestArtifactsChunkBytesSplitsAStreamedBody":                                     TestArtifactsChunkBytesSplitsAStreamedBody,
	"TestScriptLauncherReplacesTheLauncher":                                          TestScriptLauncherReplacesTheLauncher,
	"TestPathsPlaceholdersDeclaresALauncherPlaceholder":                              TestPathsPlaceholdersDeclaresALauncherPlaceholder,
	"TestArtifactsInterceptImagesStoresAnEmittedImage":                               TestArtifactsInterceptImagesStoresAnEmittedImage,
	"TestPlumbingIndexOnQueryIngestsBeforeALogQuery":                                 TestPlumbingIndexOnQueryIngestsBeforeALogQuery,
	"TestPlumbingValidatePathsChecksTheLogDirUpFront":                                TestPlumbingValidatePathsChecksTheLogDirUpFront,
	"TestScriptLanguageRunsPython":                                                   TestScriptLanguageRunsPython,
	"TestLoggingTraceLogsEachCall":                                                   TestLoggingTraceLogsEachCall,
	"TestExecWhereRemoteRunsOnTheDaemon":                                             TestExecWhereRemoteRunsOnTheDaemon,
	"TestPathsAdaptersFindsADeclarationFile":                                         TestPathsAdaptersFindsADeclarationFile,
	"TestAdaptersAllowBareGatesABareDeclaration":                                     TestAdaptersAllowBareGatesABareDeclaration,
	"TestPathsAPIsFindsADeclarationFile":                                             TestPathsAPIsFindsADeclarationFile,
	"TestScriptRuntimeSelectsTheRuntime":                                             TestScriptRuntimeSelectsTheRuntime,
	"TestScriptRuntimeOrderRanksTheAutoPick":                                         TestScriptRuntimeOrderRanksTheAutoPick,
	"TestScriptPermissionsSandboxesNode":                                             TestScriptPermissionsSandboxesNode,
	"TestScriptProfilesDefinesAProfile":                                              TestScriptProfilesDefinesAProfile,
	"TestScriptRuntimesDeclaresARuntime":                                             TestScriptRuntimesDeclaresARuntime,
	"TestScriptPhasesRunFromTheEnvironment":                                          TestScriptPhasesRunFromTheEnvironment,
	"TestHooksRunOnTheirEvent":                                                       TestHooksRunOnTheirEvent,
	"TestHooksTimeoutStopsASlowHook":                                                 TestHooksTimeoutStopsASlowHook,
	"TestDaemonUITurnsTheDashboardOff":                                               TestDaemonUITurnsTheDashboardOff,
	"TestDaemonUIDirOverridesTheDashboard":                                           TestDaemonUIDirOverridesTheDashboard,
	"TestLoggingLevelShowsDebugRecords":                                              TestLoggingLevelShowsDebugRecords,
	"TestLoggingFormatRendersJSON":                                                   TestLoggingFormatRendersJSON,
	"TestLoggingSourceAddsTheCallSite":                                               TestLoggingSourceAddsTheCallSite,
	"TestLoggingDirHoldsTheDurableLog":                                               TestLoggingDirHoldsTheDurableLog,
	"TestLoggingIncludeChoosesTheAmbientBlocks":                                      TestLoggingIncludeChoosesTheAmbientBlocks,
	"TestLoggingMaxLinesRotatesTheFile":                                              TestLoggingMaxLinesRotatesTheFile,
	"TestLoggingMaxBytesRotatesTheFile":                                              TestLoggingMaxBytesRotatesTheFile,
	"TestLoggingKeepPrunesRotatedFiles":                                              TestLoggingKeepPrunesRotatedFiles,
	"TestArtifactsEnabledGatesTheStore":                                              TestArtifactsEnabledGatesTheStore,
	"TestArtifactsMaxBytesRefusesALargeArtifact":                                     TestArtifactsMaxBytesRefusesALargeArtifact,
	"TestArtifactsQuotaBoundsTheStore":                                               TestArtifactsQuotaBoundsTheStore,
	"TestArtifactsDeliveryInlinesTheBody":                                            TestArtifactsDeliveryInlinesTheBody,
	"TestArtifactsInlineMaxBytesBoundsAnInlinedBody":                                 TestArtifactsInlineMaxBytesBoundsAnInlinedBody,
	"TestArtifactsDirReceivesTheRunsArtifacts":                                       TestArtifactsDirReceivesTheRunsArtifacts,
	"TestPresetSelectsADefinedBundle":                                                TestPresetSelectsADefinedBundle,
	"TestServeModeSelectsTheServedSurface":                                           TestServeModeSelectsTheServedSurface,
	"TestPresetsDefinesNamedFlagBundles":                                             TestPresetsDefinesNamedFlagBundles,
	"TestCatalogBudgetFitsTheSignatures":                                             TestCatalogBudgetFitsTheSignatures,
	"TestCatalogBiasPicksWhatATightBudgetShows":                                      TestCatalogBiasPicksWhatATightBudgetShows,
	"TestCatalogPinnedToolsAlwaysListInReactiveMode":                                 TestCatalogPinnedToolsAlwaysListInReactiveMode,
	"TestRetentionStickyWindowExpiresReactiveTools":                                  TestRetentionStickyWindowExpiresReactiveTools,
	"TestRetentionMaxToolsCapsTheReactiveWorkingSet":                                 TestRetentionMaxToolsCapsTheReactiveWorkingSet,
	"TestDiagnoseHistoryCapsRecordedSchemaChanges":                                   TestDiagnoseHistoryCapsRecordedSchemaChanges,
	"TestDiagnosePreflightStopsAMismatchedScript":                                    TestDiagnosePreflightStopsAMismatchedScript,
	"TestServeMCPUnmountsTheEndpointFromTheEnvironment":                              TestServeMCPUnmountsTheEndpointFromTheEnvironment,
	"TestDaemonAddressIsReachableFromTheEnvironment":                                 TestDaemonAddressIsReachableFromTheEnvironment,
	"TestDaemonAutostartIsReachableFromTheEnvironment":                               TestDaemonAutostartIsReachableFromTheEnvironment,
	"TestHTTPBodyLimitsRejectOversizedBodies":                                        TestHTTPBodyLimitsRejectOversizedBodies,
	"TestHTTPEventStreamSettingsChangeTheStream":                                     TestHTTPEventStreamSettingsChangeTheStream,
	"TestHTTPReadHeaderTimeoutClosesSlowClients":                                     TestHTTPReadHeaderTimeoutClosesSlowClients,
	"TestHTTPRequestTimeoutBoundsAClientCall":                                        TestHTTPRequestTimeoutBoundsAClientCall,
	"TestHTTPShutdownGraceBoundsHowLongAStopWaits":                                   TestHTTPShutdownGraceBoundsHowLongAStopWaits,
	"TestHTTPStreamBufferMaxEndsAnOversizedEventLine":                                TestHTTPStreamBufferMaxEndsAnOversizedEventLine,
	"TestHTTPIdleConnectionPoolSettingsChangeReuse":                                  TestHTTPIdleConnectionPoolSettingsChangeReuse,
	"TestHTTPIdleConnTimeoutExpiresAnIdleClientConnection":                           TestHTTPIdleConnTimeoutExpiresAnIdleClientConnection,
	"TestTransportAllowedOriginsAreServed":                                           TestTransportAllowedOriginsAreServed,
	"TestTransportBearerTokenGatesTheDaemon":                                         TestTransportBearerTokenGatesTheDaemon,
	"TestTransportSSEKeepAliveCommentsOnAQuietStream":                                TestTransportSSEKeepAliveCommentsOnAQuietStream,
	"TestTransportSessionMetaKeysNameTheSession":                                     TestTransportSessionMetaKeysNameTheSession,
	"TestTransportStdioDrainBoundsServeShutdown":                                     TestTransportStdioDrainBoundsServeShutdown,
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
	testsupport.RequireBinary(t, "deno")
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

// TestScriptCheckOnErrorDecidesOnACheckError is the effect probe for
// script.checkOnError. A script with a type error is checked under Deno.
// Oracle: by default (fail) the run stops before running and reports the
// error; warn reports it and still runs; run stays quiet and runs.
func TestScriptCheckOnErrorDecidesOnACheckError(t *testing.T) {
	testsupport.RequireBinary(t, "deno")
	e := newEnv(t, oneServer)
	script := filepath.Join(e.dir, "bad.ts")
	src := "const n: number = \"not a number\";\n" +
		"export default function () { return n; }\n"
	if err := os.WriteFile(script, []byte(src), 0o644); err != nil {
		t.Fatal(err)
	}
	e.setenv("MCPX_SCRIPT_CHECK=types")

	if out, err := e.try("run", script); err == nil || !strings.Contains(out, "does not type check") {
		t.Fatalf("default (fail) should refuse and report a type error (err=%v):\n%s", err, out)
	}

	e.setenv("MCPX_SCRIPT_CHECK_ON_ERROR=warn")
	out, err := e.try("run", script)
	if err != nil {
		t.Fatalf("warn should run the script: %v\n%s", err, out)
	}
	if !strings.Contains(out, "does not type check") || !strings.Contains(out, "not a number") {
		t.Fatalf("warn should report the error and run the script:\n%s", out)
	}

	e.setenv("MCPX_SCRIPT_CHECK_ON_ERROR=run")
	out, err = e.try("run", script)
	if err != nil {
		t.Fatalf("run should run the script: %v\n%s", err, out)
	}
	if strings.Contains(out, "does not type check") || !strings.Contains(out, "not a number") {
		t.Fatalf("run should stay quiet and run the script:\n%s", out)
	}
}

// TestScriptImportsCrossProjectDecidesAForeignProjectEdge is the effect
// probe for script.importsCrossProject. A script in one project imports a
// module from a second project whose manifest declares a package, so the
// cross-project policy decides whether that foreign subset is isolated,
// merged, or ignored.
func TestScriptImportsCrossProjectDecidesAForeignProjectEdge(t *testing.T) {
	testsupport.RequireBinary(t, "deno")
	e := newEnv(t, oneServer)
	if err := os.MkdirAll(filepath.Join(e.dir, "a"), 0o755); err != nil {
		t.Fatal(err)
	}
	if err := os.MkdirAll(filepath.Join(e.dir, "b"), 0o755); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(e.dir, "a", "main.ts"), []byte("import \"../b/helper.ts\";\nconsole.log(\"ran-main\")\n"), 0o644); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(e.dir, "b", "helper.ts"), []byte("import \"foreign-dep\";\nconsole.log(\"ran-helper\")\n"), 0o644); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(e.dir, "a", "package.json"), []byte(`{"dependencies":{}}`), 0o644); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(e.dir, "b", "package.json"), []byte(`{"dependencies":{"foreign-dep":"1.2.3"}}`), 0o644); err != nil {
		t.Fatal(err)
	}
	script := filepath.Join(e.dir, "a", "main.ts")
	e.setenv("MCPX_SCRIPT_CHECK=off")
	e.setenv("MCPX_SCRIPT_IMPORTS_ON_MISSING=install")

	run := func(mode string) string {
		t.Helper()
		e.setenv("MCPX_SCRIPT_IMPORTS_CROSS_PROJECT=" + mode)
		out, _ := e.try("run", script)
		return out
	}

	if out := run("isolate"); !strings.Contains(out, "isolated dependency subset") {
		t.Fatalf("isolate should report an isolated foreign subset:\n%s", out)
	}
	if out := run("merge"); !strings.Contains(out, "resolved dependency set includes") {
		t.Fatalf("merge should fold the foreign package into the root set:\n%s", out)
	}
	if out := run("ignore"); !strings.Contains(out, "ignored import") {
		t.Fatalf("ignore should report the ignored foreign import:\n%s", out)
	}
}

// TestScriptImportsOnMissingDecidesOnAMissingPackage is the effect probe
// for script.importsOnMissing. A script imports a package declared (but
// not installed) in its own manifest, so the missing-package policy
// decides whether the run stops, warns, or records the set.
func TestScriptImportsOnMissingDecidesOnAMissingPackage(t *testing.T) {
	testsupport.RequireBinary(t, "deno")
	e := newEnv(t, oneServer)
	if err := os.WriteFile(filepath.Join(e.dir, "main.ts"), []byte("import \"foreign-dep\";\nconsole.log(\"ran\")\n"), 0o644); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(e.dir, "package.json"), []byte(`{"dependencies":{"foreign-dep":"1.2.3"}}`), 0o644); err != nil {
		t.Fatal(err)
	}
	script := filepath.Join(e.dir, "main.ts")
	e.setenv("MCPX_SCRIPT_CHECK=off")
	e.setenv("MCPX_SCRIPT_IMPORTS_NETWORK=off")

	run := func(mode string) string {
		t.Helper()
		e.setenv("MCPX_SCRIPT_IMPORTS_ON_MISSING=" + mode)
		out, _ := e.try("run", script)
		return out
	}

	if out := run("fail"); !strings.Contains(out, "unresolved import") || !strings.Contains(out, "but is not installed") {
		t.Fatalf("fail should report an unresolved, not-installed import:\n%s", out)
	}
	if out := run("warn"); !strings.Contains(out, "dependency was not installed") {
		t.Fatalf("warn should report a non-fatal missing dependency:\n%s", out)
	}
	if out := run("install"); !strings.Contains(out, "stage 2 records the set but does not install it") {
		t.Fatalf("install should record the set without installing:\n%s", out)
	}
}

// TestScriptImportsNetworkDecidesWhetherToPinAnUndeclaredPackage is the
// effect probe for script.importsNetwork. A script imports a package
// with no manifest entry, so the network policy decides whether mcpx
// tries to pin it from the registry or refuses up front.
func TestScriptImportsNetworkDecidesWhetherToPinAnUndeclaredPackage(t *testing.T) {
	testsupport.RequireBinary(t, "deno")
	e := newEnv(t, oneServer)
	if err := os.WriteFile(filepath.Join(e.dir, "main.ts"), []byte("import \"undeclared-pkg-xyz\";\nconsole.log(\"ran\")\n"), 0o644); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(e.dir, "package.json"), []byte(`{"dependencies":{}}`), 0o644); err != nil {
		t.Fatal(err)
	}
	script := filepath.Join(e.dir, "main.ts")
	e.setenv("MCPX_SCRIPT_CHECK=off")
	e.setenv("MCPX_SCRIPT_IMPORTS_ON_MISSING=fail")

	if out, _ := func() (string, error) {
		e.setenv("MCPX_SCRIPT_IMPORTS_NETWORK=off")
		return e.try("run", script)
	}(); !strings.Contains(out, "script.importsNetwork is off") {
		t.Fatalf("off should refuse up front naming the disabled network:\n%s", out)
	}
	if out, _ := func() (string, error) {
		e.setenv("MCPX_SCRIPT_IMPORTS_NETWORK=on")
		return e.try("run", script)
	}(); !strings.Contains(out, "cannot pin") {
		t.Fatalf("on should attempt to pin (and report the outcome):\n%s", out)
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
		// Absorb the one-time "reading tool schemas for the first
		// time..." notice a cold sandbox prints to the merged capture,
		// as TestExecOutputShapesTheResultFromTheEnvironment does.
		_ = e.run("exec", `console.log("warmup")`)
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
		// Warm the schema cache first: in a cold environment (a fresh
		// nix build sandbox) the first exec prints a one-time
		// "reading tool schemas for the first time..." progress notice
		// to stderr, and the harness captures stderr together with
		// stdout. The notice is not part of the output shape under
		// test, so absorb it in a throwaway run.
		_ = e.run("exec", `console.log("warmup")`)
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

// promptWithRecipes saves the say recipe under each name and runs
// `mcpx prompt` for "echo a message back" with the message filled in.
func promptWithRecipes(t *testing.T, env []string, names ...string) string {
	t.Helper()
	e := newEnv(t, oneServer)
	e.setenv(env...)
	e.withoutModelCLIs(t)
	for _, name := range names {
		writeRecipe(t, e, name, sayRecipe)
	}
	e.run("ls")
	return e.run("prompt", "--set", "message=hello", "echo a message back")
}

// recipes.minScore is the score below which no recipe counts as a match.
// Oracle: with one saved recipe, `mcpx prompt` uses it at the default;
// with MCPX_RECIPES_MIN_SCORE=100000 nothing matches.
func TestRecipesMinScoreRefusesAWeakMatchFromTheEnvironment(t *testing.T) {
	if got := promptWithRecipes(t, nil, "say"); !strings.Contains(got, "recipe say") {
		t.Fatalf("at the default recipes.minScore the say recipe should be used:\n%s", got)
	}
	if got := promptWithRecipes(t, []string{"MCPX_RECIPES_MIN_SCORE=100000"}, "say"); !strings.Contains(got, "no recipe matched") {
		t.Fatalf("recipes.minScore=100000 should leave nothing matched:\n%s", got)
	}
}

// recipes.matchMargin is how far ahead the best recipe must be. Two
// identical recipes tie.
// Oracle: at the default a tie is no match; with
// MCPX_RECIPES_MATCH_MARGIN=0 the first of the tied recipes is used.
func TestRecipesMatchMarginSettlesATieFromTheEnvironment(t *testing.T) {
	if got := promptWithRecipes(t, nil, "say", "shout"); !strings.Contains(got, "no recipe matched") {
		t.Fatalf("at the default recipes.matchMargin two tied recipes should not match:\n%s", got)
	}
	if got := promptWithRecipes(t, []string{"MCPX_RECIPES_MATCH_MARGIN=0"}, "say", "shout"); !strings.Contains(got, "recipe say") {
		t.Fatalf("recipes.matchMargin=0 should use the first of two tied recipes:\n%s", got)
	}
}

// recipes.limit is how many ranked recipes are returned.
// Oracle: `mcpx recipes match` lists all three matching recipes at the
// default and only one with MCPX_RECIPES_LIMIT=1.
func TestRecipesLimitBoundsTheRankingFromTheEnvironment(t *testing.T) {
	listed := func(env ...string) int {
		t.Helper()
		e := newEnv(t, oneServer)
		e.setenv(env...)
		for _, name := range []string{"say", "shout", "speak"} {
			writeRecipe(t, e, name, sayRecipe)
		}
		e.run("ls")
		out := e.run("recipes", "match", "echo a message")
		n := 0
		for _, name := range []string{" say ", " shout ", " speak "} {
			if strings.Contains(out, name) {
				n++
			}
		}
		return n
	}

	if n := listed(); n != 3 {
		t.Fatalf("at the default recipes.limit all three recipes should be ranked, got %d", n)
	}
	if n := listed("MCPX_RECIPES_LIMIT=1"); n != 1 {
		t.Fatalf("recipes.limit=1 should rank one recipe, got %d", n)
	}
}

// startExecTask starts an /v1/exec script as a task on a fresh daemon
// and returns the socket client and the task the daemon handed back.
func startExecTask(t *testing.T, source string, env ...string) (*http.Client, map[string]any) {
	t.Helper()
	e := newEnv(t, oneServer)
	e.setenv(env...)
	e.run("ls")
	c := e.socketClient(t)
	body, _ := json.Marshal(map[string]any{"source": source, "options": map[string]any{"task": map[string]any{}}})
	code, doc := postJSON(t, c, "/v1/exec", string(body))
	task, _ := doc["task"].(map[string]any)
	if code != http.StatusAccepted || task["taskId"] == nil {
		t.Fatalf("an exec task should be accepted with a handle, got %d %v", code, doc)
	}
	return c, task
}

// tasks.ttl is how long a finished task's result is kept, when the caller
// names no retention.
// Oracle: an /v1/exec task carries ttl 600000 (10m) at the default and
// 7000 with MCPX_TASKS_TTL=7s.
func TestTasksTTLSetsAnExecTasksRetentionFromTheEnvironment(t *testing.T) {
	ttl := func(env ...string) any {
		t.Helper()
		_, task := startExecTask(t, `console.log("kept")`, env...)
		return task["ttl"]
	}

	if got := ttl(); got != float64(600000) {
		t.Fatalf("at the default tasks.ttl an exec task should carry ttl 600000, got %v", got)
	}
	if got := ttl("MCPX_TASKS_TTL=7s"); got != float64(7000) {
		t.Fatalf("tasks.ttl=7s should give an exec task ttl 7000, got %v", got)
	}
}

// tasks.resultWait is how long collecting a task's result blocks when the
// caller names no wait. The task's script takes three seconds.
// Oracle: at the default (2m) the result request waits and gets the
// script's output; with MCPX_TASKS_RESULT_WAIT=500ms it gives up first
// with 408.
func TestTasksResultWaitBoundsCollectingAResultFromTheEnvironment(t *testing.T) {
	collect := func(env ...string) (int, string) {
		t.Helper()
		c, task := startExecTask(t, `await new Promise((r) => setTimeout(r, 3000)); console.log("collected")`, env...)
		// The socket client's own timeout is shorter than the script.
		patient := *c
		patient.Timeout = 30 * time.Second
		resp, err := patient.Get("http://mcpx/v1/tasks/" + task["taskId"].(string) + "/result")
		if err != nil {
			t.Fatal(err)
		}
		defer func() { _ = resp.Body.Close() }()
		raw, _ := io.ReadAll(resp.Body)
		return resp.StatusCode, string(raw)
	}

	if code, body := collect(); code != http.StatusOK || !strings.Contains(body, "collected") {
		t.Fatalf("at the default tasks.resultWait the result request should wait for the script, got %d:\n%s", code, body)
	}
	if code, body := collect("MCPX_TASKS_RESULT_WAIT=500ms"); code != http.StatusRequestTimeout {
		t.Fatalf("tasks.resultWait=500ms should give up before a three-second script finishes with 408, got %d:\n%s", code, body)
	}
}

// tasksClient is a 2026-07-28 client that declared the tasks extension,
// against a daemon that hands any call over 100ms a task.
func tasksClient(t *testing.T, env ...string) func(method string, params map[string]any) map[string]any {
	t.Helper()
	e := newEnv(t, oneServer)
	e.setenv(append([]string{"MCPX_PROTO_MESSAGES_TASK_AFTER=100ms"}, env...)...)
	e.run("refresh")
	ep := e.endpoint(t)
	id := 0
	return func(method string, params map[string]any) map[string]any {
		t.Helper()
		id++
		params["_meta"] = map[string]any{
			"io.modelcontextprotocol/protocolVersion": "2026-07-28",
			"io.modelcontextprotocol/clientCapabilities": map[string]any{
				"extensions": map[string]any{"io.modelcontextprotocol/tasks": map[string]any{}},
			},
		}
		resp := mcpPost(t, ep, "", map[string]any{"jsonrpc": "2.0", "id": id, "method": method, "params": params})
		defer func() { _ = resp.Body.Close() }()
		return decodeJSON(t, resp.Body)
	}
}

// startSlowTask calls a two-second script, which the 100ms threshold
// turns into a task, and returns the reply.
func startSlowTask(t *testing.T, rpc func(string, map[string]any) map[string]any) map[string]any {
	t.Helper()
	return rpc("tools/call", map[string]any{"name": "mcpx_exec",
		"arguments": map[string]any{"source": "await new Promise((r) => setTimeout(r, 2000))"}})
}

// protoTasks.maxTTL clamps the retention of a task mcpx hands out.
// Oracle: the task carries ttlMs 600000 at the default and 5000 with
// MCPX_PROTO_TASKS_MAX_TTL=5s.
func TestProtoTasksMaxTTLClampsATasksRetentionFromTheEnvironment(t *testing.T) {
	ttl := func(env ...string) any {
		t.Helper()
		doc := startSlowTask(t, tasksClient(t, env...))
		res, _ := doc["result"].(map[string]any)
		if res["resultType"] != "task" {
			t.Fatalf("a two-second call past a 100ms threshold should be a task:\n%s", dumpJSON(t, doc))
		}
		return res["ttlMs"]
	}

	if got := ttl(); got != float64(600000) {
		t.Fatalf("at the default protoTasks.maxTTL a task should carry ttlMs 600000, got %v", got)
	}
	if got := ttl("MCPX_PROTO_TASKS_MAX_TTL=5s"); got != float64(5000) {
		t.Fatalf("protoTasks.maxTTL=5s should clamp a task to ttlMs 5000, got %v", got)
	}
}

// protoTasks.maxPerRequestor caps one requestor's unfinished tasks.
// Oracle: two slow calls both become tasks at the default; with
// MCPX_PROTO_TASKS_MAX_PER_REQUESTOR=1 the second is refused.
func TestProtoTasksMaxPerRequestorCapsUnfinishedTasksFromTheEnvironment(t *testing.T) {
	second := func(env ...string) string {
		t.Helper()
		rpc := tasksClient(t, env...)
		startSlowTask(t, rpc)
		return dumpJSON(t, startSlowTask(t, rpc))
	}

	if got := second(); !strings.Contains(got, `"resultType":"task"`) {
		t.Fatalf("at the default protoTasks.maxPerRequestor a second slow call should still become a task:\n%s", got)
	}
	if got := second("MCPX_PROTO_TASKS_MAX_PER_REQUESTOR=1"); !strings.Contains(got, "per-requestor task cap reached") {
		t.Fatalf("protoTasks.maxPerRequestor=1 should refuse a second unfinished task:\n%s", got)
	}
}

// protoTasks.perMinute is the per-requestor admission window for tasks/*
// calls. A task is started and then read twice with tasks/get.
// Oracle: both reads succeed at the default; with
// MCPX_PROTO_TASKS_PER_MINUTE=1 the second is refused.
func TestProtoTasksPerMinuteWindowsTaskReadsFromTheEnvironment(t *testing.T) {
	secondRead := func(env ...string) string {
		t.Helper()
		rpc := tasksClient(t, env...)
		res, _ := startSlowTask(t, rpc)["result"].(map[string]any)
		id, _ := res["taskId"].(string)
		if id == "" {
			t.Fatalf("no task was started: %v", res)
		}
		rpc("tasks/get", map[string]any{"taskId": id})
		return dumpJSON(t, rpc("tasks/get", map[string]any{"taskId": id}))
	}

	if got := secondRead(); !strings.Contains(got, `"taskId"`) || strings.Contains(got, "admission window") {
		t.Fatalf("at the default protoTasks.perMinute a second tasks/get should succeed:\n%s", got)
	}
	if got := secondRead("MCPX_PROTO_TASKS_PER_MINUTE=1"); !strings.Contains(got, "admission window") {
		t.Fatalf("protoTasks.perMinute=1 should refuse a second tasks/get:\n%s", got)
	}
}

// protoTasks.cancelGrace is how long work on the modern tasks
// extension path may still finish after a tasks/cancel before the
// store records the cancellation anyway (cooperative Store.Cancel).
// The task is a two-second script; cancelling it starts an unwind
// that takes measurable time to record the script's own outcome, so
// the window's edge is observable: a window no work can finish
// inside loses the record to the cancellation, a wide one keeps it.
// Oracle: with MCPX_PROTO_TASKS_CANCEL_GRACE=1us the store records
// the task cancelled; with a 30s grace the work's own outcome
// stands and the task reads completed.
func TestProtoTasksCancelGraceExpiresACancelledTaskFromTheEnvironment(t *testing.T) {
	statusAfterCancel := func(env ...string) string {
		t.Helper()
		rpc := tasksClient(t, env...)
		res, _ := startSlowTask(t, rpc)["result"].(map[string]any)
		id, _ := res["taskId"].(string)
		if id == "" {
			t.Fatalf("no task was started: %v", res)
		}
		rpc("tasks/cancel", map[string]any{"taskId": id})
		// Read until the record turns terminal; either outcome
		// lands within moments of the cancel.
		deadline := time.Now().Add(10 * time.Second)
		for {
			got, _ := rpc("tasks/get", map[string]any{"taskId": id})["result"].(map[string]any)
			status, _ := got["status"].(string)
			if status == "cancelled" || status == "completed" || status == "failed" || time.Now().After(deadline) {
				return status
			}
			time.Sleep(25 * time.Millisecond)
		}
	}

	if got := statusAfterCancel("MCPX_PROTO_TASKS_CANCEL_GRACE=1us"); got != "cancelled" {
		t.Fatalf("protoTasks.cancelGrace=1us should record the task cancelled once the grace expires, got status %q", got)
	}
	if got := statusAfterCancel("MCPX_PROTO_TASKS_CANCEL_GRACE=30s"); got != "completed" {
		t.Fatalf("with a 30s protoTasks.cancelGrace the cancelled task's own outcome should stand, got status %q", got)
	}
}

// replayFromStart reads what /v1/events replays from sequence zero. The
// stream stays open for live events, so it reads until the client's short
// deadline and keeps whatever arrived.
func replayFromStart(t *testing.T, e *env) string {
	t.Helper()
	c := *e.socketClient(t)
	c.Timeout = 1500 * time.Millisecond
	resp, err := c.Get("http://mcpx/v1/events?kinds=call&since=0")
	if err != nil {
		t.Fatal(err)
	}
	defer func() { _ = resp.Body.Close() }()
	body, _ := io.ReadAll(resp.Body)
	return string(body)
}

// TestEventsHistoryBoundsTheReplay is the effect probe for events.history:
// the bus keeps that many events for reconnecting subscribers. At the
// default a from-the-start replay is whole; kept to one event, the same
// replay starts past what was trimmed and says so with a gap notice.
func TestEventsHistoryBoundsTheReplay(t *testing.T) {
	calls := func(e *env) {
		for _, m := range []string{"a", "b", "c"} {
			e.run("call", "demo.echo", `{"message":"`+m+`"}`)
		}
	}

	whole := newEnv(t, oneServer)
	calls(whole)
	body := replayFromStart(t, whole)
	if strings.Contains(body, "event: gap") {
		t.Errorf("the default history should replay from the start without a gap:\n%s", body)
	}
	if n := strings.Count(body, "event: call.finished"); n < 3 {
		t.Errorf("the default history should replay all 3 calls, got %d:\n%s", n, body)
	}

	short := newEnv(t, oneServer)
	short.setenv("MCPX_EVENTS_HISTORY=1")
	calls(short)
	body = replayFromStart(t, short)
	if !strings.Contains(body, "event: gap") {
		t.Errorf("events.history=1 should trim the replay and report the gap:\n%s", body)
	}
	if n := strings.Count(body, "event: call.finished"); n > 1 {
		t.Errorf("events.history=1 should keep at most one event, replayed %d calls:\n%s", n, body)
	}
}

// TestEventsReconnectWaitsForTheConfiguredDelay drives the real client
// against a local endpoint that drops its first connection and serves an
// event on the retry. The accepted-connection timestamps are the oracle:
// events.reconnect must delay the second attempt without relying on a daemon
// restart or a network service.
func TestEventsReconnectWaitsForTheConfiguredDelay(t *testing.T) {
	const delay = 200 * time.Millisecond
	recordSettingProbes(t)
	t.Setenv("MCPX_EVENTS_RECONNECT", delay.String())
	ln, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { _ = ln.Close() })
	accepted := make(chan time.Time, 2)
	serveErr := make(chan error, 1)
	go func() {
		for attempt := 0; attempt < 2; attempt++ {
			conn, err := ln.Accept()
			if err != nil {
				serveErr <- err
				return
			}
			accepted <- time.Now()
			// Drain the request head before writing. A close() with unread
			// request bytes in the receive buffer sends a reset instead of a
			// FIN, and the peer may drop response bytes it has not yet read:
			// on macOS that loses the first event and the stream sees seq 2
			// first, which this test then reports as "unexpected event kind".
			if _, err := http.ReadRequest(bufio.NewReader(conn)); err != nil {
				serveErr <- err
				_ = conn.Close()
				return
			}
			body := fmt.Sprintf("id: %d\ndata: {\"seq\":%d,\"kind\":\"call.finished\"}\n\n", attempt+1, attempt+1)
			_, err = fmt.Fprintf(conn, "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: %d\r\nConnection: close\r\n\r\n%s", len(body), body)
			_ = conn.Close()
			if err != nil {
				serveErr <- err
			}
		}
	}()

	paths := daemon.PathsAt(t.TempDir(), t.TempDir())
	configPath := filepath.Join(t.TempDir(), ".mcpx.json")
	if err := os.WriteFile(configPath, []byte("{}"), 0o600); err != nil {
		t.Fatal(err)
	}
	app := &cli.App{ConfigPath: configPath}
	// The CLI installs its App as the plumbing source before constructing
	// clients. Use the same settings path here so NewClientAt resolves the
	// user-supplied environment value instead of receiving an injected set.
	cli.SetPlumbingSource(app)
	t.Cleanup(func() { cli.SetPlumbingSource(nil) })
	if got := app.Settings().Duration("events.reconnect"); got != delay {
		t.Fatalf("resolved events.reconnect = %s, want environment value %s", got, delay)
	}
	c := cli.NewClientAt(paths, configPath, "http://"+ln.Addr().String())
	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()
	eventsSeen := make(chan events.Event, 1)
	streamDone := make(chan error, 1)
	go func() {
		streamDone <- c.Stream(ctx, events.Filter{}, func(event events.Event) {
			eventsSeen <- event
			if event.Seq == 2 {
				cancel()
			}
		})
	}()

	var first, second time.Time
	select {
	case first = <-accepted:
	case err := <-serveErr:
		t.Fatal(err)
	case <-time.After(3 * time.Second):
		t.Fatal("client did not make its initial event-stream connection")
	}
	select {
	case second = <-accepted:
	case err := <-serveErr:
		t.Fatal(err)
	case <-time.After(3 * time.Second):
		t.Fatal("client did not retry the dropped event-stream connection")
	}
	if elapsed := second.Sub(first); elapsed < delay*3/4 || elapsed >= time.Second {
		t.Fatalf("events.reconnect=%s should delay a dropped stream (and differ from the 2s default); second connection arrived after %s", delay, elapsed)
	}
	select {
	case event := <-eventsSeen:
		if event.Kind != events.CallFinished || event.Seq != 1 {
			t.Fatalf("first stream delivered unexpected event: %+v", event)
		}
	case <-time.After(3 * time.Second):
		t.Fatal("client did not process the event delivered on the first stream")
	}
	select {
	case event := <-eventsSeen:
		if event.Kind != events.CallFinished || event.Seq != 2 {
			t.Fatalf("reconnected stream delivered unexpected event: %+v", event)
		}
	case <-time.After(3 * time.Second):
		t.Fatal("client did not process the event delivered on reconnect")
	}
	select {
	case err := <-streamDone:
		if !errors.Is(err, context.Canceled) {
			t.Fatalf("event stream stopped with %v, want context.Canceled", err)
		}
	case <-time.After(3 * time.Second):
		t.Fatal("event stream did not stop after cancellation")
	}
}

// churnedSchemaServer starts a daemon over the schema fixture and flips
// demo.create_issue's schema three times, so the catalog history has three
// recorded changes to keep or trim. env pairs reach the daemon at start.
func churnedSchemaServer(t *testing.T, env ...string) *env {
	t.Helper()
	schema := filepath.Join(t.TempDir(), "schema")
	mustWrite(t, schema, "v1")
	e := newEnv(t, strings.ReplaceAll(schemaServer, "VERSIONFILE", schema))
	e.setenv(env...)
	e.run("refresh")
	for _, v := range []string{"v2", "v1", "v2"} {
		mustWrite(t, schema, v)
		e.run("refresh")
	}
	return e
}

// recordedChanges counts the schema changes the daemon keeps for one tool.
func recordedChanges(t *testing.T, e *env, tool string) int {
	t.Helper()
	resp, err := e.socketClient(t).Get("http://mcpx/v1/catalog/history?tool=" + tool)
	if err != nil {
		t.Fatal(err)
	}
	defer func() { _ = resp.Body.Close() }()
	var doc struct {
		Changes map[string][]json.RawMessage `json:"changes"`
	}
	if err := json.NewDecoder(resp.Body).Decode(&doc); err != nil {
		t.Fatal(err)
	}
	return len(doc.Changes[tool])
}

// TestDiagnoseHistoryCapsRecordedSchemaChanges is the effect probe for
// diagnose.history: how many schema changes the daemon remembers per tool.
// Three flips are all kept at the default and trimmed to one at 1.
func TestDiagnoseHistoryCapsRecordedSchemaChanges(t *testing.T) {
	if n := recordedChanges(t, churnedSchemaServer(t), "demo.create_issue"); n != 3 {
		t.Errorf("the default diagnose.history should keep all 3 changes, kept %d", n)
	}
	if n := recordedChanges(t, churnedSchemaServer(t, "MCPX_DIAGNOSE_HISTORY=1"), "demo.create_issue"); n != 1 {
		t.Errorf("diagnose.history=1 should keep one change, kept %d", n)
	}
}

// TestDiagnosePreflightStopsAMismatchedScript is the effect probe for
// diagnose.preflight: a script calling a tool whose schema moved under it is
// stopped before it runs by default, and runs (catching the tool's own
// error) when the environment turns preflight off.
func TestDiagnosePreflightStopsAMismatchedScript(t *testing.T) {
	e := churnedSchemaServer(t)
	const script = `try { await demo.create_issue({ title: "x" }); } catch (err) {}
console.log("script ran");`

	out, err := e.try("exec", script)
	if err == nil || strings.Contains(out, "script ran") || !strings.Contains(out, "does not match the tools it calls") {
		t.Errorf("default diagnose.preflight should stop the script before it runs (err %v):\n%s", err, out)
	}

	e.setenv("MCPX_DIAGNOSE_PREFLIGHT=false")
	out, err = e.try("exec", script)
	if err != nil || !strings.Contains(out, "script ran") {
		t.Errorf("diagnose.preflight=false should let the script run (err %v):\n%s", err, out)
	}
}

// reactiveTools opens one `mcpx serve --mode reactive` stdio session,
// optionally asks request_tools for query first, and returns the names the
// session then lists. Frames go one at a time and each waits for its own
// answer: the server handles requests concurrently, so a pipelined
// tools/list could be answered before the request_tools it follows.
func reactiveTools(t *testing.T, e *env, query string) map[string]bool {
	t.Helper()
	return servedTools(t, e, query, "--mode", "reactive")
}

// servedTools is reactiveTools for any `mcpx serve` arguments.
func servedTools(t *testing.T, e *env, query string, args ...string) map[string]bool {
	return servedToolsAfterRequests(t, e, []string{query}, 5, 0, args...)
}

// servedToolsAfterRequests activates each query in one reactive session,
// optionally pausing before the final tools/list request.
func servedToolsAfterRequests(t *testing.T, e *env, queries []string, limit int, pause time.Duration, args ...string) map[string]bool {
	t.Helper()
	cmd := exec.Command(e.mcpx, append([]string{"serve"}, args...)...)
	cmd.Dir = e.dir
	cmd.Env = e.envVars
	stdin, err := cmd.StdinPipe()
	if err != nil {
		t.Fatal(err)
	}
	stdout, err := cmd.StdoutPipe()
	if err != nil {
		t.Fatal(err)
	}
	if err := cmd.Start(); err != nil {
		t.Fatal(err)
	}
	defer func() { _ = cmd.Process.Kill(); _ = cmd.Wait() }()
	dec := json.NewDecoder(bufio.NewReaderSize(stdout, 1<<20))
	id := 0
	rpc := func(method string, params any) json.RawMessage {
		t.Helper()
		id++
		b, _ := json.Marshal(map[string]any{"jsonrpc": "2.0", "id": id, "method": method, "params": params})
		if _, err := stdin.Write(append(b, '\n')); err != nil {
			t.Fatal(err)
		}
		for {
			var f struct {
				ID     *int            `json:"id"`
				Result json.RawMessage `json:"result"`
				Error  json.RawMessage `json:"error"`
			}
			if err := dec.Decode(&f); err != nil {
				t.Fatalf("read %s: %v", method, err)
			}
			if f.ID != nil && *f.ID == id {
				if len(f.Error) > 0 {
					t.Fatalf("%s failed: %s", method, f.Error)
				}
				return f.Result
			}
		}
	}
	rpc("initialize", map[string]any{})
	for _, query := range queries {
		if query == "" {
			continue
		}
		rpc("tools/call", map[string]any{"name": "request_tools",
			"arguments": map[string]any{"query": query, "limit": limit}})
	}
	if pause > 0 {
		time.Sleep(pause)
	}
	var list struct {
		Tools []struct {
			Name string `json:"name"`
		} `json:"tools"`
	}
	if err := json.Unmarshal(rpc("tools/list", map[string]any{}), &list); err != nil {
		t.Fatal(err)
	}
	names := map[string]bool{}
	for _, tool := range list.Tools {
		names[tool.Name] = true
	}
	return names
}

// TestRetentionStickyWindowExpiresReactiveTools is the effect probe for
// retention.stickyWindow: a tool requested within the configured window stays
// listed, while an older requested tool disappears after the window expires.
func TestRetentionStickyWindowExpiresReactiveTools(t *testing.T) {
	const pause = 50 * time.Millisecond
	retained := func(names map[string]bool) int {
		n := 0
		for name := range names {
			if name != "request_tools" && name != "batch_call" {
				n++
			}
		}
		return n
	}

	serve := func(window string) map[string]bool {
		e := newEnv(t, oneServer)
		e.run("ls")
		e.setenv("MCPX_RETENTION_STICKY_WINDOW=" + window)
		return servedToolsAfterRequests(t, e, []string{"echo"}, 5, pause, "--mode", "reactive")
	}

	withinWindow := serve("1h")
	if n := retained(withinWindow); n != 5 {
		t.Errorf("a request inside retention.stickyWindow should keep all 5 requested tools listed; kept %d: %v", n, withinWindow)
	}

	expired := serve("20ms")
	if n := retained(expired); n != 0 {
		t.Errorf("an expired request should no longer list the requested tools; kept %d: %v", n, expired)
	}
}

// TestCatalogPinnedToolsAlwaysListInReactiveMode is the effect probe for
// catalog.pinnedTools: a fresh reactive session lists only its meta tools,
// and a tool pinned from the environment is listed from the start.
func TestCatalogPinnedToolsAlwaysListInReactiveMode(t *testing.T) {
	e := newEnv(t, oneServer)
	e.run("ls")
	if got := reactiveTools(t, e, ""); got["mcpx_status"] {
		t.Errorf("a fresh reactive session should not list mcpx_status unpinned: %v", got)
	}
	e.setenv("MCPX_CATALOG_PINNED_TOOLS=mcpx_status")
	if got := reactiveTools(t, e, ""); !got["mcpx_status"] {
		t.Errorf("catalog.pinnedTools=mcpx_status should list it from the start: %v", got)
	}
}

// TestRetentionMaxToolsCapsTheReactiveWorkingSet is the effect probe for
// retention.maxTools: request_tools activates five tools, all of which the
// default working set keeps; capped at two, only two stay listed.
func TestRetentionMaxToolsCapsTheReactiveWorkingSet(t *testing.T) {
	retained := func(names map[string]bool) int {
		n := 0
		for name := range names {
			if name != "request_tools" && name != "batch_call" {
				n++
			}
		}
		return n
	}
	e := newEnv(t, oneServer)
	e.run("ls")
	if n := retained(reactiveTools(t, e, "echo")); n != 5 {
		t.Errorf("the default retention.maxTools should keep all 5 activated tools, kept %d", n)
	}
	e.setenv("MCPX_RETENTION_MAX_TOOLS=2")
	if n := retained(reactiveTools(t, e, "echo")); n != 2 {
		t.Errorf("retention.maxTools=2 should keep 2 activated tools, kept %d", n)
	}
}

// shownSignatures counts the tool signatures `mcpx catalog` printed.
func shownSignatures(out string) int { return strings.Count(out, "  - demo.") }

// TestCatalogBudgetFitsTheSignatures is the effect probe for catalog.budget:
// the default budget shows every demo signature, and a 20-token budget from
// the environment shows none of them while still naming the namespace.
func TestCatalogBudgetFitsTheSignatures(t *testing.T) {
	e := newEnv(t, oneServer)
	if out := e.run("catalog"); shownSignatures(out) != 9 {
		t.Errorf("the default catalog.budget should show all 9 demo signatures:\n%s", out)
	}
	e.setenv("MCPX_CATALOG_BUDGET=20")
	if out := e.run("catalog"); shownSignatures(out) != 0 || !strings.Contains(out, "none shown") {
		t.Errorf("catalog.budget=20 should show no signatures:\n%s", out)
	}
}

// TestCatalogBiasPicksWhatATightBudgetShows is the effect probe for
// catalog.bias: at a budget that fits two signatures, demo.wipe is not
// among them unbiased and is once the environment biases toward it.
func TestCatalogBiasPicksWhatATightBudgetShows(t *testing.T) {
	e := newEnv(t, oneServer)
	e.setenv("MCPX_CATALOG_BUDGET=60")
	if out := e.run("catalog"); strings.Contains(out, "demo.wipe(") {
		t.Errorf("unbiased, a 60-token catalog should not reach demo.wipe:\n%s", out)
	}
	e.setenv("MCPX_CATALOG_BIAS=wipe")
	if out := e.run("catalog"); !strings.Contains(out, "demo.wipe(") {
		t.Errorf("catalog.bias=wipe should show demo.wipe within the same budget:\n%s", out)
	}
}

// TestPresetsDefinesNamedFlagBundles is the effect probe for presets: with
// nothing defined, --preset j is refused as unknown; once the environment
// defines j as ["--json"], the same command applies it and prints JSON.
func TestPresetsDefinesNamedFlagBundles(t *testing.T) {
	e := newEnv(t, oneServer)
	out, err := e.try("status", "--preset", "j")
	if err == nil || !strings.Contains(out, `no preset "j"`) {
		t.Errorf("with no presets defined, --preset j should be refused (err %v):\n%s", err, out)
	}
	e.setenv(`MCPX_PRESETS={"j":["--json"]}`)
	out, err = e.try("status", "--preset", "j")
	if err != nil || !json.Valid([]byte(strings.TrimSpace(out))) {
		t.Errorf("presets defining j as --json should make status print JSON (err %v):\n%s", err, out)
	}
}

// TestPresetSelectsADefinedBundle is the effect probe for preset: with j
// defined as ["--json"], plain `mcpx status` prints text, and naming j in
// MCPX_PRESET applies it without the flag.
func TestPresetSelectsADefinedBundle(t *testing.T) {
	e := newEnv(t, oneServer)
	e.setenv(`MCPX_PRESETS={"j":["--json"]}`)
	if out := e.run("status"); json.Valid([]byte(strings.TrimSpace(out))) {
		t.Errorf("with no preset selected, status should print text:\n%s", out)
	}
	e.setenv("MCPX_PRESET=j")
	if out := e.run("status"); !json.Valid([]byte(strings.TrimSpace(out))) {
		t.Errorf("preset=j should make status print JSON:\n%s", out)
	}
}

// TestServeModeSelectsTheServedSurface is the effect probe for serve.mode:
// plain `mcpx serve` is code mode and lists no request_tools; with
// MCPX_SERVE_MODE=reactive the same command serves the reactive surface.
func TestServeModeSelectsTheServedSurface(t *testing.T) {
	e := newEnv(t, oneServer)
	e.run("ls")
	if got := servedTools(t, e, ""); got["request_tools"] || !got["mcpx_exec"] {
		t.Errorf("the default serve.mode should serve code mode: %v", got)
	}
	e.setenv("MCPX_SERVE_MODE=reactive")
	if got := servedTools(t, e, ""); !got["request_tools"] || got["mcpx_exec"] {
		t.Errorf("serve.mode=reactive should serve the reactive surface: %v", got)
	}
}

// artifactScript stores one four-byte artifact and prints "done".
const artifactScript = `await artifact("n.txt", new Uint8Array([104, 105, 104, 105]));
console.log("done");`

// runArtifactScript runs artifactScript with --json in a fresh env carrying
// env, and returns what it printed and whether it failed.
func runArtifactScript(t *testing.T, env ...string) (string, error) {
	t.Helper()
	e := newEnv(t, oneServer)
	e.setenv(env...)
	return e.try("exec", "--json", artifactScript)
}

// TestArtifactsEnabledGatesTheStore is the effect probe for
// artifacts.enabled: a script's artifact is stored at the default and
// refused once the environment disables the store.
func TestArtifactsEnabledGatesTheStore(t *testing.T) {
	if out, err := runArtifactScript(t); err != nil || !strings.Contains(out, `"name": "n.txt"`) {
		t.Errorf("artifacts are on by default (err %v):\n%s", err, out)
	}
	if out, err := runArtifactScript(t, "MCPX_ARTIFACTS_ENABLED=false"); err == nil || !strings.Contains(out, "artifacts are disabled") {
		t.Errorf("artifacts.enabled=false should refuse the artifact (err %v):\n%s", err, out)
	}
}

// TestArtifactsMaxBytesRefusesALargeArtifact is the effect probe for
// artifacts.maxBytes: four bytes fit the default and not a 2-byte limit.
func TestArtifactsMaxBytesRefusesALargeArtifact(t *testing.T) {
	if out, err := runArtifactScript(t); err != nil {
		t.Errorf("four bytes should fit the default artifacts.maxBytes (err %v):\n%s", err, out)
	}
	if out, err := runArtifactScript(t, "MCPX_ARTIFACTS_MAX_BYTES=2"); err == nil || !strings.Contains(out, "artifacts.maxBytes") {
		t.Errorf("artifacts.maxBytes=2 should refuse four bytes (err %v):\n%s", err, out)
	}
}

// TestArtifactsQuotaBoundsTheStore is the effect probe for artifacts.quota:
// four bytes fit the default quota and not a 2-byte one.
func TestArtifactsQuotaBoundsTheStore(t *testing.T) {
	if out, err := runArtifactScript(t); err != nil {
		t.Errorf("four bytes should fit the default artifacts.quota (err %v):\n%s", err, out)
	}
	if out, err := runArtifactScript(t, "MCPX_ARTIFACTS_QUOTA=2"); err == nil || !strings.Contains(out, "artifacts.quota") {
		t.Errorf("artifacts.quota=2 should refuse four bytes (err %v):\n%s", err, out)
	}
}

// TestArtifactsDeliveryInlinesTheBody is the effect probe for
// artifacts.delivery: the default hands back a reference only, and inline
// carries the base64 body in the result.
func TestArtifactsDeliveryInlinesTheBody(t *testing.T) {
	const body = `"data": "aGloaQ=="`
	if out, err := runArtifactScript(t); err != nil || strings.Contains(out, body) {
		t.Errorf("the default delivery should not inline the body (err %v):\n%s", err, out)
	}
	if out, err := runArtifactScript(t, "MCPX_ARTIFACTS_DELIVERY=inline"); err != nil || !strings.Contains(out, body) {
		t.Errorf("artifacts.delivery=inline should carry the body (err %v):\n%s", err, out)
	}
}

// TestArtifactsInlineMaxBytesBoundsAnInlinedBody is the effect probe for
// artifacts.inlineMaxBytes: with inline delivery, four bytes are inlined
// under the default bound and left as a reference under a 1-byte bound.
func TestArtifactsInlineMaxBytesBoundsAnInlinedBody(t *testing.T) {
	const body = `"data": "aGloaQ=="`
	if out, err := runArtifactScript(t, "MCPX_ARTIFACTS_DELIVERY=inline"); err != nil || !strings.Contains(out, body) {
		t.Errorf("four bytes should inline under the default bound (err %v):\n%s", err, out)
	}
	out, err := runArtifactScript(t, "MCPX_ARTIFACTS_DELIVERY=inline", "MCPX_ARTIFACTS_INLINE_MAX_BYTES=1")
	if err != nil || strings.Contains(out, body) || !strings.Contains(out, `"name": "n.txt"`) {
		t.Errorf("artifacts.inlineMaxBytes=1 should keep the body out of the result (err %v):\n%s", err, out)
	}
}

// TestArtifactsDirReceivesTheRunsArtifacts is the effect probe for
// artifacts.dir: by default nothing lands in the directory, and naming it
// in the environment copies the run's artifact there.
func TestArtifactsDirReceivesTheRunsArtifacts(t *testing.T) {
	dir := filepath.Join(t.TempDir(), "out")
	e := newEnv(t, oneServer)
	e.run("exec", artifactScript)
	if _, err := os.Stat(filepath.Join(dir, "n.txt")); err == nil {
		t.Fatal("with no artifacts.dir, nothing should be written to the directory")
	}
	e.setenv("MCPX_ARTIFACTS_DIR=" + dir)
	e.run("exec", artifactScript)
	if b, err := os.ReadFile(filepath.Join(dir, "n.txt")); err != nil || string(b) != "hihi" {
		t.Errorf("artifacts.dir should receive n.txt with its body, got %q (%v)", b, err)
	}
}

// loggedRun starts a daemon with env and its durable log in a fresh
// directory, then `mcpx run`s a script that logs at debug and at info. It
// returns the run's stderr, the log directory, and the daemon.start record.
func loggedRun(t *testing.T, env ...string) (stderr, dir, start string) {
	t.Helper()
	e := newEnv(t, oneServer)
	dir = filepath.Join(t.TempDir(), "logs")
	e.setenv(append([]string{"MCPX_LOGGING_DIR=" + dir}, env...)...)
	e.run("ls")
	script := filepath.Join(e.dir, "logged.ts")
	mustWrite(t, script, `log.debug("debug-line"); log.info("info-a"); log.info("info-b"); log.info("info-c");`)
	_, stderr, err := e.split("run", script)
	if err != nil {
		t.Fatalf("run: %v\n%s", err, stderr)
	}
	for _, f := range logFiles(t, dir) {
		b, _ := os.ReadFile(filepath.Join(dir, f))
		for _, line := range strings.Split(string(b), "\n") {
			if strings.Contains(line, `"event":"daemon.start"`) {
				start = line
			}
		}
	}
	return stderr, dir, start
}

// logFiles lists the JSONL files in a log directory.
func logFiles(t *testing.T, dir string) []string {
	t.Helper()
	ents, _ := os.ReadDir(dir)
	var out []string
	for _, ent := range ents {
		if strings.HasSuffix(ent.Name(), ".jsonl") {
			out = append(out, ent.Name())
		}
	}
	return out
}

// rotatedLogFiles excludes the per-day active files. A run that crosses
// midnight can legitimately leave one active file for each day, while
// logging.keep applies only to timestamp-suffixed rotations.
func rotatedLogFiles(t *testing.T, dir string) []string {
	t.Helper()
	var out []string
	for _, name := range logFiles(t, dir) {
		stem := strings.TrimSuffix(name, ".jsonl")
		if len(stem) > len("mcpx-2006-01-02") {
			out = append(out, name)
		}
	}
	return out
}

func TestRotatedLogFilesIgnoresDailyActiveFiles(t *testing.T) {
	dir := t.TempDir()
	for _, name := range []string{
		"mcpx-2026-11-21.jsonl",
		"mcpx-2026-11-22.jsonl",
		"mcpx-2026-11-22-031602.jsonl",
	} {
		if err := os.WriteFile(filepath.Join(dir, name), nil, 0o600); err != nil {
			t.Fatal(err)
		}
	}
	got := rotatedLogFiles(t, dir)
	if len(got) != 1 || got[0] != "mcpx-2026-11-22-031602.jsonl" {
		t.Fatalf("rotated log list = %v, want only the timestamp-suffixed rotation", got)
	}
}

// TestLoggingLevelShowsDebugRecords is the effect probe for logging.level:
// a script's debug record is hidden at the default level and shown at debug.
func TestLoggingLevelShowsDebugRecords(t *testing.T) {
	if se, _, _ := loggedRun(t); strings.Contains(se, "debug-line") || !strings.Contains(se, "info-a") {
		t.Errorf("the default logging.level should show info and hide debug:\n%s", se)
	}
	if se, _, _ := loggedRun(t, "MCPX_LOGGING_LEVEL=debug"); !strings.Contains(se, "debug-line") {
		t.Errorf("logging.level=debug should show the debug record:\n%s", se)
	}
}

// TestLoggingFormatRendersJSON is the effect probe for logging.format: the
// default renders a script's record as text, and json renders it as JSON.
func TestLoggingFormatRendersJSON(t *testing.T) {
	const asJSON = `"msg":"info-a"`
	if se, _, _ := loggedRun(t); strings.Contains(se, asJSON) || !strings.Contains(se, "info-a") {
		t.Errorf("the default logging.format should render text:\n%s", se)
	}
	if se, _, _ := loggedRun(t, "MCPX_LOGGING_FORMAT=json"); !strings.Contains(se, asJSON) {
		t.Errorf("logging.format=json should render the record as JSON:\n%s", se)
	}
}

// TestLoggingSourceAddsTheCallSite is the effect probe for logging.source:
// a script's record carries no source location at the default, and does
// at logging.source=debug.
func TestLoggingSourceAddsTheCallSite(t *testing.T) {
	if se, _, _ := loggedRun(t); strings.Contains(se, "source.file=") {
		t.Errorf("the default logging.source should add no call site:\n%s", se)
	}
	if se, _, _ := loggedRun(t, "MCPX_LOGGING_SOURCE=debug"); !strings.Contains(se, "source.file=") {
		t.Errorf("logging.source=debug should add the call site:\n%s", se)
	}
}

// TestLoggingDirHoldsTheDurableLog is the effect probe for logging.dir: the
// durable log lands in the directory the environment names, and nowhere
// in it before then.
func TestLoggingDirHoldsTheDurableLog(t *testing.T) {
	dir := filepath.Join(t.TempDir(), "logs")
	e := newEnv(t, oneServer)
	e.run("ls")
	if got := logFiles(t, dir); len(got) != 0 {
		t.Fatalf("with no logging.dir, nothing should be logged to %s: %v", dir, got)
	}
	e2 := newEnv(t, oneServer)
	e2.setenv("MCPX_LOGGING_DIR=" + dir)
	e2.run("ls")
	if got := logFiles(t, dir); len(got) == 0 {
		t.Errorf("logging.dir should hold the daemon's durable log")
	}
}

// TestLoggingIncludeChoosesTheAmbientBlocks is the effect probe for
// logging.include: daemon.start carries host but not network facts at the
// default, none at "none", and available network facts too at "all".
func TestLoggingIncludeChoosesTheAmbientBlocks(t *testing.T) {
	if _, _, start := loggedRun(t); !strings.Contains(start, `"host.name"`) || strings.Contains(start, `"net.`) {
		t.Errorf("the default logging.include should carry host and not network facts:\n%s", start)
	}
	if _, _, start := loggedRun(t, "MCPX_LOGGING_INCLUDE=none"); start == "" || strings.Contains(start, `"host.name"`) {
		t.Errorf("logging.include=none should drop the host block:\n%s", start)
	}
	_, _, start := loggedRun(t, "MCPX_LOGGING_INCLUDE=all")
	if start == "" {
		t.Errorf("logging.include=all should still log daemon.start")
	} else if hasIPv4NetworkInterface() && !strings.Contains(start, `"net.`) {
		t.Errorf("logging.include=all should add the network block:\n%s", start)
	}
}

func hasIPv4NetworkInterface() bool {
	ifaces, err := net.Interfaces()
	if err != nil {
		return false
	}
	for _, iface := range ifaces {
		if iface.Flags&net.FlagUp == 0 || iface.Flags&net.FlagLoopback != 0 {
			continue
		}
		addrs, err := iface.Addrs()
		if err != nil {
			continue
		}
		for _, addr := range addrs {
			ipnet, ok := addr.(*net.IPNet)
			if ok && ipnet.IP.To4() != nil {
				return true
			}
		}
	}
	return false
}

// TestLoggingMaxLinesRotatesTheFile is the effect probe for
// logging.maxLines: the default keeps one file, and one line per file
// rotates into several.
func TestLoggingMaxLinesRotatesTheFile(t *testing.T) {
	if _, dir, _ := loggedRun(t); len(rotatedLogFiles(t, dir)) != 0 {
		t.Errorf("the default logging.maxLines should not rotate: %v", rotatedLogFiles(t, dir))
	}
	if _, dir, _ := loggedRun(t, "MCPX_LOGGING_MAX_LINES=1"); len(rotatedLogFiles(t, dir)) < 3 {
		t.Errorf("logging.maxLines=1 should rotate into several files: %v", rotatedLogFiles(t, dir))
	}
}

// TestLoggingMaxBytesRotatesTheFile is the effect probe for
// logging.maxBytes: the default keeps one file, and a 10-byte bound
// rotates into several.
func TestLoggingMaxBytesRotatesTheFile(t *testing.T) {
	if _, dir, _ := loggedRun(t); len(rotatedLogFiles(t, dir)) != 0 {
		t.Errorf("the default logging.maxBytes should not rotate: %v", rotatedLogFiles(t, dir))
	}
	if _, dir, _ := loggedRun(t, "MCPX_LOGGING_MAX_BYTES=10"); len(rotatedLogFiles(t, dir)) < 3 {
		t.Errorf("logging.maxBytes=10 should rotate into several files: %v", rotatedLogFiles(t, dir))
	}
}

// TestLoggingKeepPrunesRotatedFiles is the effect probe for logging.keep:
// rotating every line keeps many rotated files at the default, and at most
// one besides the live file at keep=1.
func TestLoggingKeepPrunesRotatedFiles(t *testing.T) {
	if _, dir, _ := loggedRun(t, "MCPX_LOGGING_MAX_LINES=1"); len(rotatedLogFiles(t, dir)) < 3 {
		t.Errorf("the default logging.keep should keep several rotated files: %v", rotatedLogFiles(t, dir))
	}
	if _, dir, _ := loggedRun(t, "MCPX_LOGGING_MAX_LINES=1", "MCPX_LOGGING_KEEP=1"); len(rotatedLogFiles(t, dir)) != 1 {
		t.Errorf("logging.keep=1 should prune to one rotated file: %v", rotatedLogFiles(t, dir))
	}
}

// touchHook declares one call.finished hook that waits a second and then
// creates mark, so whether mark appears says whether the hook ran to the end.
func touchHook(mark string) string {
	return `MCPX_HOOKS=[{"on":"call.finished","run":["sh","-c","sleep 1; touch ` + mark + `"]}]`
}

// appears polls for path for up to wait.
func appears(path string, wait time.Duration) bool {
	for deadline := time.Now().Add(wait); time.Now().Before(deadline); {
		if _, err := os.Stat(path); err == nil {
			return true
		}
		time.Sleep(100 * time.Millisecond)
	}
	return false
}

// TestHooksRunOnTheirEvent is the effect probe for hooks: with no hooks a
// call leaves nothing behind, and a call.finished hook declared in the
// environment runs after the call.
func TestHooksRunOnTheirEvent(t *testing.T) {
	mark := filepath.Join(t.TempDir(), "mark")
	e := newEnv(t, oneServer)
	e.run("call", "demo.echo", `{"message":"x"}`)
	if appears(mark, 2*time.Second) {
		t.Fatal("with no hooks declared, nothing should create the mark")
	}
	e2 := newEnv(t, oneServer)
	e2.setenv(touchHook(mark))
	e2.run("call", "demo.echo", `{"message":"x"}`)
	if !appears(mark, 10*time.Second) {
		t.Error("a declared call.finished hook should run after the call")
	}
}

// TestHooksTimeoutStopsASlowHook is the effect probe for hooks.timeout: a
// hook that takes a second finishes under the default timeout, and is
// stopped, with a hook.failed event, under a 200ms one.
func TestHooksTimeoutStopsASlowHook(t *testing.T) {
	mark := filepath.Join(t.TempDir(), "mark")
	e := newEnv(t, oneServer)
	e.setenv(touchHook(mark))
	e.run("call", "demo.echo", `{"message":"x"}`)
	if !appears(mark, 10*time.Second) {
		t.Fatal("a one-second hook should finish under the default hooks.timeout")
	}

	slow := filepath.Join(t.TempDir(), "mark")
	e2 := newEnv(t, oneServer)
	e2.setenv(touchHook(slow), "MCPX_HOOKS_TIMEOUT=200ms")
	e2.run("call", "demo.echo", `{"message":"x"}`)
	c := *e2.socketClient(t)
	c.Timeout = 5 * time.Second
	resp, err := c.Get("http://mcpx/v1/events?kinds=hook&since=0")
	if err != nil {
		t.Fatal(err)
	}
	defer func() { _ = resp.Body.Close() }()
	buf := make([]byte, 8192)
	n, _ := io.ReadAtLeast(resp.Body, buf, 64)
	for !strings.Contains(string(buf[:n]), "timed out") && n < len(buf) {
		m, rerr := resp.Body.Read(buf[n:])
		n += m
		if rerr != nil {
			break
		}
	}
	if !strings.Contains(string(buf[:n]), "hook timed out after 200ms") {
		t.Errorf("hooks.timeout=200ms should fail the hook as timed out:\n%s", buf[:n])
	}
	if appears(slow, 2*time.Second) {
		t.Error("a hook stopped by hooks.timeout should not finish its work")
	}
}

// dashboard fetches /dashboard from a daemon started with env.
func dashboard(t *testing.T, env ...string) (int, string) {
	t.Helper()
	e := newEnv(t, oneServer)
	e.setenv(env...)
	e.run("ls")
	resp, err := e.socketClient(t).Get("http://mcpx/dashboard")
	if err != nil {
		t.Fatal(err)
	}
	defer func() { _ = resp.Body.Close() }()
	b, _ := io.ReadAll(resp.Body)
	return resp.StatusCode, string(b)
}

// TestDaemonUITurnsTheDashboardOff is the effect probe for daemon.ui: the
// loopback daemon serves its dashboard by default and 404s it at false.
func TestDaemonUITurnsTheDashboardOff(t *testing.T) {
	if code, _ := dashboard(t); code != http.StatusOK {
		t.Errorf("the default daemon.ui should serve the dashboard, got %d", code)
	}
	if code, _ := dashboard(t, "MCPX_DAEMON_UI=false"); code != http.StatusNotFound {
		t.Errorf("daemon.ui=false should 404 the dashboard, got %d", code)
	}
}

// TestDaemonUIDirOverridesTheDashboard is the effect probe for
// daemon.uiDir: the embedded page by default, and the named directory's
// index.html once the environment sets it.
func TestDaemonUIDirOverridesTheDashboard(t *testing.T) {
	ui := t.TempDir()
	mustWrite(t, filepath.Join(ui, "index.html"), "custom-ui-page")
	if _, body := dashboard(t); strings.Contains(body, "custom-ui-page") {
		t.Errorf("the default daemon.uiDir should serve the embedded page")
	}
	if code, body := dashboard(t, "MCPX_DAEMON_UI_DIR="+ui); code != http.StatusOK || body != "custom-ui-page" {
		t.Errorf("daemon.uiDir should serve its own index.html, got %d %q", code, body)
	}
}

// phaseRun `mcpx run`s a script that succeeds, or throws when fail is set,
// with env, and returns everything it printed.
func phaseRun(t *testing.T, fail bool, env ...string) string {
	t.Helper()
	e := newEnv(t, oneServer)
	e.setenv(env...)
	script := filepath.Join(e.dir, "phases.ts")
	body := "export default function () { return \"done\" }\n"
	if fail {
		body = "throw new Error(\"script-boom\")\n"
	}
	mustWrite(t, script, body)
	out, _ := e.try("run", script)
	return out
}

// TestScriptPhasesRunFromTheEnvironment is the effect probe for
// script.before, script.onSuccess, script.onError and script.suffix: none
// of their markers print by default, and each prints on the path it
// belongs to once the environment sets it.
func TestScriptPhasesRunFromTheEnvironment(t *testing.T) {
	phases := []string{
		`MCPX_SCRIPT_BEFORE=console.info("phase-before")`,
		`MCPX_SCRIPT_ON_SUCCESS=console.info("phase-success")`,
		`MCPX_SCRIPT_ON_ERROR=console.info("phase-error")`,
		`MCPX_SCRIPT_SUFFIX=console.info("phase-suffix")`,
	}
	for _, fail := range []bool{false, true} {
		if out := phaseRun(t, fail); strings.Contains(out, "phase-") {
			t.Errorf("no phase should run by default (fail=%v):\n%s", fail, out)
		}
	}
	ok := phaseRun(t, false, phases...)
	for _, want := range []string{"phase-before", "phase-success", "phase-suffix"} {
		if !strings.Contains(ok, want) {
			t.Errorf("a succeeding script should run %s:\n%s", want, ok)
		}
	}
	if strings.Contains(ok, "phase-error") {
		t.Errorf("a succeeding script should not run script.onError:\n%s", ok)
	}
	bad := phaseRun(t, true, phases...)
	for _, want := range []string{"phase-before", "phase-error", "phase-suffix", "script-boom"} {
		if !strings.Contains(bad, want) {
			t.Errorf("a throwing script should show %s:\n%s", want, bad)
		}
	}
	if strings.Contains(bad, "phase-success") {
		t.Errorf("a throwing script should not run script.onSuccess:\n%s", bad)
	}
}

// runtimeOf runs a script with --json and returns the runtime it reports.
func runtimeOf(t *testing.T, e *env, args ...string) string {
	t.Helper()
	var doc struct {
		Runtime string `json:"runtime"`
	}
	out := e.run(append(append([]string{"exec", "--json"}, args...), `console.log("hi")`)...)
	if err := json.Unmarshal([]byte(jsonOf(t, out)), &doc); err != nil {
		t.Fatalf("%v\n%s", err, out)
	}
	return doc.Runtime
}

// firstDefaultRuntime returns the first installed runtime in the configured
// default order. The suite can run with only a subset of runtimes installed.
// The probes then switch to node, so a host where node is the default
// cannot show the setting changing anything, and the probe is skipped.
func firstDefaultRuntime(t *testing.T) string {
	t.Helper()
	for _, name := range []string{"deno", "bun", "node"} {
		if _, err := exec.LookPath(name); err == nil {
			if name == "node" {
				t.Skip("only node is installed, so switching to node cannot show an effect")
			}
			return name
		}
	}
	t.Skip("no supported script runtime is installed")
	return ""
}

// TestScriptRuntimeSelectsTheRuntime is the effect probe for
// script.runtime: scripts use the first available runtime in the configured
// default order (deno, bun, node), and node once the environment names it.
func TestScriptRuntimeSelectsTheRuntime(t *testing.T) {
	testsupport.RequireBinary(t, "node")
	e := newEnv(t, oneServer)
	want := firstDefaultRuntime(t)
	if rt := runtimeOf(t, e); rt != want {
		t.Errorf("the default script.runtime should pick %s, got %q", want, rt)
	}
	e.setenv("MCPX_SCRIPT_RUNTIME=node")
	if rt := runtimeOf(t, e); rt != "node" {
		t.Errorf("script.runtime=node should run on node, got %q", rt)
	}
}

// TestScriptRuntimeOrderRanksTheAutoPick is the effect probe for
// script.runtimeOrder: the automatic pick is deno by default and node once
// the environment ranks node first.
func TestScriptRuntimeOrderRanksTheAutoPick(t *testing.T) {
	testsupport.RequireBinary(t, "node")
	e := newEnv(t, oneServer)
	want := firstDefaultRuntime(t)
	if rt := runtimeOf(t, e); rt != want {
		t.Errorf("the default script.runtimeOrder should pick %s, got %q", want, rt)
	}
	e.setenv("MCPX_SCRIPT_RUNTIME_ORDER=node,bun")
	if rt := runtimeOf(t, e); rt != "node" {
		t.Errorf("script.runtimeOrder=node,bun should pick node, got %q", rt)
	}
}

// nodeWrite runs, on node, a script that tries to write a file and prints
// WROTE or DENIED.
func nodeWrite(t *testing.T, e *env, args ...string) string {
	t.Helper()
	target := filepath.Join(t.TempDir(), "w")
	probe := `import { writeFileSync } from "node:fs";
try { writeFileSync(` + jsonQuote(target) + `, "1"); console.log("WROTE"); } catch (e) { console.log("DENIED " + e.code); }`
	out, _ := e.try(append(append([]string{"exec", "--runtime", "node"}, args...), probe)...)
	return out
}

// TestScriptPermissionsSandboxesNode is the effect probe for
// script.permissions: node writes freely at the default, and the readnet
// profile named in the environment makes node refuse the write.
func TestScriptPermissionsSandboxesNode(t *testing.T) {
	testsupport.RequireBinary(t, "node")
	e := newEnv(t, oneServer)
	if out := nodeWrite(t, e); !strings.Contains(out, "WROTE") {
		t.Errorf("the default script.permissions should let node write:\n%s", out)
	}
	e.setenv("MCPX_SCRIPT_PERMISSIONS=readnet")
	if out := nodeWrite(t, e); !strings.Contains(out, "DENIED ERR_ACCESS_DENIED") {
		t.Errorf("script.permissions=readnet should make node refuse the write:\n%s", out)
	}
}

// TestScriptProfilesDefinesAProfile is the effect probe for script.profiles:
// --permissions ro names no profile by default, and once the environment
// defines ro with node's read-only flags, node refuses the write under it.
func TestScriptProfilesDefinesAProfile(t *testing.T) {
	testsupport.RequireBinary(t, "node")
	e := newEnv(t, oneServer)
	if out := nodeWrite(t, e, "--permissions", "ro"); strings.Contains(out, "WROTE") || strings.Contains(out, "DENIED") {
		t.Errorf("with no profile ro defined, the script should not run:\n%s", out)
	}
	e.setenv(`MCPX_SCRIPT_PROFILES={"ro":{"node":["--permission","--allow-fs-read=*"]}}`)
	if out := nodeWrite(t, e, "--permissions", "ro"); !strings.Contains(out, "DENIED ERR_ACCESS_DENIED") {
		t.Errorf("script.profiles defining ro should make node refuse the write:\n%s", out)
	}
}

// TestScriptRuntimesDeclaresARuntime is the effect probe for
// script.runtimes: --runtime engine is unknown by default, and runs on
// node once the environment declares engine as a node binary.
func TestScriptRuntimesDeclaresARuntime(t *testing.T) {
	node := testsupport.RequireBinary(t, "node")
	e := newEnv(t, oneServer)
	src := `console.log("kind=" + process.env.MCPX_RUNTIME)`
	if out, err := e.try("exec", "--runtime", "engine", src); err == nil || strings.Contains(out, "kind=node") {
		t.Errorf("with nothing declared, --runtime engine should fail: %v\n%s", err, out)
	}
	e.setenv(`MCPX_SCRIPT_RUNTIMES={"engine":{"kind":"node","bin":` + jsonQuote(node) + `}}`)
	if out, err := e.try("exec", "--runtime", "engine", src); err != nil || !strings.Contains(out, "kind=node") {
		t.Errorf("script.runtimes should declare engine as node: %v\n%s", err, out)
	}
}

// TestPathsAdaptersFindsADeclarationFile is the effect probe for
// paths.adapters: `mcpx adapter list` declares nothing by default, and
// lists the adapter in the file the environment points it at.
func TestPathsAdaptersFindsADeclarationFile(t *testing.T) {
	e := newEnv(t, oneServer)
	if out := e.run("adapter", "list"); !strings.Contains(out, "No adapters declared") {
		t.Errorf("by default no adapters should be declared:\n%s", out)
	}
	file := filepath.Join(t.TempDir(), "adapters.json")
	mustWrite(t, file, `{"adapters":[{"name":"probeadapter","command":"echo","tools":[{"name":"say","params":[{"name":"text"}]}]}]}`)
	e.setenv("MCPX_PATHS_ADAPTERS=" + file)
	if out := e.run("adapter", "list"); !strings.Contains(out, "probeadapter") {
		t.Errorf("paths.adapters should declare probeadapter:\n%s", out)
	}
}

// TestAdaptersAllowBareGatesABareDeclaration is the effect probe for
// adapters.allowBare (#11147): a bare declaration file fails to load
// while the setting is off (its default), with an error naming the
// setting, and lists once the environment turns the setting on.
func TestAdaptersAllowBareGatesABareDeclaration(t *testing.T) {
	e := newEnv(t, oneServer)
	file := filepath.Join(t.TempDir(), "adapters.json")
	mustWrite(t, file, `{"adapters":[{"name":"probebare","command":"echo","description":"echo, passed straight through","bare":true}]}`)
	e.setenv("MCPX_PATHS_ADAPTERS=" + file)
	if out, err := e.try("adapter", "list"); err == nil || !strings.Contains(out, "adapters.allowBare") {
		t.Errorf("with adapters.allowBare off, a bare declaration should fail to load naming the setting: %v\n%s", err, out)
	}
	e.setenv("MCPX_ADAPTERS_ALLOW_BARE=true")
	if out := e.run("adapter", "list"); !strings.Contains(out, "probebare") {
		t.Errorf("adapters.allowBare on should load the bare declaration:\n%s", out)
	}
}

// TestPathsAPIsFindsADeclarationFile is the effect probe for paths.apis:
// `mcpx api list` declares nothing by default, and lists the API in the
// file the environment points it at.
func TestPathsAPIsFindsADeclarationFile(t *testing.T) {
	e := newEnv(t, oneServer)
	if out := e.run("api", "list"); !strings.Contains(out, "No API specifications declared") {
		t.Errorf("by default no APIs should be declared:\n%s", out)
	}
	dir := t.TempDir()
	spec := filepath.Join(dir, "spec.json")
	mustWrite(t, spec, `{"openapi":"3.0.0","info":{"title":"P","version":"1"},"servers":[{"url":"http://127.0.0.1:9"}],"paths":{"/w":{"get":{"operationId":"listwidgets","responses":{"200":{"description":"ok"}}}}}}`)
	file := filepath.Join(dir, "apis.json")
	mustWrite(t, file, `{"apis":[{"name":"probeapi","spec":`+jsonQuote(spec)+`}]}`)
	e.setenv("MCPX_PATHS_APIS=" + file)
	if out := e.run("api", "list"); !strings.Contains(out, "probeapi") || !strings.Contains(out, "1 operations") {
		t.Errorf("paths.apis should declare probeapi with its one operation:\n%s", out)
	}
}

// TestLoggingTraceLogsEachCall is the effect probe for logging.trace: the
// daemon's log names the server and tool of a call only when tracing is on.
func TestLoggingTraceLogsEachCall(t *testing.T) {
	traced := func(on string) bool {
		e := newEnv(t, oneServer)
		e.setenv("MCPX_TRACE=" + on)
		e.run("call", "demo.echo", `{"message":"x"}`)
		b, _ := os.ReadFile(filepath.Join(e.dir, "state", "daemon.log"))
		return strings.Contains(string(b), "call demo.echo")
	}
	if traced("false") {
		t.Error("with logging.trace off the daemon log should not trace the call")
	}
	if !traced("true") {
		t.Error("logging.trace should log the call to demo.echo in the daemon log")
	}
}

// TestExecWhereRemoteRunsOnTheDaemon is the effect probe for exec.where:
// a local run is a child of each short-lived mcpx process, so two runs see
// different parents, while a remote run is a child of the one daemon.
func TestExecWhereRemoteRunsOnTheDaemon(t *testing.T) {
	parents := func(where string) (string, string) {
		e := newEnv(t, oneServer)
		e.setenv("MCPX_EXEC_WHERE=" + where)
		script := filepath.Join(e.dir, "where.ts")
		mustWrite(t, script, `console.log(process.ppid)`)
		a, _, err := e.split("run", script)
		if err != nil {
			t.Fatalf("run (%s): %v", where, err)
		}
		b, _, _ := e.split("run", script)
		return strings.TrimSpace(a), strings.TrimSpace(b)
	}
	if a, b := parents("local"); a == b {
		t.Errorf("exec.where=local should run each script under its own mcpx process, both had parent %s", a)
	}
	if a, b := parents("remote"); a != b || a == "" {
		t.Errorf("exec.where=remote should run both scripts under the daemon, got parents %q and %q", a, b)
	}
}

// TestPlumbingValidatePathsChecksTheLogDirUpFront is the effect probe for
// plumbing.validatePaths: a logging.dir that is a file stops a run before
// it starts, and with the check off the run goes ahead.
func TestPlumbingValidatePathsChecksTheLogDirUpFront(t *testing.T) {
	run := func(env ...string) (string, string, error) {
		e := newEnv(t, oneServer)
		notADir := filepath.Join(t.TempDir(), "notadir")
		mustWrite(t, notADir, "x")
		e.setenv(append([]string{"MCPX_LOGGING_DIR=" + notADir}, env...)...)
		script := filepath.Join(e.dir, "validate.ts")
		mustWrite(t, script, `console.log("ran")`)
		return e.split("run", script)
	}
	if _, se, err := run(); err == nil || !strings.Contains(se, "logging.dir") {
		t.Errorf("by default a file as logging.dir should stop the run: err=%v\n%s", err, se)
	}
	if out, se, err := run("MCPX_PLUMBING_VALIDATE_PATHS=false"); err != nil || !strings.Contains(out, "ran") {
		t.Errorf("with plumbing.validatePaths off the run should go ahead: err=%v\n%s%s", err, out, se)
	}
}

// TestScriptLanguageRunsPython is the effect probe for script.language: a
// file with no extension runs under the TypeScript runtime by default, so a
// Python print fails, and runs under Python when the language says so.
func TestScriptLanguageRunsPython(t *testing.T) {
	testsupport.RequireBinary(t, "python3")
	run := func(env ...string) (string, error) {
		e := newEnv(t, oneServer)
		e.setenv(env...)
		script := filepath.Join(e.dir, "noext")
		mustWrite(t, script, `print("ran-" + "py")`)
		out, _, err := e.split("run", script)
		return out, err
	}
	if out, err := run(); err == nil && strings.Contains(out, "ran-py") {
		t.Errorf("by default the extensionless script should not run as Python:\n%s", out)
	}
	if out, err := run("MCPX_SCRIPT_LANGUAGE=python"); err != nil || !strings.Contains(out, "ran-py") {
		t.Errorf("script.language=python should run the script under Python: err=%v\n%s", err, out)
	}
}

// TestArtifactsInterceptImagesStoresAnEmittedImage is the effect probe for
// artifacts.interceptImages: an image block a script emits is captured as
// an artifact by default, and left alone once interception is off.
func TestArtifactsInterceptImagesStoresAnEmittedImage(t *testing.T) {
	const script = `emit({ type: "image", data: "iVBORw0KGgo=", mimeType: "image/png" })`
	run := func(env ...string) string {
		e := newEnv(t, oneServer)
		e.setenv(env...)
		out, err := e.try("exec", "--json", script)
		if err != nil {
			t.Fatalf("exec: %v\n%s", err, out)
		}
		return out
	}
	if out := run(); !strings.Contains(out, `"name": "image.png"`) || strings.Contains(out, "iVBORw0KGgo=") {
		t.Errorf("by default an emitted image should become an artifact, and its body leave the results:\n%s", out)
	}
	if out := run("MCPX_ARTIFACTS_INTERCEPT_IMAGES=false"); strings.Contains(out, `"name": "image.png"`) {
		t.Errorf("with artifacts.interceptImages off the image should not be stored:\n%s", out)
	}
}

// TestPlumbingIndexOnQueryIngestsBeforeALogQuery is the effect probe for
// plumbing.indexOnQuery: `mcpx log` reads the daemon's records by default,
// and answers from the empty index alone once ingest-on-query is off.
func TestPlumbingIndexOnQueryIngestsBeforeALogQuery(t *testing.T) {
	query := func(env ...string) string {
		e := newEnv(t, oneServer)
		e.setenv(append([]string{"MCPX_LOGGING_DIR=" + filepath.Join(t.TempDir(), "logs")}, env...)...)
		e.run("ls")
		out, se, err := e.split("log")
		if err != nil {
			t.Fatalf("log: %v\n%s", err, se)
		}
		return out
	}
	if out := query(); !strings.Contains(out, "daemon starting") {
		t.Errorf("by default `mcpx log` should ingest and show the daemon's start:\n%s", out)
	}
	if out := query("MCPX_PLUMBING_INDEX_ON_QUERY=false"); strings.Contains(out, "daemon starting") {
		t.Errorf("with plumbing.indexOnQuery off nothing should have been ingested:\n%s", out)
	}
}

// TestArtifactsChunkBytesSplitsAStreamedBody is the effect probe for
// artifacts.chunkBytes: a streamed four-byte artifact arrives in one chunk at
// the default size and in four at a one-byte size.
func TestArtifactsChunkBytesSplitsAStreamedBody(t *testing.T) {
	chunks := func(env ...string) int {
		e := newEnv(t, oneServer)
		e.setenv(append([]string{"MCPX_EXEC_OUTPUT=stream", "MCPX_ARTIFACTS_DELIVERY=stream"}, env...)...)
		out, err := e.try("exec", artifactScript)
		if err != nil {
			t.Fatalf("exec: %v\n%s", err, out)
		}
		return strings.Count(out, `"type":"artifact.chunk"`)
	}
	if n := chunks(); n != 1 {
		t.Errorf("at the default chunk size a four-byte body should arrive in one chunk, got %d", n)
	}
	if n := chunks("MCPX_ARTIFACTS_CHUNK_BYTES=1"); n != 4 {
		t.Errorf("artifacts.chunkBytes=1 should split the four-byte body into four chunks, got %d", n)
	}
}

// launcherRun runs a one-line script with the given launcher source and
// environment, and returns its stdout.
func launcherRun(t *testing.T, launcher string, env ...string) (string, error) {
	t.Helper()
	file := filepath.Join(t.TempDir(), "launcher.ts")
	mustWrite(t, file, launcher)
	return launcherAt(t, file, env...)
}

// launcherAt runs a one-line script with script.launcher set to source,
// which may be a file, a directory or inline text, and returns its stdout.
func launcherAt(t *testing.T, source string, env ...string) (string, error) {
	t.Helper()
	e := newEnv(t, oneServer)
	e.setenv(append([]string{"MCPX_SCRIPT_LAUNCHER=" + source}, env...)...)
	script := filepath.Join(e.dir, "launched.ts")
	mustWrite(t, script, `console.log("script-ran")`)
	out, _, err := e.split("run", script)
	return out, err
}

// TestScriptLauncherReplacesTheLauncher is the effect probe for
// script.launcher: the default launcher runs the script, and a launcher
// given in the environment runs in its place.
func TestScriptLauncherReplacesTheLauncher(t *testing.T) {
	e := newEnv(t, oneServer)
	script := filepath.Join(e.dir, "launched.ts")
	mustWrite(t, script, `console.log("script-ran")`)
	if out := e.run("run", script); !strings.Contains(out, "script-ran") || strings.Contains(out, "custom-launcher") {
		t.Errorf("the default launcher should run the script:\n%s", out)
	}
	if out, err := launcherRun(t, `console.log("custom-launcher");`); err != nil || !strings.Contains(out, "custom-launcher") || strings.Contains(out, "script-ran") {
		t.Errorf("script.launcher should replace the launcher (err %v):\n%s", err, out)
	}
}

// TestPathsPlaceholdersDeclaresALauncherPlaceholder is the effect probe for
// paths.placeholders: a launcher that uses @greet fails with no placeholder
// search path, and runs greet.ts once the path names its directory.
func TestPathsPlaceholdersDeclaresALauncherPlaceholder(t *testing.T) {
	if out, err := launcherRun(t, "@greet\n"); err == nil && strings.Contains(out, "greeted") {
		t.Errorf("with no placeholder path @greet should not resolve:\n%s", out)
	}
	dir := t.TempDir()
	mustWrite(t, filepath.Join(dir, "greet.ts"), `console.log("greeted");`)
	if out, err := launcherRun(t, "@greet\n", "MCPX_PATHS_PLACEHOLDERS="+dir); err != nil || !strings.Contains(out, "greeted") {
		t.Errorf("paths.placeholders should make @greet run greet.ts (err %v):\n%s", err, out)
	}
}

// TestPlumbingLauncherPlaceholderRepeatAllowsATwiceUsedName is the effect
// probe for plumbing.launcherPlaceholderRepeat: a launcher that uses @greet
// twice is refused by default and runs it twice once greet is listed.
func TestPlumbingLauncherPlaceholderRepeatAllowsATwiceUsedName(t *testing.T) {
	dir := t.TempDir()
	mustWrite(t, filepath.Join(dir, "greet.ts"), `console.log("greeted");`)
	if out, err := launcherRun(t, "@greet\n@greet\n", "MCPX_PATHS_PLACEHOLDERS="+dir); err == nil {
		t.Errorf("a placeholder used twice should be refused by default:\n%s", out)
	}
	out, err := launcherRun(t, "@greet\n@greet\n", "MCPX_PATHS_PLACEHOLDERS="+dir, "MCPX_PLUMBING_LAUNCHER_PLACEHOLDER_REPEAT=greet")
	if err != nil || strings.Count(out, "greeted") != 2 {
		t.Errorf("plumbing.launcherPlaceholderRepeat=greet should run @greet twice (err %v):\n%s", err, out)
	}
}

// TestPlumbingAllowTsJsOverlapPicksAmbiguousScripts is the effect probe for
// plumbing.allowTsJsOverlap: a script directory holding foo.ts and foo.js
// makes `mcpx run foo` ambiguous by default, and runs once overlap is allowed.
func TestPlumbingAllowTsJsOverlapPicksAmbiguousScripts(t *testing.T) {
	run := func(env ...string) (string, string, error) {
		e := newEnv(t, oneServer)
		dir := t.TempDir()
		mustWrite(t, filepath.Join(dir, "foo.ts"), `console.log("ts")`)
		mustWrite(t, filepath.Join(dir, "foo.js"), `console.log("js")`)
		e.setenv(append([]string{"MCPX_PATHS_SCRIPTS=" + dir}, env...)...)
		return e.split("run", "foo")
	}
	if _, se, err := run(); err == nil || !strings.Contains(se, "ambiguous") {
		t.Errorf("foo.ts beside foo.js should be ambiguous by default (err %v):\n%s", err, se)
	}
	if out, se, err := run("MCPX_PLUMBING_ALLOW_TS_JS_OVERLAP=true"); err != nil {
		t.Errorf("plumbing.allowTsJsOverlap should let foo run (err %v):\n%s%s", err, out, se)
	}
}

// launcherDir is a directory of launcher fragments: a.ts at the top and
// b.ts one level down.
func launcherDir(t *testing.T) string {
	t.Helper()
	dir := t.TempDir()
	if err := os.MkdirAll(filepath.Join(dir, "sub"), 0o755); err != nil {
		t.Fatal(err)
	}
	mustWrite(t, filepath.Join(dir, "a.ts"), `console.log("dir-a");`)
	mustWrite(t, filepath.Join(dir, "sub", "b.ts"), `console.log("dir-b");`)
	return dir
}

// TestPlumbingSourceDirAllowedTakesADirectoryAsSource is the effect probe
// for plumbing.sourceDirAllowed: a directory given as the launcher is read
// by default and refused once directories are not allowed.
func TestPlumbingSourceDirAllowedTakesADirectoryAsSource(t *testing.T) {
	dir := launcherDir(t)
	if out, err := launcherAt(t, dir); err != nil || !strings.Contains(out, "dir-a") {
		t.Errorf("a directory should stand in for the launcher by default (err %v):\n%s", err, out)
	}
	if out, err := launcherAt(t, dir, "MCPX_PLUMBING_SOURCE_DIR_ALLOWED=false"); err == nil || strings.Contains(out, "dir-a") {
		t.Errorf("plumbing.sourceDirAllowed=false should refuse the directory (err %v):\n%s", err, out)
	}
}

// TestPlumbingSourceDirRecursiveReadsSubdirectories is the effect probe for
// plumbing.sourceDirRecursive: a directory launcher reads only its top level
// by default, and its subdirectories too when recursion is on.
func TestPlumbingSourceDirRecursiveReadsSubdirectories(t *testing.T) {
	dir := launcherDir(t)
	if out, err := launcherAt(t, dir); err != nil || strings.Contains(out, "dir-b") {
		t.Errorf("by default the subdirectory should not be read (err %v):\n%s", err, out)
	}
	if out, err := launcherAt(t, dir, "MCPX_PLUMBING_SOURCE_DIR_RECURSIVE=true"); err != nil || !strings.Contains(out, "dir-b") {
		t.Errorf("plumbing.sourceDirRecursive should read sub/b.ts (err %v):\n%s", err, out)
	}
}

// TestPlumbingSourceProbePathsReadsANamedFile is the effect probe for
// plumbing.sourceProbePaths: a launcher value naming a file is read from it
// by default, and taken as inline source once probing is off.
func TestPlumbingSourceProbePathsReadsANamedFile(t *testing.T) {
	file := filepath.Join(launcherDir(t), "a.ts")
	if out, err := launcherAt(t, file); err != nil || !strings.Contains(out, "dir-a") {
		t.Errorf("a path should be read as a file by default (err %v):\n%s", err, out)
	}
	if out, err := launcherAt(t, file, "MCPX_PLUMBING_SOURCE_PROBE_PATHS=false"); err == nil || strings.Contains(out, "dir-a") {
		t.Errorf("plumbing.sourceProbePaths=false should take the path as source text (err %v):\n%s", err, out)
	}
}

// instructedServer is oneServer's fake with free-text instructions, set
// through FAKEMCP_INSTRUCTIONS (spelled with a JSON escape because newEnv
// replaces every "FAKE" in a config with the binary's path).
const instructedServer = `{
  "mcpServers": {
    "demo": {
      "command": "FAKE",
      "env": { "\u0046AKEMCP_INSTRUCTIONS": "probe-instructions" },
      "mcpx": { "sharing": "shared", "scope": "global" }
    }
  }
}`

// TestCatalogInstructionsShowsServerGuidance is the effect probe for
// catalog.instructions: `mcpx types` carries the server's instructions by
// default and leaves them out once the setting is off.
func TestCatalogInstructionsShowsServerGuidance(t *testing.T) {
	e := newEnv(t, instructedServer)
	if out := e.run("types", "demo"); !strings.Contains(out, "probe-instructions") {
		t.Errorf("by default `types` should carry the server's instructions:\n%s", out)
	}
	e.setenv("MCPX_CATALOG_INSTRUCTIONS=false")
	if out := e.run("types", "demo"); strings.Contains(out, "probe-instructions") {
		t.Errorf("catalog.instructions=false should leave the instructions out:\n%s", out)
	}
}

// completingServer is oneServer's fake answering every completion with
// three values, set through FAKEMCP_COMPLETIONS (JSON-escaped, because
// newEnv replaces every "FAKE" in a config with the binary's path).
const completingServer = `{
  "mcpServers": {
    "demo": {
      "command": "FAKE",
      "env": { "\u0046AKEMCP_COMPLETIONS": "alpha,beta,gamma" },
      "mcpx": { "sharing": "shared", "scope": "global" }
    }
  }
}`

// TestCompletionMaxValuesCapsTheValues is the effect probe for
// completion.maxValues: all three of the server's values come back by
// default, and one once the environment caps completions at one.
func TestCompletionMaxValuesCapsTheValues(t *testing.T) {
	values := func(env ...string) int {
		e := newEnv(t, completingServer)
		e.setenv(env...)
		e.run("refresh")
		body := `{"server":"demo","ref":{"type":"ref/prompt","name":"summarise"},"argument":{"name":"style","value":""}}`
		resp, err := e.socketClient(t).Post("http://mcpx/v1/complete", "application/json", strings.NewReader(body))
		if err != nil {
			t.Fatal(err)
		}
		defer func() { _ = resp.Body.Close() }()
		var got struct {
			Completion struct {
				Values []string `json:"values"`
			} `json:"completion"`
		}
		if err := json.NewDecoder(resp.Body).Decode(&got); err != nil {
			t.Fatalf("decode completion (status %d): %v", resp.StatusCode, err)
		}
		return len(got.Completion.Values)
	}
	if n := values(); n != 3 {
		t.Errorf("by default all three completion values should come back, got %d", n)
	}
	if n := values("MCPX_COMPLETION_MAX_VALUES=1"); n != 1 {
		t.Errorf("completion.maxValues=1 should cap the values at one, got %d", n)
	}
}

// TestDoctorTimeoutBoundsTheDaemonCheck is the effect probe for
// doctor.timeout: `mcpx doctor` reaches the daemon at the default bound, and
// a one-nanosecond bound fails the connection check before it can.
func TestDoctorTimeoutBoundsTheDaemonCheck(t *testing.T) {
	e := newEnv(t, oneServer)
	if out, err := e.try("doctor"); err != nil {
		t.Errorf("at the default doctor.timeout the daemon should be reached (err %v):\n%s", err, out)
	}
	e2 := newEnv(t, oneServer)
	e2.setenv("MCPX_DOCTOR_TIMEOUT=1ns")
	if out, err := e2.try("doctor"); err == nil || !strings.Contains(out, "deadline exceeded") {
		t.Errorf("doctor.timeout=1ns should fail the connection check on its deadline (err %v):\n%s", err, out)
	}
}

// TestRegistryTimeoutBoundsARegistryRequest is the effect probe for
// registry.timeout: a search against the stub registry answers at the
// default bound, and a one-nanosecond bound cuts it off.
func TestRegistryTimeoutBoundsARegistryRequest(t *testing.T) {
	e := newEnv(t, oneServer)
	if out, err := e.try("registry", "search", "demo"); err != nil || !strings.Contains(out, "a stub entry") {
		t.Errorf("at the default registry.timeout the stub should answer (err %v):\n%s", err, out)
	}
	e2 := newEnv(t, oneServer)
	e2.setenv("MCPX_REGISTRY_TIMEOUT=1ns")
	if out, err := e2.try("registry", "search", "demo"); err == nil || strings.Contains(out, "a stub entry") {
		t.Errorf("registry.timeout=1ns should cut the search off (err %v):\n%s", err, out)
	}
}

// TestPoolCallTimeoutBoundsASlowCall is the effect probe for
// pool.callTimeout: a 600ms call finishes at the default bound and is cut
// off at a 100ms one.
func TestPoolCallTimeoutBoundsASlowCall(t *testing.T) {
	call := func(env ...string) (string, error) {
		e := newEnv(t, oneServer)
		e.setenv(env...)
		return e.try("call", "demo.slow", `{"ms":600}`)
	}
	if out, err := call(); err != nil || !strings.Contains(out, "slept 600ms") {
		t.Errorf("at the default pool.callTimeout the slow call should finish (err %v):\n%s", err, out)
	}
	if out, err := call("MCPX_POOL_CALL_TIMEOUT=100ms"); err == nil || strings.Contains(out, "slept 600ms") {
		t.Errorf("pool.callTimeout=100ms should cut the 600ms call off (err %v):\n%s", err, out)
	}
}

// TestPoolStartTimeoutBoundsServerStart is the effect probe for
// pool.startTimeout: the fake server starts at the default bound, and a
// one-nanosecond bound fails its start.
func TestPoolStartTimeoutBoundsServerStart(t *testing.T) {
	call := func(env ...string) (string, error) {
		e := newEnv(t, oneServer)
		e.setenv(env...)
		return e.try("call", "demo.echo", `{"message":"started"}`)
	}
	if out, err := call(); err != nil || !strings.Contains(out, "started") {
		t.Errorf("at the default pool.startTimeout the server should start (err %v):\n%s", err, out)
	}
	if out, err := call("MCPX_POOL_START_TIMEOUT=1ns"); err == nil || !strings.Contains(out, "deadline exceeded") {
		t.Errorf("pool.startTimeout=1ns should fail the server's start (err %v):\n%s", err, out)
	}
}

// TestDaemonRefreshTimeoutBoundsARefresh is the effect probe for
// daemon.refreshTimeout: `mcpx refresh` lists the server at the default
// bound, and a one-nanosecond bound leaves it in error.
func TestDaemonRefreshTimeoutBoundsARefresh(t *testing.T) {
	refresh := func(env ...string) string {
		e := newEnv(t, oneServer)
		e.setenv(env...)
		out, _ := e.try("refresh")
		return out
	}
	if out := refresh(); strings.Contains(out, "deadline exceeded") || !strings.Contains(out, "running") {
		t.Errorf("at the default daemon.refreshTimeout the refresh should succeed:\n%s", out)
	}
	if out := refresh("MCPX_DAEMON_REFRESH_TIMEOUT=1ns"); !strings.Contains(out, "deadline exceeded") {
		t.Errorf("daemon.refreshTimeout=1ns should cut the refresh off:\n%s", out)
	}
}

// TestDaemonWarmTimeoutBoundsTheStartupWarm is the effect probe for
// daemon.warmTimeout: the daemon's startup warm reaches the fake server at
// the default bound, and fails it at a one-nanosecond bound.
func TestDaemonWarmTimeoutBoundsTheStartupWarm(t *testing.T) {
	warmLine := func(env ...string) string {
		e := newEnv(t, oneServer)
		e.setenv(env...)
		e.run("ls")
		_, _ = e.try("daemon", "stop")
		b, _ := os.ReadFile(filepath.Join(e.dir, "state", "daemon.log"))
		for _, line := range strings.Split(string(b), "\n") {
			if strings.Contains(line, "warm complete") {
				return line
			}
		}
		t.Fatalf("no warm line in the daemon log:\n%s", b)
		return ""
	}
	if line := warmLine(); !strings.Contains(line, "(0 failed)") {
		t.Errorf("at the default daemon.warmTimeout the warm should succeed: %s", line)
	}
	if line := warmLine("MCPX_DAEMON_WARM_TIMEOUT=1ns"); !strings.Contains(line, "(1 failed)") {
		t.Errorf("daemon.warmTimeout=1ns should fail the warm: %s", line)
	}
}

// autostartFails reports whether the first command in a fresh env fails to
// start its daemon under the given environment.
func autostartFails(t *testing.T, env ...string) (bool, string) {
	t.Helper()
	e := newEnv(t, oneServer)
	saved := e.envVars
	e.setenv(env...)
	out, err := e.try("ls")
	// The daemon that was given up on still starts. Reach it with the
	// default bounds, so the env's cleanup can find and stop it rather than
	// leaving it running past the suite.
	e.envVars = saved
	_, _ = e.try("ls")
	return err != nil && strings.Contains(out, "did not become ready"), out
}

// TestAutostartConnectTimeoutBoundsTheWaitForADaemon is the effect probe
// for autostart.connectTimeout: an auto-started daemon is waited for at the
// default bound, and given up on at a one-nanosecond bound.
func TestAutostartConnectTimeoutBoundsTheWaitForADaemon(t *testing.T) {
	if failed, out := autostartFails(t); failed {
		t.Errorf("at the default autostart.connectTimeout the daemon should start:\n%s", out)
	}
	if failed, out := autostartFails(t, "MCPX_AUTOSTART_CONNECT_TIMEOUT=1ns"); !failed {
		t.Errorf("autostart.connectTimeout=1ns should give up on the daemon:\n%s", out)
	}
}

// TestAutostartPingTimeoutBoundsEachReadinessPing is the effect probe for
// autostart.pingTimeout: the readiness ping answers at the default bound,
// and none can at a one-nanosecond bound, so the start is given up on. The
// two cases use different connect waits on purpose. The default-ping case
// waits generously: its assertion is that the daemon does start, and under
// CPU contention startup can take far longer than a short wait allows.
// The one-nanosecond case waits only two seconds: no ping can answer in a
// nanosecond however fast the daemon starts, so the give-up is certain
// and a short wait keeps the test quick.
func TestAutostartPingTimeoutBoundsEachReadinessPing(t *testing.T) {
	if failed, out := autostartFails(t, "MCPX_AUTOSTART_CONNECT_TIMEOUT=60s"); failed {
		t.Errorf("at the default autostart.pingTimeout the daemon should start:\n%s", out)
	}
	if failed, out := autostartFails(t, "MCPX_AUTOSTART_CONNECT_TIMEOUT=2s", "MCPX_AUTOSTART_PING_TIMEOUT=1ns"); !failed {
		t.Errorf("autostart.pingTimeout=1ns should leave every ping unanswered:\n%s", out)
	}
}

// TestDaemonInlineStartTimeoutBoundsTheInProcessStartFromTheEnvironment is
// the effect probe for daemon.inlineStartTimeout. With daemon.inline=always
// the client skips the socket and spawn rungs and hosts the daemon in
// process; a 1ns start bound expires before the first ping, so the inline
// rung reports that the in-process daemon did not answer. The default
// bound lets the same ls answer.
func TestDaemonInlineStartTimeoutBoundsTheInProcessStartFromTheEnvironment(t *testing.T) {
	inline := func(extra ...string) (string, error) {
		e := newEnv(t, oneServer)
		e.setenv(append([]string{"MCPX_DAEMON_INLINE=always"}, extra...)...)
		return e.try("ls")
	}
	if out, err := inline(); err != nil || !strings.Contains(out, "demo") {
		t.Fatalf("oracle: an inline ls with the default start bound should list demo (err %v):\n%s", err, out)
	}
	out, err := inline("MCPX_DAEMON_INLINE_START_TIMEOUT=1ns")
	if err == nil {
		t.Fatalf("with daemon.inlineStartTimeout=1ns the inline ls should fail; got:\n%s", out)
	}
	if !strings.Contains(out, "the in-process daemon did not answer") {
		t.Fatalf("the failure should be the inline rung's expired start, got:\n%s", out)
	}
}

// TestDaemonProbeTimeoutDecidesWhetherASiblingDaemonIsReportedFromTheEnvironment
// is the effect probe for daemon.probeTimeout. A daemon is started for one
// configuration; a second configuration then finds no daemon of its own
// and, with autostart off, fails with the connection ladder. The ladder's
// sibling rung dials every other socket in the state directory within
// daemon.probeTimeout: by default the live sibling answers and is
// reported, and with a 1ns bound no dial can finish, so it is not.
func TestDaemonProbeTimeoutDecidesWhetherASiblingDaemonIsReportedFromTheEnvironment(t *testing.T) {
	ladder := func(extra ...string) string {
		e := newEnv(t, oneServer)
		e.run("ls") // a daemon for the first configuration
		other := filepath.Join(e.dir, "other.json")
		mustWrite(t, other, "{}")
		e.setenv(append([]string{"MCPX_DAEMON_AUTOSTART=false"}, extra...)...)
		out, err := e.try("--config", other, "ls")
		if err == nil {
			t.Fatalf("oracle: the second configuration has no daemon and autostart is off, so ls should fail; got:\n%s", out)
		}
		return out
	}
	if out := ladder(); !strings.Contains(out, "1 other daemon(s) here") {
		t.Fatalf("oracle: with the default probe bound the live sibling should be reported, got:\n%s", out)
	}
	if out := ladder("MCPX_DAEMON_PROBE_TIMEOUT=1ns"); strings.Contains(out, "sibling-socket") {
		t.Fatalf("with daemon.probeTimeout=1ns no sibling dial can finish, so none should be reported, got:\n%s", out)
	}
}

// TestDaemonInlineAlwaysAnswersWithoutASocketFromTheEnvironment is the
// effect probe for daemon.inline. By default ls auto-starts a daemon that
// outlives the command, so a later ls with autostart off still answers.
// With daemon.inline=always the client skips the socket and spawn rungs
// and answers from an in-process daemon: the ls lists demo, and nothing
// is left running for the later ls to reach.
func TestDaemonInlineAlwaysAnswersWithoutASocketFromTheEnvironment(t *testing.T) {
	leftRunning := func(inline bool) bool {
		e := newEnv(t, oneServer)
		if inline {
			e.setenv("MCPX_DAEMON_INLINE=always")
		}
		if out := e.run("ls"); !strings.Contains(out, "demo") {
			t.Fatalf("ls should list demo (inline %v):\n%s", inline, out)
		}
		e.unsetenv("MCPX_DAEMON_INLINE")
		e.setenv("MCPX_DAEMON_AUTOSTART=false")
		_, err := e.try("ls")
		return err == nil
	}
	if !leftRunning(false) {
		t.Fatalf("oracle: by default ls should leave a daemon that a later ls reaches with autostart off")
	}
	if leftRunning(true) {
		t.Fatalf("with daemon.inline=always the ls should leave no daemon behind")
	}
}

// autostartFailure runs ls with autostart settings that keep the daemon
// from coming up and returns the failure the client prints.
func autostartFailure(t *testing.T, env ...string) string {
	t.Helper()
	e := newEnv(t, oneServer)
	e.setenv(append([]string{"MCPX_AUTOSTART_CONNECT_TIMEOUT=2s"}, env...)...)
	out, err := e.try("ls")
	if err == nil {
		t.Fatalf("ls should fail to start a daemon with %v; got:\n%s", env, out)
	}
	return out
}

// TestAutostartBinChoosesTheDaemonExecutableFromTheEnvironment is the
// effect probe for autostart.bin. The default starts this binary as the
// daemon and ls answers; naming an executable that does not exist makes
// the spawn rung fail on exec of that path.
func TestAutostartBinChoosesTheDaemonExecutableFromTheEnvironment(t *testing.T) {
	if out := newEnv(t, oneServer).run("ls"); !strings.Contains(out, "demo") {
		t.Fatalf("oracle: with the default autostart.bin ls should list demo:\n%s", out)
	}
	missing := filepath.Join(t.TempDir(), "no-such-mcpx")
	out := autostartFailure(t, "MCPX_AUTOSTART_BIN="+missing)
	if !strings.Contains(out, "fork/exec "+missing) {
		t.Fatalf("the spawn rung should fail on exec of autostart.bin, got:\n%s", out)
	}
}

// TestAutostartArgsReplaceTheDaemonCommandLineFromTheEnvironment is the
// effect probe for autostart.args. The default arguments start a daemon;
// replacing them with a flag the daemon command does not define makes
// the spawned process exit, and its own complaint about the flag is in
// the log excerpt the client prints.
func TestAutostartArgsReplaceTheDaemonCommandLineFromTheEnvironment(t *testing.T) {
	if out := newEnv(t, oneServer).run("ls"); !strings.Contains(out, "demo") {
		t.Fatalf("oracle: with the default autostart.args ls should list demo:\n%s", out)
	}
	out := autostartFailure(t, "MCPX_AUTOSTART_ARGS=daemon,--no-such-flag")
	if !strings.Contains(out, "flag provided but not defined: -no-such-flag") {
		t.Fatalf("the daemon started with the replaced arguments should refuse the flag, got:\n%s", out)
	}
}

// TestAutostartLogTailBoundsTheFailureExcerptFromTheEnvironment is the
// effect probe for autostart.logTail. A daemon that will not start has
// the end of its log shown; by default the excerpt holds the daemon's
// whole complaint, and with an 8-byte tail only the last 8 bytes of it.
func TestAutostartLogTailBoundsTheFailureExcerptFromTheEnvironment(t *testing.T) {
	bad := "MCPX_AUTOSTART_ARGS=daemon,--no-such-flag"
	if out := autostartFailure(t, bad); !strings.Contains(out, "flag provided but not defined") {
		t.Fatalf("oracle: the default excerpt should hold the daemon's complaint, got:\n%s", out)
	}
	out := autostartFailure(t, bad, "MCPX_AUTOSTART_LOG_TAIL=8")
	if strings.Contains(out, "flag provided but not defined") {
		t.Fatalf("with autostart.logTail=8 the excerpt should be cut to its last 8 bytes, got:\n%s", out)
	}
	if !strings.Contains(out, "ch-flag") {
		t.Fatalf("the 8-byte excerpt should be the end of the complaint, got:\n%s", out)
	}
}

// TestDaemonWatchConfigPicksUpAHandEditFromTheEnvironment is the effect
// probe for daemon.watchConfig. A running daemon serving demo has its
// configuration rewritten by hand to alpha and beta. Watching (the
// default), the next ls lists the new servers without a refresh; with
// watching off, it still lists demo.
func TestDaemonWatchConfigPicksUpAHandEditFromTheEnvironment(t *testing.T) {
	edited := func(extra ...string) string {
		e := newEnv(t, oneServer)
		e.setenv(append([]string{"MCPX_DAEMON_REAP_INTERVAL=100ms"}, extra...)...)
		e.run("ls")
		mustWrite(t, filepath.Join(e.dir, ".mcpx.json"), strings.ReplaceAll(twoVisibleServers, "FAKE", e.fake))
		time.Sleep(1200 * time.Millisecond) // past the edit's mtime second and a few reap ticks
		return e.run("ls")
	}
	if out := edited(); !strings.Contains(out, "beta") {
		t.Fatalf("oracle: watching, the hand edit should be picked up, got:\n%s", out)
	}
	if out := edited("MCPX_DAEMON_WATCH_CONFIG=false"); strings.Contains(out, "beta") || !strings.Contains(out, "demo") {
		t.Fatalf("with daemon.watchConfig=false the hand edit should not be picked up, got:\n%s", out)
	}
}

// TestPolicyWriteDeniesAnUnannotatedCallFromTheEnvironment is the effect
// probe for policy.write. demo.structured publishes no annotations, so
// it counts as a call that may write. Under the harness's allow it
// returns its result; with policy.write=deny it is refused by the write
// policy and never runs.
func TestPolicyWriteDeniesAnUnannotatedCallFromTheEnvironment(t *testing.T) {
	allow := newEnv(t, oneServer)
	if out := allow.run("call", "demo.structured", `{}`); !strings.Contains(out, "42") {
		t.Fatalf("oracle: under policy.write=allow the call should return its result:\n%s", out)
	}
	deny := newEnv(t, oneServer)
	deny.setenv("MCPX_POLICY_WRITE=deny")
	out, err := deny.try("call", "demo.structured", `{}`)
	if err == nil {
		t.Fatalf("with policy.write=deny the call should be refused; got:\n%s", out)
	}
	if !strings.Contains(out, "refused by the write policy (mode deny)") {
		t.Fatalf("the refusal should name the write policy, got:\n%s", out)
	}
}

// TestElicitConfirmDestructiveGatesADestructiveCallFromTheEnvironment is
// the effect probe for elicit.confirmDestructive. Under the harness's
// policy.write=allow, demo.wipe runs by default. With the floor on, a
// destructive call asks even under allow, and a headless caller cannot
// confirm, so it is refused as not confirmed.
func TestElicitConfirmDestructiveGatesADestructiveCallFromTheEnvironment(t *testing.T) {
	if out := newEnv(t, oneServer).run("call", "demo.wipe", `{}`); !strings.Contains(out, "wiped") {
		t.Fatalf("oracle: by default the destructive call should run:\n%s", out)
	}
	e := newEnv(t, oneServer)
	e.setenv("MCPX_ELICIT_CONFIRM_DESTRUCTIVE=true")
	out, err := e.try("call", "demo.wipe", `{}`)
	if err == nil {
		t.Fatalf("with elicit.confirmDestructive=true the headless destructive call should be refused; got:\n%s", out)
	}
	if !strings.Contains(out, "may be destructive and was not confirmed") {
		t.Fatalf("the refusal should say the destructive call was not confirmed, got:\n%s", out)
	}
}

// TestAutonomyMaxClampsARecipeRunFromTheEnvironment is the effect probe
// for autonomy.max. A recipe that leaves a file behind is run with
// --autonomy run. Under the default ceiling (run) it runs and the file
// appears; with autonomy.max=propose the request is lowered to propose,
// the CLI says so, and the file never appears.
func TestAutonomyMaxClampsARecipeRunFromTheEnvironment(t *testing.T) {
	run := func(extra ...string) (string, bool) {
		e := newEnv(t, oneServer)
		writeRecipe(t, e, "mark", markRecipe)
		e.setenv(extra...)
		mark := filepath.Join(e.dir, "mark.out")
		out := e.run("recipes", "run", "mark", "--autonomy", "run", "path="+mark)
		_, err := os.Stat(mark)
		return out, err == nil
	}
	if out, marked := run(); !marked {
		t.Fatalf("oracle: under the default ceiling the recipe should run and leave its mark:\n%s", out)
	}
	out, marked := run("MCPX_AUTONOMY_MAX=propose")
	if marked {
		t.Fatalf("with autonomy.max=propose the recipe should not run:\n%s", out)
	}
	if !strings.Contains(out, "clamped to propose by autonomy.max") {
		t.Fatalf("the CLI should say the request was lowered, got:\n%s", out)
	}
}

// TestPromptAutonomyRunsTheMatchedRecipeFromTheEnvironment is the effect
// probe for prompt.autonomy. A prompt that matches a recipe leaving a
// file behind proposes the script by default (propose): the source comes
// back and nothing runs. With prompt.autonomy=run the same prompt runs
// the recipe and the file appears.
func TestPromptAutonomyRunsTheMatchedRecipeFromTheEnvironment(t *testing.T) {
	prompt := func(extra ...string) (string, bool) {
		e := newEnv(t, oneServer)
		writeRecipe(t, e, "mark", markRecipe)
		e.setenv(extra...)
		mark := filepath.Join(e.dir, "mark.out")
		out := e.run("prompt", "--set", "path="+mark, "leave a mark on the filesystem")
		_, err := os.Stat(mark)
		return out, err == nil
	}
	out, marked := prompt()
	if marked || !strings.Contains(out, "writeFileSync") {
		t.Fatalf("oracle: by default the prompt should propose the recipe's source without running it (marked %v):\n%s", marked, out)
	}
	if out, marked := prompt("MCPX_PROMPT_AUTONOMY=run"); !marked || !strings.Contains(out, "marked") {
		t.Fatalf("with prompt.autonomy=run the recipe should run and leave its mark (marked %v):\n%s", marked, out)
	}
}

// TestHooksAutonomySkipsAConfiguredHookFromTheEnvironment is the effect
// probe for hooks.autonomy. A hook from the environment is code the
// command did not send. At the default (run) it runs after the body;
// with hooks.autonomy=propose it is skipped and stderr says so.
func TestHooksAutonomySkipsAConfiguredHookFromTheEnvironment(t *testing.T) {
	hook := `MCPX_SCRIPT_ON_SUCCESS=console.log("configured-hook")`
	def := newEnv(t, oneServer)
	def.setenv(hook)
	if out := def.run("exec", `console.log("body")`); !strings.Contains(out, "configured-hook") {
		t.Fatalf("oracle: at the default hooks.autonomy the configured hook should run:\n%s", out)
	}
	e := newEnv(t, oneServer)
	e.setenv(hook, "MCPX_HOOKS_AUTONOMY=propose")
	out, errOut, err := e.split("exec", `console.log("body")`)
	if err != nil {
		t.Fatalf("%v\n%s\n%s", err, out, errOut)
	}
	if strings.Contains(out, "configured-hook") || !strings.Contains(out, "body") {
		t.Fatalf("with hooks.autonomy=propose the body should run and the hook should not:\n%s", out)
	}
	if !strings.Contains(errOut, "hooks.autonomy is propose; not running the configured onSuccess hook") {
		t.Fatalf("stderr should say the hook was skipped:\n%s", errOut)
	}
}

// TestRepairAutonomyOffDropsTheDiagnosticsFromTheEnvironment is the
// effect probe for repair.autonomy. A server's schema gains a required
// argument, and a call made to the old shape is refused. At the default
// (advise) the refusal carries a diagnostic naming the missing argument;
// with repair.autonomy=off only the validator's own error comes back.
func TestRepairAutonomyOffDropsTheDiagnosticsFromTheEnvironment(t *testing.T) {
	refused := func(extra ...string) callErrorDoc {
		schema := filepath.Join(t.TempDir(), "schema")
		mustWrite(t, schema, "v1")
		e := newEnv(t, strings.ReplaceAll(schemaServer, "VERSIONFILE", schema))
		e.setenv(extra...)
		e.run("refresh")
		mustWrite(t, schema, "v2")
		e.run("refresh")
		resp, err := e.socketClient(t).Post("http://mcpx/v1/call", "application/json",
			strings.NewReader(`{"server":"demo","tool":"create_issue","args":{"title":"x"}}`))
		if err != nil {
			t.Fatal(err)
		}
		b, _ := io.ReadAll(resp.Body)
		_ = resp.Body.Close()
		var got callErrorDoc
		if err := json.Unmarshal(b, &got); err != nil || resp.StatusCode != http.StatusBadGateway {
			t.Fatalf("the old-shape call should be refused with 502: status %d: %v\n%s", resp.StatusCode, err, b)
		}
		if !strings.HasPrefix(got.Error, "Invalid params") {
			t.Fatalf("the refusal's own error should come back:\n%s", b)
		}
		return got
	}
	if got := refused(); len(got.Diagnostics) == 0 {
		t.Fatalf("oracle: at the default repair.autonomy the refusal should carry diagnostics: %+v", got)
	}
	if got := refused("MCPX_REPAIR_AUTONOMY=off"); len(got.Diagnostics) != 0 {
		t.Fatalf("with repair.autonomy=off no diagnostics should be attached: %+v", got)
	}
}

// TestUpstreamProbeTimeoutBoundsTheDiscoverWaitFromTheEnvironment is the
// effect probe for upstream.probeTimeout. The era server in legacy-silent
// mode never answers server/discover, so a start waits out the probe
// bound before falling back to initialize. With a 300ms bound the call
// is answered inside a 2s start bound; with a one-hour bound the probe
// is still waiting when the start bound expires, and the call fails.
func TestUpstreamProbeTimeoutBoundsTheDiscoverWaitFromTheEnvironment(t *testing.T) {
	call := func(probe string) (string, error) {
		e, _ := eraEnv(t, "legacy-silent", "",
			"MCPX_POOL_START_TIMEOUT=2s", "MCPX_UPSTREAM_PROBE_TIMEOUT="+probe)
		return e.try("call", "era.hello", "{}")
	}
	if out, err := call("300ms"); err != nil || !strings.Contains(out, "hello from legacy-silent") {
		t.Fatalf("oracle: a 300ms probe bound should fall back to initialize in time (err %v):\n%s", err, out)
	}
	if out, err := call("1h"); err == nil {
		t.Fatalf("with upstream.probeTimeout=1h the start should run out before the probe gives up; got:\n%s", out)
	}
}

// lsDemoRow returns the fields of ls's row for the demo namespace:
// NAMESPACE TOOLS SHARING SCOPE LIVE STATE, then the description.
func lsDemoRow(t *testing.T, out string) []string {
	t.Helper()
	for _, line := range strings.Split(out, "\n") {
		if f := strings.Fields(line); len(f) >= 6 && f[0] == "demo" {
			return f
		}
	}
	t.Fatalf("no demo row in ls:\n%s", out)
	return nil
}

// TestDaemonWarmReadsTheServersAtStartupFromTheEnvironment is the effect
// probe for daemon.warm. The first ls auto-starts a daemon. Warming (the
// default), it reads demo's schemas on its own: tools and a running
// instance appear without any call. With daemon.warm=false nothing reads
// the server, so the row stays unread, with no tools and nothing live, for
// at least as long as the warm daemon took.
//
// Warming is asynchronous by design (Server.WarmAsync: "without delaying
// the listener"), so the first ls can answer before it finishes. The test
// used to assert on that first answer and failed whenever ls won the race,
// which on macOS was about two runs in three.
func TestDaemonWarmReadsTheServersAtStartupFromTheEnvironment(t *testing.T) {
	warm := newEnv(t, oneServer)
	start := time.Now()
	deadline := start.Add(30 * time.Second)
	for row := lsDemoRow(t, warm.run("ls")); row[1] == "0" || row[5] != "running"; row = lsDemoRow(t, warm.run("ls")) {
		if time.Now().After(deadline) {
			t.Fatalf("oracle: a warmed daemon should come to list demo's tools and a running instance: %v", row)
		}
		time.Sleep(100 * time.Millisecond)
	}
	took := time.Since(start)

	cold := newEnv(t, oneServer)
	cold.setenv("MCPX_DAEMON_WARM=false")
	cold.run("ls")
	time.Sleep(took + 500*time.Millisecond)
	if row := lsDemoRow(t, cold.run("ls")); row[1] != "0" || row[4] != "0" || row[5] != "unread" {
		t.Fatalf("with daemon.warm=false demo should stay unread, with no tools and nothing live: %v", row)
	}
}

// idleAfterACall makes one call to demo, waits past a 200ms idle bound,
// and returns ls's LIVE column for demo.
func idleAfterACall(t *testing.T, env ...string) string {
	t.Helper()
	e := newEnv(t, oneServer)
	e.setenv(env...)
	e.run("call", "demo.echo", `{"message":"x"}`)
	time.Sleep(1500 * time.Millisecond)
	return lsDemoRow(t, e.run("ls"))[4]
}

// TestPoolIdleTimeoutReapsAnIdleInstanceFromTheEnvironment is the effect
// probe for pool.idleTimeout. With the reaper ticking every 100ms, the
// instance a call started is still live 1.5s later under the default
// idle bound, and has been reaped under a 200ms one.
func TestPoolIdleTimeoutReapsAnIdleInstanceFromTheEnvironment(t *testing.T) {
	if live := idleAfterACall(t, "MCPX_DAEMON_REAP_INTERVAL=100ms"); live != "1" {
		t.Fatalf("oracle: under the default idle bound the instance should still be live, LIVE=%s", live)
	}
	if live := idleAfterACall(t, "MCPX_DAEMON_REAP_INTERVAL=100ms", "MCPX_POOL_IDLE_TIMEOUT=200ms"); live != "0" {
		t.Fatalf("with pool.idleTimeout=200ms the idle instance should be reaped, LIVE=%s", live)
	}
}

// TestDaemonReapIntervalSetsWhenIdleInstancesAreReapedFromTheEnvironment
// is the effect probe for daemon.reapInterval. With a 200ms idle bound,
// a reaper ticking every 100ms has reaped the instance 1.5s after the
// call; one ticking hourly has not yet looked, so it is still live.
func TestDaemonReapIntervalSetsWhenIdleInstancesAreReapedFromTheEnvironment(t *testing.T) {
	if live := idleAfterACall(t, "MCPX_POOL_IDLE_TIMEOUT=200ms", "MCPX_DAEMON_REAP_INTERVAL=100ms"); live != "0" {
		t.Fatalf("oracle: a 100ms reaper should have reaped the idle instance, LIVE=%s", live)
	}
	if live := idleAfterACall(t, "MCPX_POOL_IDLE_TIMEOUT=200ms", "MCPX_DAEMON_REAP_INTERVAL=1h"); live != "1" {
		t.Fatalf("with daemon.reapInterval=1h the reaper should not have run yet, LIVE=%s", live)
	}
}

// TestDaemonEndpointNamesTheDaemonToReachFromTheEnvironment is the effect
// probe for daemon.endpoint. By default the client finds or starts a
// local daemon and ls answers. A named endpoint short-circuits the
// ladder: when nothing answers there, ls fails and says it will not
// start a local daemon in its place.
func TestDaemonEndpointNamesTheDaemonToReachFromTheEnvironment(t *testing.T) {
	if out := newEnv(t, oneServer).run("ls"); !strings.Contains(out, "demo") {
		t.Fatalf("oracle: by default ls should list demo:\n%s", out)
	}
	e := newEnv(t, oneServer)
	e.setenv("MCPX_DAEMON_ENDPOINT=http://127.0.0.1:1")
	out, err := e.try("ls")
	if err == nil {
		t.Fatalf("with an unreachable daemon.endpoint ls should fail; got:\n%s", out)
	}
	if !strings.Contains(out, "will not start a local one") {
		t.Fatalf("the failure should say no local daemon is started instead, got:\n%s", out)
	}
}

// artifactsLeft stores one artifact under env, waits 2.5s, and returns
// how many artifacts the daemon still lists.
func artifactsLeft(t *testing.T, env ...string) int {
	t.Helper()
	e := newEnv(t, oneServer)
	e.setenv(env...)
	e.run("exec", artifactScript)
	time.Sleep(2500 * time.Millisecond)
	resp, err := e.socketClient(t).Get("http://mcpx/v1/artifacts")
	if err != nil {
		t.Fatal(err)
	}
	defer func() { _ = resp.Body.Close() }()
	var doc struct {
		Artifacts []json.RawMessage `json:"artifacts"`
	}
	if err := json.NewDecoder(resp.Body).Decode(&doc); err != nil {
		t.Fatal(err)
	}
	return len(doc.Artifacts)
}

// TestArtifactsTTLExpiresAnArtifactFromTheEnvironment is the effect probe
// for artifacts.ttl. With the collector sweeping every 200ms, a script's
// artifact is still listed 2.5s later under the default retention and
// has been collected under a 1s one.
func TestArtifactsTTLExpiresAnArtifactFromTheEnvironment(t *testing.T) {
	if n := artifactsLeft(t, "MCPX_ARTIFACTS_GC_INTERVAL=200ms"); n != 1 {
		t.Fatalf("oracle: under the default retention the artifact should still be listed, got %d", n)
	}
	if n := artifactsLeft(t, "MCPX_ARTIFACTS_GC_INTERVAL=200ms", "MCPX_ARTIFACTS_TTL=1s"); n != 0 {
		t.Fatalf("with artifacts.ttl=1s the artifact should have been collected, got %d", n)
	}
}

// TestArtifactsGCIntervalSetsWhenExpiredArtifactsAreSweptFromTheEnvironment
// is the effect probe for artifacts.gcInterval. Under a 1s retention, a
// collector sweeping every 200ms has removed the artifact 2.5s later;
// one sweeping hourly has not run yet, so it is still listed.
func TestArtifactsGCIntervalSetsWhenExpiredArtifactsAreSweptFromTheEnvironment(t *testing.T) {
	if n := artifactsLeft(t, "MCPX_ARTIFACTS_TTL=1s", "MCPX_ARTIFACTS_GC_INTERVAL=200ms"); n != 0 {
		t.Fatalf("oracle: a 200ms sweep should have collected the expired artifact, got %d", n)
	}
	if n := artifactsLeft(t, "MCPX_ARTIFACTS_TTL=1s", "MCPX_ARTIFACTS_GC_INTERVAL=1h"); n != 1 {
		t.Fatalf("with artifacts.gcInterval=1h no sweep should have run yet, got %d", n)
	}
}

// TestPoolMinKeepsAnInstanceWarmFromTheEnvironment is the effect probe
// for pool.min. Under a 200ms idle bound and a 100ms reaper, a call's
// instance is reaped 1.5s later by default (min 0); with pool.min=1 it
// survives the idle timeout.
func TestPoolMinKeepsAnInstanceWarmFromTheEnvironment(t *testing.T) {
	reap := []string{"MCPX_POOL_IDLE_TIMEOUT=200ms", "MCPX_DAEMON_REAP_INTERVAL=100ms"}
	if live := idleAfterACall(t, reap...); live != "0" {
		t.Fatalf("oracle: by default the idle instance should be reaped, LIVE=%s", live)
	}
	if live := idleAfterACall(t, append(reap, "MCPX_POOL_MIN=1")...); live != "1" {
		t.Fatalf("with pool.min=1 the instance should survive the idle timeout, LIVE=%s", live)
	}
}

// TestAutostartIdleExitStopsAnIdleDaemonFromTheEnvironment is the effect
// probe for autostart.idleExit. With instances reaped after 200ms, an
// auto-started daemon soon has nothing to do. Under the harness's 60s
// bound it is still there 4s later, so ls with autostart off reaches
// it; under a 1s bound it has exited, and the same ls fails.
func TestAutostartIdleExitStopsAnIdleDaemonFromTheEnvironment(t *testing.T) {
	stillThere := func(extra ...string) bool {
		e := newEnv(t, oneServer)
		e.setenv(append([]string{"MCPX_POOL_IDLE_TIMEOUT=200ms", "MCPX_DAEMON_REAP_INTERVAL=100ms"}, extra...)...)
		e.run("ls")
		time.Sleep(4 * time.Second)
		e.setenv("MCPX_DAEMON_AUTOSTART=false")
		_, err := e.try("ls")
		return err == nil
	}
	if !stillThere() {
		t.Fatalf("oracle: under the harness's idle bound the daemon should still be running")
	}
	if stillThere("MCPX_AUTOSTART_IDLE_EXIT=1s") {
		t.Fatalf("with autostart.idleExit=1s the idle daemon should have exited")
	}
}

// readAfterRenewal stores one artifact under a 6s retention swept every
// 200ms (plus env), reads it 4.5s after the store began, and returns the
// status of a second read: 200 if the first read renewed it past its
// expiry, 404 if it was collected. Callers set idleTtl=9s (renewal from
// the first read reaches past the second) and maxAge=6s (the cap is the
// plain retention again).
//
// The schedule used to be 2s/1.5s/3s, timed from after the store and a
// status call: under a loaded nix build the store plus that call took
// long enough that the "1.5s" read landed after the 2s expiry, and the
// first read 404ed. Times are now taken from the start of the store, the
// unit is three times larger, and the second read waits until the
// artifact is certainly past its unrenewed expiry, measured from when
// the store returned.
func readAfterRenewal(t *testing.T, env ...string) int {
	t.Helper()
	const unit = 3 * time.Second
	e := newEnv(t, oneServer)
	e.setenv(append([]string{"MCPX_ARTIFACTS_TTL=6s", "MCPX_ARTIFACTS_GC_INTERVAL=200ms"}, env...)...)
	began := time.Now()
	e.run("exec", artifactScript)
	stored := time.Now()
	c := e.socketClient(t)
	get := func(path string) (int, []byte) {
		t.Helper()
		resp, err := c.Get("http://mcpx" + path)
		if err != nil {
			t.Fatal(err)
		}
		defer func() { _ = resp.Body.Close() }()
		b, _ := io.ReadAll(resp.Body)
		return resp.StatusCode, b
	}
	_, b := get("/v1/artifacts")
	var doc struct {
		Artifacts []struct {
			ID string `json:"id"`
		} `json:"artifacts"`
	}
	if err := json.Unmarshal(b, &doc); err != nil || len(doc.Artifacts) != 1 {
		t.Fatalf("one artifact should be listed (%v):\n%s", err, b)
	}
	id := doc.Artifacts[0].ID
	time.Sleep(time.Until(began.Add(unit * 3 / 2)))
	firstRead := time.Now()
	if code, b := get("/v1/artifacts/" + id); code != http.StatusOK {
		t.Fatalf("the first read, inside the retention, should succeed: %d %s", code, b)
	}
	// Past the unrenewed expiry (stored+2 units, plus a sweep), and well
	// inside a renewal from the first read (firstRead+3 units).
	second := firstRead.Add(unit * 3 / 2)
	if expired := stored.Add(2*unit + time.Second); expired.After(second) {
		second = expired
	}
	if !second.Before(firstRead.Add(3 * unit)) {
		t.Fatalf("the store took %v, too long to tell a renewal from an expiry", stored.Sub(began))
	}
	time.Sleep(time.Until(second))
	code, _ := get("/v1/artifacts/" + id)
	return code
}

// TestArtifactsIdleTTLRenewsAReadArtifactFromTheEnvironment is the effect
// probe for artifacts.idleTtl. Under a 6s absolute retention an artifact
// read at 4.5s is gone at 9s by default (no renewal); with idleTtl=9s the
// read renews it, so the 9s read still finds it.
func TestArtifactsIdleTTLRenewsAReadArtifactFromTheEnvironment(t *testing.T) {
	if code := readAfterRenewal(t); code != http.StatusNotFound {
		t.Fatalf("oracle: without renewal the artifact should have expired, got %d", code)
	}
	if code := readAfterRenewal(t, "MCPX_ARTIFACTS_IDLE_TTL=9s"); code != http.StatusOK {
		t.Fatalf("with artifacts.idleTtl=3s the read should have renewed the artifact, got %d", code)
	}
}

// TestArtifactsMaxAgeCapsARenewedArtifactFromTheEnvironment is the
// effect probe for artifacts.maxAge. With idleTtl=9s a read at 4.5s
// renews the artifact past 9s under the default ceiling; a 6s maxAge
// caps the renewal at creation plus 6s, so it is gone at 9s.
func TestArtifactsMaxAgeCapsARenewedArtifactFromTheEnvironment(t *testing.T) {
	if code := readAfterRenewal(t, "MCPX_ARTIFACTS_IDLE_TTL=9s"); code != http.StatusOK {
		t.Fatalf("oracle: under the default ceiling the renewed artifact should survive, got %d", code)
	}
	if code := readAfterRenewal(t, "MCPX_ARTIFACTS_IDLE_TTL=9s", "MCPX_ARTIFACTS_MAX_AGE=6s"); code != http.StatusNotFound {
		t.Fatalf("with artifacts.maxAge=2s the renewal should be capped and the artifact collected, got %d", code)
	}
}

// TestElicitAskTimeoutBoundsTheWaitForAnAnswerFromTheEnvironment is the
// effect probe for elicit.askTimeout. Under policy.write=ask, with an
// answerer present, an unannotated call raises a confirmation and waits.
// Under the default deadline (45s) the question is still pending 3s
// later, and the call ends only when it is declined. With a 1s deadline
// the call gives up by itself, saying nobody answered within 1s.
func TestElicitAskTimeoutBoundsTheWaitForAnAnswerFromTheEnvironment(t *testing.T) {
	asking := func(extra ...string) *env {
		e := newEnv(t, oneServer)
		e.setenv(append([]string{"MCPX_POLICY_WRITE=ask"}, extra...)...)
		e.run("ls")
		beAnswerer(t, e)
		return e
	}

	def := asking()
	done := make(chan string, 1)
	go func() {
		out, _ := def.try("call", "demo.structured", `{}`)
		done <- out
	}()
	time.Sleep(3 * time.Second)
	ids := pendingQuestions(t, def)
	if len(ids) != 1 {
		t.Fatalf("oracle: under the default deadline the confirmation should still be pending, got %v", ids)
	}
	def.run("elicit", "decline", ids[0])
	select {
	case out := <-done:
		if strings.Contains(out, "nobody answered") {
			t.Fatalf("oracle: the call should end on the decline, not a deadline:\n%s", out)
		}
	case <-time.After(30 * time.Second):
		t.Fatal("oracle: the declined call never finished")
	}

	out, err := asking("MCPX_ELICIT_ASK_TIMEOUT=1s").try("call", "demo.structured", `{}`)
	if err == nil {
		t.Fatalf("with elicit.askTimeout=1s the unanswered confirmation should refuse the call; got:\n%s", out)
	}
	if !strings.Contains(out, "nobody answered within 1s") {
		t.Fatalf("the refusal should name the 1s deadline, got:\n%s", out)
	}
}

// TestElicitPendingLimitBoundsTheListingFromTheEnvironment is the effect
// probe for elicit.pendingLimit. Under policy.write=ask, two unannotated
// calls each raise a confirmation that waits up to 5s. While both wait,
// `mcpx elicit list` shows both under the default limit and one under
// elicit.pendingLimit=1.
func TestElicitPendingLimitBoundsTheListingFromTheEnvironment(t *testing.T) {
	listed := func(extra ...string) int {
		e := newEnv(t, oneServer)
		e.setenv(append([]string{"MCPX_POLICY_WRITE=ask", "MCPX_ELICIT_ASK_TIMEOUT=5s"}, extra...)...)
		e.run("ls")
		beAnswerer(t, e)
		done := make(chan struct{}, 2)
		for i := 0; i < 2; i++ {
			go func() {
				_, _ = e.try("call", "demo.structured", `{}`)
				done <- struct{}{}
			}()
		}
		time.Sleep(2 * time.Second)
		n := len(pendingQuestions(t, e))
		for i := 0; i < 2; i++ {
			select {
			case <-done:
			case <-time.After(30 * time.Second):
				t.Fatal("a held call never finished")
			}
		}
		return n
	}
	if n := listed(); n != 2 {
		t.Fatalf("oracle: both waiting confirmations should be listed under the default limit, got %d", n)
	}
	if n := listed("MCPX_ELICIT_PENDING_LIMIT=1"); n != 1 {
		t.Fatalf("with elicit.pendingLimit=1 the listing should hold one question, got %d", n)
	}
}

// upstreamQuestionLife raises one question from an upstream server (the
// ask fixture's need_repo) under env, reads its listed deadline, answers
// it and returns the deadline's distance from the question's creation.
func upstreamQuestionLife(t *testing.T, env ...string) time.Duration {
	t.Helper()
	e := askWith(t, `{"sharing":"shared","scope":"global"}`)
	e.setenv(env...)
	e.run("refresh")
	beAnswerer(t, e)
	c := e.longSocketClient(t)
	done := make(chan struct{})
	go func() {
		sockDo(t, c, http.MethodPost, "/v1/call", map[string]any{"server": "ask", "tool": "need_repo"})
		close(done)
	}()
	ids := waitQuestions(t, e, 1)
	var listed []struct {
		Created   time.Time `json:"created"`
		ExpiresAt time.Time `json:"expiresAt"`
	}
	out := e.run("--json", "elicit", "list")
	if err := json.Unmarshal([]byte(out), &listed); err != nil || len(listed) != 1 {
		t.Fatalf("one question should be listed (%v):\n%s", err, out)
	}
	e.run("elicit", "answer", ids[0], `{"repo":"probe"}`)
	select {
	case <-done:
	case <-time.After(30 * time.Second):
		t.Fatal("the answered call never finished")
	}
	return listed[0].ExpiresAt.Sub(listed[0].Created).Round(time.Second)
}

// TestElicitTTLSetsAnUpstreamQuestionsDeadlineFromTheEnvironment is the
// effect probe for elicit.ttl. A question an upstream server asks lives
// the built-in two minutes by default and seven with elicit.ttl=7m.
func TestElicitTTLSetsAnUpstreamQuestionsDeadlineFromTheEnvironment(t *testing.T) {
	if got := upstreamQuestionLife(t); got != 2*time.Minute {
		t.Fatalf("oracle: an upstream question should live the default 2m, got %v", got)
	}
	if got := upstreamQuestionLife(t, "MCPX_ELICIT_TTL=7m"); got != 7*time.Minute {
		t.Fatalf("with elicit.ttl=7m the question should live 7m, got %v", got)
	}
}

// TestElicitMaxAgeCapsAnUpstreamQuestionsDeadlineFromTheEnvironment is
// the effect probe for elicit.maxAge. Under elicit.ttl=7m the question
// lives seven minutes by default (the 30m ceiling is far off) and one
// with elicit.maxAge=1m.
func TestElicitMaxAgeCapsAnUpstreamQuestionsDeadlineFromTheEnvironment(t *testing.T) {
	if got := upstreamQuestionLife(t, "MCPX_ELICIT_TTL=7m"); got != 7*time.Minute {
		t.Fatalf("oracle: under elicit.ttl=7m the question should live 7m, got %v", got)
	}
	if got := upstreamQuestionLife(t, "MCPX_ELICIT_TTL=7m", "MCPX_ELICIT_MAX_AGE=1m"); got != time.Minute {
		t.Fatalf("with elicit.maxAge=1m the question should live 1m, got %v", got)
	}
}

// exclusiveThree is a demo server leased one exclusive instance per
// session, with no elicit block, so the disambiguation settings come
// from the environment alone.
const exclusiveThree = `{
  "mcpServers": {
    "demo": { "command": "FAKE", "mcpx": { "sharing": "exclusive", "scope": "session", "max": 4 } }
  }
}`

// thirdSessionCall leaves two sessions holding demo instances, then
// makes a call from a third session that names neither. It reports
// whether a question was raised while that call waited, and the call's
// output once it finished unanswered.
func thirdSessionCall(t *testing.T, env ...string) (asked bool, out string) {
	t.Helper()
	e := newEnv(t, exclusiveThree)
	e.setenv(append([]string{"MCPX_ELICIT_ASK_TIMEOUT=3s"}, env...)...)
	e.run("call", "--session", "work", "demo.open", `{"value":"work"}`)
	e.run("call", "--session", "scratch", "demo.open", `{"value":"scratch"}`)
	done := make(chan string, 1)
	go func() {
		o, _ := e.try("call", "--session", "third", "demo.state", "{}")
		done <- o
	}()
	for {
		select {
		case o := <-done:
			return asked, o
		case <-time.After(150 * time.Millisecond):
			if !asked && len(pendingQuestions(t, e)) > 0 {
				asked = true
			}
		case <-time.After(60 * time.Second):
			t.Fatal("the third session's call never finished")
		}
	}
}

// TestElicitDisambiguateAsksWhichInstanceFromTheEnvironment is the
// effect probe for elicit.disambiguate. With two live exclusive
// instances and a caller naming neither, no question is raised by
// default (never); with elicit.disambiguate=ask one is.
func TestElicitDisambiguateAsksWhichInstanceFromTheEnvironment(t *testing.T) {
	if asked, out := thirdSessionCall(t); asked {
		t.Fatalf("oracle: by default no disambiguation question should be raised:\n%s", out)
	}
	if asked, out := thirdSessionCall(t, "MCPX_ELICIT_DISAMBIGUATE=ask"); !asked {
		t.Fatalf("with elicit.disambiguate=ask a question should have been raised:\n%s", out)
	}
}

// TestElicitDisambiguateDefaultPicksTheRecentInstanceFromTheEnvironment
// is the effect probe for elicit.disambiguateDefault. When the
// disambiguation question goes unanswered, the default (new) starts a
// fresh instance that holds neither session's value; with
// elicit.disambiguateDefault=recent the call lands on the most recently
// used live instance. Idle time is counted in whole seconds, so which of
// the two sessions' instances wins a tie is not asserted.
func TestElicitDisambiguateDefaultPicksTheRecentInstanceFromTheEnvironment(t *testing.T) {
	_, out := thirdSessionCall(t, "MCPX_ELICIT_DISAMBIGUATE=ask")
	if strings.Contains(out, "work") || strings.Contains(out, "scratch") {
		t.Fatalf("oracle: unanswered, the default should start a fresh instance:\n%s", out)
	}
	_, out = thirdSessionCall(t, "MCPX_ELICIT_DISAMBIGUATE=ask", "MCPX_ELICIT_DISAMBIGUATE_DEFAULT=recent")
	if !strings.Contains(out, "work") && !strings.Contains(out, "scratch") {
		t.Fatalf("with elicit.disambiguateDefault=recent the call should land on a live instance:\n%s", out)
	}
}

// liveAfterASilentCaller makes one call to a call-scoped demo under a
// fixed caller id that never releases, waits 1.5s with the reaper
// sweeping every 100ms, and returns ls's LIVE column for demo.
func liveAfterASilentCaller(t *testing.T, env ...string) string {
	t.Helper()
	e := newEnv(t, `{"mcpServers":{"demo":{"command":"FAKE","mcpx":{"sharing":"exclusive","scope":"call","max":4}}}}`)
	e.setenv(append([]string{"MCPX_DAEMON_REAP_INTERVAL=100ms"}, env...)...)
	e.run("ls")
	req, err := http.NewRequest(http.MethodPost, "http://mcpx/v1/call",
		strings.NewReader(`{"server":"demo","tool":"echo","args":{"message":"held"}}`))
	if err != nil {
		t.Fatal(err)
	}
	req.Header.Set("X-Mcpx-Call", "silent-caller")
	resp, err := e.socketClient(t).Do(req)
	if err != nil {
		t.Fatal(err)
	}
	_ = resp.Body.Close()
	time.Sleep(1500 * time.Millisecond)
	return lsDemoRow(t, e.run("ls"))[4]
}

// TestDaemonLeaseTTLForgetsASilentCallerFromTheEnvironment is the effect
// probe for daemon.leaseTTL. A caller that never says it has finished
// keeps its call-scoped instance live under the default 30m retention;
// with daemon.leaseTTL=500ms the lease is forgotten and the reaper
// releases the instance.
func TestDaemonLeaseTTLForgetsASilentCallerFromTheEnvironment(t *testing.T) {
	if live := liveAfterASilentCaller(t); live == "0" {
		t.Fatal("oracle: under the default lease retention the silent caller's instance should stay live")
	}
	if live := liveAfterASilentCaller(t, "MCPX_DAEMON_LEASE_TTL=500ms"); live != "0" {
		t.Fatalf("with daemon.leaseTTL=500ms the silent caller's instance should be released, LIVE=%s", live)
	}
}

// autostartTook times the first ls in a fresh environment, which starts
// the daemon and polls until it answers. Warming is off so the time is
// the start and nothing else.
func autostartTook(t *testing.T, env ...string) time.Duration {
	t.Helper()
	e := newEnv(t, oneServer)
	e.setenv(append([]string{"MCPX_DAEMON_WARM=false"}, env...)...)
	start := time.Now()
	e.run("ls")
	return time.Since(start)
}

// TestAutostartPollIntervalPacesTheReadinessPollFromTheEnvironment is
// the effect probe for autostart.pollInterval. A freshly spawned daemon
// never answers the very first ping, so the client waits one interval
// before its next: well under a second by default, at least 4s with
// autostart.pollInterval=4s.
func TestAutostartPollIntervalPacesTheReadinessPollFromTheEnvironment(t *testing.T) {
	fast := autostartTook(t)
	if fast >= 3*time.Second {
		t.Fatalf("oracle: a default autostart should take well under 3s, took %v", fast)
	}
	if slow := autostartTook(t, "MCPX_AUTOSTART_POLL_INTERVAL=4s"); slow < 4*time.Second {
		t.Fatalf("with autostart.pollInterval=4s the autostart should wait at least one 4s interval, took %v", slow)
	}
}

// leaseSavedWithin makes one call under a fixed caller id and reports
// whether the daemon's sessions.json names that caller 1.5s later,
// while the daemon is still running.
func leaseSavedWithin(t *testing.T, env ...string) bool {
	t.Helper()
	e := newEnv(t, oneServer)
	e.setenv(env...)
	e.run("ls")
	req, err := http.NewRequest(http.MethodPost, "http://mcpx/v1/call",
		strings.NewReader(`{"server":"demo","tool":"echo","args":{"message":"saved"}}`))
	if err != nil {
		t.Fatal(err)
	}
	req.Header.Set("X-Mcpx-Call", "periodically-saved-caller")
	resp, err := e.socketClient(t).Do(req)
	if err != nil {
		t.Fatal(err)
	}
	_ = resp.Body.Close()
	time.Sleep(1500 * time.Millisecond)
	b, _ := os.ReadFile(filepath.Join(e.dir, "state", "sessions.json"))
	return strings.Contains(string(b), "periodically-saved-caller")
}

// TestDaemonSaveIntervalWritesTheLeaseTableFromTheEnvironment is the
// effect probe for daemon.saveInterval. A caller's lease reaches
// sessions.json only when the daemon saves: not within 1.5s under the
// default five-minute interval, and within it at
// daemon.saveInterval=200ms.
func TestDaemonSaveIntervalWritesTheLeaseTableFromTheEnvironment(t *testing.T) {
	if leaseSavedWithin(t) {
		t.Fatal("oracle: under the default save interval the lease should not be on disk yet")
	}
	if !leaseSavedWithin(t, "MCPX_DAEMON_SAVE_INTERVAL=200ms") {
		t.Fatal("with daemon.saveInterval=200ms the lease should have been saved to sessions.json")
	}
}

// TestDaemonPortBindsTheChosenPortFromTheEnvironment is the effect probe
// for daemon.port. By default the daemon binds an ephemeral port; with
// daemon.port set to a port the test just found free, the daemon
// reports that exact endpoint, and it answers there.
func TestDaemonPortBindsTheChosenPortFromTheEnvironment(t *testing.T) {
	l, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		t.Fatal(err)
	}
	port := strconv.Itoa(l.Addr().(*net.TCPAddr).Port)
	_ = l.Close()
	want := "http://127.0.0.1:" + port

	def := newEnv(t, oneServer)
	def.run("ls")
	if got := endpointOf(t, def); got == want || !strings.HasPrefix(got, "http://127.0.0.1:") {
		t.Fatalf("oracle: by default the daemon should bind some other loopback port, got %s", got)
	}

	e := newEnv(t, oneServer)
	e.setenv("MCPX_DAEMON_PORT=" + port)
	e.run("ls")
	if got := endpointOf(t, e); got != want {
		t.Fatalf("with daemon.port=%s the daemon should report %s, got %s", port, want, got)
	}
	resp, err := http.Get(want + "/v1/health")
	if err != nil {
		t.Fatalf("the chosen port should answer: %v", err)
	}
	defer func() { _ = resp.Body.Close() }()
	if resp.StatusCode != http.StatusOK {
		t.Fatalf("the chosen port answered %d, want 200", resp.StatusCode)
	}
}

// TestDaemonIdleExitStopsAnIdleDaemonFromTheEnvironment is the effect
// probe for daemon.idleExit, the daemon's own idle bound (the one a
// daemon started without --idle-exit uses). autostart.idleExit is
// zeroed so the auto-started daemon gets no flag, and instances are
// reaped after 200ms. With daemon.idleExit at its 0s default the daemon
// never exits, so ls with autostart off reaches it 4s later; with
// daemon.idleExit=1s it has exited, and the same ls fails.
func TestDaemonIdleExitStopsAnIdleDaemonFromTheEnvironment(t *testing.T) {
	stillThere := func(extra ...string) bool {
		e := newEnv(t, oneServer)
		e.setenv(append([]string{"MCPX_AUTOSTART_IDLE_EXIT=0s",
			"MCPX_POOL_IDLE_TIMEOUT=200ms", "MCPX_DAEMON_REAP_INTERVAL=100ms"}, extra...)...)
		e.run("ls")
		time.Sleep(4 * time.Second)
		e.setenv("MCPX_DAEMON_AUTOSTART=false")
		_, err := e.try("ls")
		return err == nil
	}
	if !stillThere() {
		t.Fatalf("oracle: with no idle bound the daemon should still be running")
	}
	if stillThere("MCPX_DAEMON_IDLE_EXIT=1s") {
		t.Fatalf("with daemon.idleExit=1s the idle daemon should have exited")
	}
}

// liveAfterAnExec runs a script that calls a session-scoped exclusive
// demo once, then returns ls's LIVE column for demo.
func liveAfterAnExec(t *testing.T, env ...string) string {
	t.Helper()
	e := newEnv(t, `{"mcpServers":{"demo":{"command":"FAKE","mcpx":{"sharing":"exclusive","scope":"session","max":4}}}}`)
	e.setenv(append([]string{"MCPX_DAEMON_WARM=false"}, env...)...)
	e.run("exec", `await demo.echo({ message: "x" });`)
	return lsDemoRow(t, e.run("ls"))[4]
}

// TestSessionReleaseTimeoutBoundsTheEndOfRunReleaseFromTheEnvironment is
// the effect probe for session.releaseTimeout. When a script ends, exec
// asks the daemon to release the instances its ephemeral session leased.
// Within the default bound the release lands and nothing is left live;
// with session.releaseTimeout=1ns the request cannot complete, so the
// script's instance is still running afterwards.
func TestSessionReleaseTimeoutBoundsTheEndOfRunReleaseFromTheEnvironment(t *testing.T) {
	if live := liveAfterAnExec(t); live != "0" {
		t.Fatalf("oracle: by default the script's instance should be released when it ends, LIVE=%s", live)
	}
	if live := liveAfterAnExec(t, "MCPX_SESSION_RELEASE_TIMEOUT=1ns"); live != "1" {
		t.Fatalf("with session.releaseTimeout=1ns the release should not land, leaving one instance, LIVE=%s", live)
	}
}

// fakeModel is an OpenAI-compatible chat endpoint that records what it
// was sent and answers with advice naming itself, after delay.
type fakeModel struct {
	URL  string
	tag  string
	mu   sync.Mutex
	reqs []fakeModelRequest
}

type fakeModelRequest struct {
	Auth      string
	Size      int    // bytes in the request body
	Model     string `json:"model"`
	MaxTokens int    `json:"max_tokens"`
}

func newFakeModel(t *testing.T, tag string, delay time.Duration) *fakeModel {
	t.Helper()
	m := &fakeModel{tag: tag}
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		var req fakeModelRequest
		raw, _ := io.ReadAll(r.Body)
		_ = json.Unmarshal(raw, &req)
		req.Auth = r.Header.Get("Authorization")
		req.Size = len(raw)
		m.mu.Lock()
		m.reqs = append(m.reqs, req)
		m.mu.Unlock()
		select {
		case <-r.Context().Done():
			return
		case <-time.After(delay):
		}
		w.Header().Set("Content-Type", "application/json")
		_, _ = io.WriteString(w, `{"id":"probe","model":"fake-model","choices":[{"index":0,`+
			`"message":{"role":"assistant","content":"advice from `+tag+`"},"finish_reason":"stop"}]}`)
	}))
	t.Cleanup(srv.Close)
	m.URL = srv.URL
	return m
}

// last is the most recent request the endpoint saw; the zero value if none.
func (m *fakeModel) last() fakeModelRequest {
	m.mu.Lock()
	defer m.mu.Unlock()
	if len(m.reqs) == 0 {
		return fakeModelRequest{}
	}
	return m.reqs[len(m.reqs)-1]
}

// promptHookFrame runs a one-line script under a config whose one
// script hook is an onSuccess prompt, with hooks allowed to run and env
// on top, and returns that hook's frame from `exec --json`.
func promptHookFrame(t *testing.T, env ...string) map[string]any {
	t.Helper()
	e := newEnv(t, `{"mcpServers":{"demo":{"command":"FAKE"}},`+
		`"script":{"hooks":[{"phase":"onSuccess","prompt":"review the run"}]}}`)
	e.setenv(append([]string{"MCPX_HOOKS_AUTONOMY=run", "FAKE_MODEL_KEY=probe-key"}, env...)...)
	out := e.run("--json", "exec", `console.log("x")`)
	var doc struct {
		Hooks []map[string]any `json:"hooks"`
	}
	if err := json.Unmarshal([]byte(out), &doc); err != nil || len(doc.Hooks) != 1 {
		t.Fatalf("exec --json should report one hook frame (%v):\n%s", err, out)
	}
	return doc.Hooks[0]
}

// modelAt is the environment that points the prompt hook at m through
// model.provider, model.baseURL and model.default.
func modelAt(m *fakeModel) []string {
	return []string{"MCPX_MODEL_PROVIDER=openai", "MCPX_MODEL_BASE_URL=" + m.URL,
		"MCPX_MODEL_DEFAULT=openai/fake-model"}
}

// TestModelProviderSelectsTheBackendFromTheEnvironment is the effect
// probe for model.provider. At the default (none) a prompt hook reports
// that no provider is configured and nothing is called; with
// model.provider=openai the hook's advice comes from the endpoint.
func TestModelProviderSelectsTheBackendFromTheEnvironment(t *testing.T) {
	a := newFakeModel(t, "A", 0)
	if f := promptHookFrame(t, "MCPX_MODEL_BASE_URL="+a.URL, "MCPX_MODEL_DEFAULT=openai/fake-model"); !strings.Contains(fmt.Sprint(f["error"]), "no model provider is configured") {
		t.Fatalf("oracle: with model.provider at its default the hook should have no provider: %v", f)
	}
	if f := promptHookFrame(t, modelAt(a)...); f["advice"] != "advice from A" {
		t.Fatalf("with model.provider=openai the hook should be advised by the endpoint: %v", f)
	}
}

// TestModelBaseURLPointsTheProviderAtAnEndpointFromTheEnvironment is the
// effect probe for model.baseURL: the same hook is answered by whichever
// endpoint model.baseURL names.
func TestModelBaseURLPointsTheProviderAtAnEndpointFromTheEnvironment(t *testing.T) {
	a, b := newFakeModel(t, "A", 0), newFakeModel(t, "B", 0)
	if f := promptHookFrame(t, modelAt(a)...); f["advice"] != "advice from A" {
		t.Fatalf("oracle: model.baseURL at A should be answered by A: %v", f)
	}
	if f := promptHookFrame(t, append(modelAt(a), "MCPX_MODEL_BASE_URL="+b.URL)...); f["advice"] != "advice from B" {
		t.Fatalf("model.baseURL at B should be answered by B: %v", f)
	}
}

// TestModelDefaultNamesTheModelFromTheEnvironment is the effect probe for
// model.default. With no default and no model on the hook, the provider
// has no model to ask for; with model.default=openai/fake-model the
// request names fake-model.
func TestModelDefaultNamesTheModelFromTheEnvironment(t *testing.T) {
	a := newFakeModel(t, "A", 0)
	if f := promptHookFrame(t, "MCPX_MODEL_PROVIDER=openai", "MCPX_MODEL_BASE_URL="+a.URL); !strings.Contains(fmt.Sprint(f["error"]), "no model named") {
		t.Fatalf("oracle: with no model.default the hook should have no model to ask for: %v", f)
	}
	if promptHookFrame(t, modelAt(a)...); a.last().Model != "fake-model" {
		t.Fatalf("with model.default=openai/fake-model the request should name fake-model: %+v", a.last())
	}
}

// TestModelAPIKeyEnvNamesTheKeyVariableFromTheEnvironment is the effect
// probe for model.apiKeyEnv. mcpx reads no key variable unless one is
// named, so by default the request carries no Authorization; naming
// FAKE_MODEL_KEY sends its value as the bearer token.
func TestModelAPIKeyEnvNamesTheKeyVariableFromTheEnvironment(t *testing.T) {
	a := newFakeModel(t, "A", 0)
	if promptHookFrame(t, modelAt(a)...); a.last().Auth != "" {
		t.Fatalf("oracle: with no model.apiKeyEnv the request should carry no key: %q", a.last().Auth)
	}
	if promptHookFrame(t, append(modelAt(a), "MCPX_MODEL_API_KEY_ENV=FAKE_MODEL_KEY")...); a.last().Auth != "Bearer probe-key" {
		t.Fatalf("with model.apiKeyEnv=FAKE_MODEL_KEY the request should carry its value: %q", a.last().Auth)
	}
}

// TestModelBudgetCapsAHooksTokensFromTheEnvironment is the effect probe
// for model.budget: a hook asks for the default token cap, and for no
// more than model.budget=7 when it is set.
func TestModelBudgetCapsAHooksTokensFromTheEnvironment(t *testing.T) {
	a := newFakeModel(t, "A", 0)
	promptHookFrame(t, modelAt(a)...)
	if def := a.last().MaxTokens; def <= 7 {
		t.Fatalf("oracle: the default cap should be well above 7 tokens, got %d", def)
	}
	if promptHookFrame(t, append(modelAt(a), "MCPX_MODEL_BUDGET=7")...); a.last().MaxTokens != 7 {
		t.Fatalf("with model.budget=7 the request should ask for 7 tokens: %+v", a.last())
	}
}

// TestPromptMaxTokensSetsAHooksTokenCapFromTheEnvironment is the effect
// probe for prompt.maxTokens: the request's max_tokens follows it.
func TestPromptMaxTokensSetsAHooksTokenCapFromTheEnvironment(t *testing.T) {
	a := newFakeModel(t, "A", 0)
	promptHookFrame(t, modelAt(a)...)
	if def := a.last().MaxTokens; def == 11 || def == 0 {
		t.Fatalf("oracle: the default cap should be set and not 11, got %d", def)
	}
	if promptHookFrame(t, append(modelAt(a), "MCPX_PROMPT_MAX_TOKENS=11")...); a.last().MaxTokens != 11 {
		t.Fatalf("with prompt.maxTokens=11 the request should ask for 11 tokens: %+v", a.last())
	}
}

// TestModelProvidersRoutesAProviderToItsEndpointFromTheEnvironment is the
// effect probe for model.providers: with model.baseURL at A, a
// per-provider entry giving openai the base URL of B routes the hook to B.
func TestModelProvidersRoutesAProviderToItsEndpointFromTheEnvironment(t *testing.T) {
	a, b := newFakeModel(t, "A", 0), newFakeModel(t, "B", 0)
	if f := promptHookFrame(t, modelAt(a)...); f["advice"] != "advice from A" {
		t.Fatalf("oracle: without model.providers the hook should go to A: %v", f)
	}
	if f := promptHookFrame(t, append(modelAt(a), `MCPX_MODEL_PROVIDERS={"openai":{"baseURL":"`+b.URL+`"}}`)...); f["advice"] != "advice from B" {
		t.Fatalf("with model.providers giving openai B's URL the hook should go to B: %v", f)
	}
}

// TestModelProfilesResolvesANamedChainFromTheEnvironment is the effect
// probe for model.profiles: with model.default naming a profile whose
// chain is openai/other-model, the request names other-model.
func TestModelProfilesResolvesANamedChainFromTheEnvironment(t *testing.T) {
	a := newFakeModel(t, "A", 0)
	if promptHookFrame(t, modelAt(a)...); a.last().Model != "fake-model" {
		t.Fatalf("oracle: without a profile the request should name fake-model: %+v", a.last())
	}
	if promptHookFrame(t, append(modelAt(a), `MCPX_MODEL_PROFILES={"quick":{"models":["openai/other-model"]}}`,
		"MCPX_MODEL_DEFAULT=quick")...); a.last().Model != "other-model" {
		t.Fatalf("with model.default naming profile quick the request should name other-model: %+v", a.last())
	}
}

// TestModelTimeoutBoundsAGenerationFromTheEnvironment is the effect probe
// for model.timeout: an endpoint that takes 700ms still advises the hook
// under the default bound, and is cut off at model.timeout=100ms.
func TestModelTimeoutBoundsAGenerationFromTheEnvironment(t *testing.T) {
	slow := newFakeModel(t, "slow", 700*time.Millisecond)
	if f := promptHookFrame(t, modelAt(slow)...); f["advice"] != "advice from slow" {
		t.Fatalf("oracle: under the default model.timeout a 700ms endpoint should answer: %v", f)
	}
	if f := promptHookFrame(t, append(modelAt(slow), "MCPX_MODEL_TIMEOUT=100ms")...); !strings.Contains(fmt.Sprint(f["error"]), "deadline exceeded") {
		t.Fatalf("with model.timeout=100ms the 700ms endpoint should be cut off: %v", f)
	}
}

// TestSamplingAnswerLocallyAnswersAnUpstreamFromTheEnvironment is the
// effect probe for sampling.answerLocally. The ask fixture's need_model
// tool sends sampling/createMessage upstream of mcpx. By default mcpx
// relays it to its caller, and the fake model endpoint is never asked;
// with sampling.answerLocally=true mcpx answers from its configured
// model, so the endpoint is asked and its text reaches the tool's result.
func TestSamplingAnswerLocallyAnswersAnUpstreamFromTheEnvironment(t *testing.T) {
	a := newFakeModel(t, "A", 0)
	call := func(env ...string) string {
		t.Helper()
		e := askWith(t, `{"sharing":"shared","scope":"global","callTimeout":"10s"}`)
		e.setenv(append(modelAt(a), env...)...)
		e.run("refresh")
		out, _ := e.try("call", "ask.need_model", "{}")
		return out
	}
	if out := call(); strings.Contains(out, "advice from A") || len(a.reqs) != 0 {
		t.Fatalf("oracle: by default the sampling request should be relayed, not answered by the model:\n%s", out)
	}
	if out := call("MCPX_SAMPLING_ANSWER_LOCALLY=true"); !strings.Contains(out, "advice from A") {
		t.Fatalf("with sampling.answerLocally=true the model's answer should reach the tool:\n%s", out)
	}
}

// twoSamplings calls the ask fixture's need_model tool twice on one shared
// upstream session, with mcpx answering each sampling request from a fake
// model, and returns the second call's output.
func twoSamplings(t *testing.T, env ...string) string {
	t.Helper()
	a := newFakeModel(t, "A", 0)
	e := askWith(t, `{"sharing":"shared","scope":"global","callTimeout":"10s"}`)
	e.setenv(append(append(modelAt(a), "MCPX_SAMPLING_ANSWER_LOCALLY=true"), env...)...)
	e.run("refresh")
	if out, _ := e.try("call", "ask.need_model", "{}"); !strings.Contains(out, "advice from A") {
		t.Fatalf("the first sampling request should be answered by the model:\n%s", out)
	}
	out, _ := e.try("call", "ask.need_model", "{}")
	return out
}

// TestSamplingPerMinuteRefusesAServersSecondSampling is the effect probe for
// proto.samplingPerMinute. Oracle: by default (60) a second sampling
// request in the same minute is answered; at 1 it fails with an error
// naming the setting.
func TestSamplingPerMinuteRefusesAServersSecondSampling(t *testing.T) {
	if out := twoSamplings(t); !strings.Contains(out, "advice from A") {
		t.Fatalf("oracle: by default the second sampling request should be answered:\n%s", out)
	}
	if out := twoSamplings(t, "MCPX_PROTO_SAMPLING_PER_MINUTE=1"); !strings.Contains(out, "proto.samplingPerMinute") {
		t.Fatalf("with proto.samplingPerMinute=1 the second request should be refused by name:\n%s", out)
	}
}

// TestSamplingSessionPerMinuteRefusesASessionsSecondSampling is the effect
// probe for proto.samplingSessionPerMinute: with the server-wide bound left
// at its default, a per-session bound of 1 refuses the session's second
// sampling request with an error naming the per-session setting.
func TestSamplingSessionPerMinuteRefusesASessionsSecondSampling(t *testing.T) {
	if out := twoSamplings(t); strings.Contains(out, "proto.samplingSessionPerMinute") {
		t.Fatalf("oracle: by default the session bound should not bite:\n%s", out)
	}
	if out := twoSamplings(t, "MCPX_PROTO_SAMPLING_SESSION_PER_MINUTE=1"); !strings.Contains(out, "proto.samplingSessionPerMinute") {
		t.Fatalf("with proto.samplingSessionPerMinute=1 the second request should be refused by name:\n%s", out)
	}
}

// TestElicitationPerMinuteRefusesAServersSecondQuestion is the effect probe
// for proto.elicitationPerMinute. The ask fixture's need_repo_twice tool
// asks two questions in one call. Oracle: by default the second question
// reaches the broker after the first is answered; at 1 it never does, and
// the call fails with an error naming the setting.
func TestElicitationPerMinuteRefusesAServersSecondQuestion(t *testing.T) {
	run := func(env ...string) (string, bool) {
		t.Helper()
		e := askWith(t, `{"sharing":"shared","scope":"global"}`)
		e.setenv(env...)
		e.run("refresh")
		beAnswerer(t, e)
		c := e.longSocketClient(t)
		done := make(chan map[string]any, 1)
		go func() {
			_, body := sockDo(t, c, http.MethodPost, "/v1/call", map[string]any{"server": "ask", "tool": "need_repo_twice"})
			done <- body
		}()
		ids := waitQuestions(t, e, 1)
		e.run("elicit", "answer", ids[0], `{"repo":"one"}`)
		deadline := time.After(30 * time.Second)
		for {
			select {
			case body := <-done:
				b, _ := json.Marshal(body)
				return string(b), false
			case <-deadline:
				t.Fatal("the two-question call neither asked again nor finished")
			case <-time.After(100 * time.Millisecond):
			}
			if ids := pendingQuestions(t, e); len(ids) == 1 {
				e.run("elicit", "answer", ids[0], `{"repo":"two"}`)
				body := <-done
				b, _ := json.Marshal(body)
				return string(b), true
			}
		}
	}
	if out, asked := run(); !asked || !strings.Contains(out, "using one and two") {
		t.Fatalf("oracle: by default the second question should be asked and answered (asked=%v):\n%s", asked, out)
	}
	if out, asked := run("MCPX_PROTO_ELICITATION_PER_MINUTE=1"); asked || !strings.Contains(out, "proto.elicitationPerMinute") {
		t.Fatalf("with proto.elicitationPerMinute=1 the second question should be refused by name (asked=%v):\n%s", asked, out)
	}
}

// fakeEmbeddings is an OpenAI-compatible /embeddings endpoint that answers
// every request with a unit vector of the index's dimension and records
// the model and Authorization header it was sent.
type fakeEmbeddings struct {
	URL    string
	mu     sync.Mutex
	models []string
	auths  []string
}

func newFakeEmbeddings(t *testing.T) *fakeEmbeddings {
	t.Helper()
	f := &fakeEmbeddings{}
	vec := make([]string, 384)
	for i := range vec {
		vec[i] = "1"
	}
	body := `{"data":[{"embedding":[` + strings.Join(vec, ",") + `]}]}`
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		var req struct {
			Model string `json:"model"`
		}
		_ = json.NewDecoder(r.Body).Decode(&req)
		f.mu.Lock()
		f.models = append(f.models, req.Model)
		f.auths = append(f.auths, r.Header.Get("Authorization"))
		f.mu.Unlock()
		w.Header().Set("Content-Type", "application/json")
		_, _ = io.WriteString(w, body)
	}))
	t.Cleanup(srv.Close)
	f.URL = srv.URL
	return f
}

// seen reports how many requests the endpoint saw, and the last model and
// Authorization header among them.
func (f *fakeEmbeddings) seen() (n int, model, auth string) {
	f.mu.Lock()
	defer f.mu.Unlock()
	if n = len(f.models); n > 0 {
		model, auth = f.models[n-1], f.auths[n-1]
	}
	return n, model, auth
}

// embeddingsSearch runs one `mcpx search` against the demo server with env
// on top and returns what the fake embeddings endpoint saw.
func embeddingsSearch(t *testing.T, f *fakeEmbeddings, args []string, env ...string) (int, string, string) {
	t.Helper()
	e := newEnv(t, oneServer)
	e.setenv(env...)
	e.run("refresh")
	e.run(append([]string{"search"}, args...)...)
	return f.seen()
}

// TestSearchSemanticRanksWithTheConfiguredEmbedder is the effect probe for
// search.semantic. Oracle: a plain search never asks the embeddings
// endpoint; with search.semantic=true the same search does.
func TestSearchSemanticRanksWithTheConfiguredEmbedder(t *testing.T) {
	f := newFakeEmbeddings(t)
	if n, _, _ := embeddingsSearch(t, f, []string{"echo"}, "MCPX_EMBEDDINGS_URL="+f.URL); n != 0 {
		t.Fatalf("oracle: a keyword search should not ask the embeddings endpoint (asked %d times)", n)
	}
	if n, _, _ := embeddingsSearch(t, f, []string{"echo"}, "MCPX_EMBEDDINGS_URL="+f.URL, "MCPX_SEARCH_SEMANTIC=true"); n == 0 {
		t.Fatal("with search.semantic=true the search should ask the embeddings endpoint")
	}
}

// TestEmbeddingsURLSendsSemanticSearchToTheEndpoint is the effect probe for
// embeddings.url: a semantic search asks the endpoint only when its URL is
// configured.
func TestEmbeddingsURLSendsSemanticSearchToTheEndpoint(t *testing.T) {
	f := newFakeEmbeddings(t)
	if n, _, _ := embeddingsSearch(t, f, []string{"--semantic", "echo"}); n != 0 {
		t.Fatalf("oracle: with no embeddings.url nothing should reach the endpoint (asked %d times)", n)
	}
	if n, _, _ := embeddingsSearch(t, f, []string{"--semantic", "echo"}, "MCPX_EMBEDDINGS_URL="+f.URL); n == 0 {
		t.Fatal("with embeddings.url set a semantic search should ask the endpoint")
	}
}

// TestEmbeddingsAPIKeyIsSentAsABearerToken is the effect probe for
// embeddings.apiKey: the endpoint sees no Authorization header by default,
// and a bearer token carrying the key when it is set.
func TestEmbeddingsAPIKeyIsSentAsABearerToken(t *testing.T) {
	f := newFakeEmbeddings(t)
	if n, _, auth := embeddingsSearch(t, f, []string{"--semantic", "echo"}, "MCPX_EMBEDDINGS_URL="+f.URL); n == 0 || auth != "" {
		t.Fatalf("oracle: with no key the endpoint should be asked without Authorization (n=%d auth=%q)", n, auth)
	}
	if _, _, auth := embeddingsSearch(t, f, []string{"--semantic", "echo"}, "MCPX_EMBEDDINGS_URL="+f.URL, "MCPX_EMBEDDINGS_API_KEY=probe-key"); auth != "Bearer probe-key" {
		t.Fatalf("with embeddings.apiKey set the endpoint should see the bearer token, saw %q", auth)
	}
}

// TestEmbeddingsModelNamesTheRequestedModel is the effect probe for
// embeddings.model: the request names text-embedding-3-small by default and
// the configured model when it is set.
func TestEmbeddingsModelNamesTheRequestedModel(t *testing.T) {
	f := newFakeEmbeddings(t)
	if _, model, _ := embeddingsSearch(t, f, []string{"--semantic", "echo"}, "MCPX_EMBEDDINGS_URL="+f.URL); model != "text-embedding-3-small" {
		t.Fatalf("oracle: the default request should name text-embedding-3-small, saw %q", model)
	}
	if _, model, _ := embeddingsSearch(t, f, []string{"--semantic", "echo"}, "MCPX_EMBEDDINGS_URL="+f.URL, "MCPX_EMBEDDINGS_MODEL=probe-model"); model != "probe-model" {
		t.Fatalf("with embeddings.model set the request should name it, saw %q", model)
	}
}

// TestEmbeddingsBackendLocalSkipsTheRemoteEndpoint is the effect probe for
// embeddings.backend. Oracle: by default (auto) a configured remote is
// asked; with backend=local the same search never reaches it.
func TestEmbeddingsBackendLocalSkipsTheRemoteEndpoint(t *testing.T) {
	f := newFakeEmbeddings(t)
	if n, _, _ := embeddingsSearch(t, f, []string{"--semantic", "echo"}, "MCPX_EMBEDDINGS_URL="+f.URL); n == 0 {
		t.Fatal("oracle: the auto backend should ask a configured remote endpoint")
	}
	before, _, _ := f.seen()
	if n, _, _ := embeddingsSearch(t, f, []string{"--semantic", "echo"}, "MCPX_EMBEDDINGS_URL="+f.URL, "MCPX_EMBEDDINGS_BACKEND=local"); n != before {
		t.Fatalf("with embeddings.backend=local the remote endpoint should not be asked (%d -> %d)", before, n)
	}
}

// unmatchedPrompt asks the daemon to resolve a prompt no recipe matches,
// with env on top, and returns the resolution's JSON.
func unmatchedPrompt(t *testing.T, env ...string) string {
	t.Helper()
	e := newEnv(t, oneServer)
	e.setenv(env...)
	e.run("ls")
	out, _ := e.try("--json", "prompt", "compose a haiku about the moon")
	return out
}

// TestPromptSampleAskLetsAnUnmatchedPromptSeekAModel is the effect probe for
// prompt.sample. Oracle: by default (never) an unmatched prompt says
// generating is off and names the setting; with prompt.sample=ask it goes
// on to look for a model provider instead.
func TestPromptSampleAskLetsAnUnmatchedPromptSeekAModel(t *testing.T) {
	if out := unmatchedPrompt(t); !strings.Contains(out, "prompt.sample is never") {
		t.Fatalf("oracle: by default generating should be off by prompt.sample:\n%s", out)
	}
	if out := unmatchedPrompt(t, "MCPX_PROMPT_SAMPLE=ask"); strings.Contains(out, "prompt.sample is never") || strings.Contains(out, "generating one is off") {
		t.Fatalf("with prompt.sample=ask generating should no longer be off:\n%s", out)
	}
}

// TestPromptCatalogBudgetBoundsTheCatalogSentToTheModel is the effect probe
// for prompt.catalogBudget. An unmatched prompt with a model configured
// sends the model a slice of the catalog; a budget of one token sends a
// smaller request than the default.
func TestPromptCatalogBudgetBoundsTheCatalogSentToTheModel(t *testing.T) {
	size := func(env ...string) int {
		t.Helper()
		m := newFakeModel(t, "A", 0)
		unmatchedPrompt(t, append(modelAt(m), env...)...)
		n := m.last().Size
		if n == 0 {
			t.Fatal("the unmatched prompt should have asked the model")
		}
		return n
	}
	def, tiny := size(), size("MCPX_PROMPT_CATALOG_BUDGET=1")
	if tiny >= def {
		t.Fatalf("prompt.catalogBudget=1 should send the model less than the default (%d bytes vs %d)", tiny, def)
	}
}

// samplingQuestionLife sends an unmatched prompt to a daemon whose model
// provider is sampling through the caller, with model.timeout unset and
// env on top, and returns how long the sampling question it raises lives.
func samplingQuestionLife(t *testing.T, env ...string) time.Duration {
	t.Helper()
	e := newEnv(t, oneServer)
	e.setenv(append([]string{"MCPX_MODEL_PROVIDER=sampling", "MCPX_MODEL_TIMEOUT=0s"}, env...)...)
	e.run("ls")
	beAnswerer(t, e)
	c := e.longSocketClient(t)
	// The intent call waits on the question; it is left to be cut off
	// when the test's daemon stops, so it reports nothing to t.
	go func() {
		resp, err := c.Post("http://mcpx/v1/intent", "application/json",
			strings.NewReader(`{"prompt":"compose a haiku about the moon"}`))
		if err == nil {
			_ = resp.Body.Close()
		}
	}()
	waitQuestions(t, e, 1)
	var listed []struct {
		Created   time.Time `json:"created"`
		ExpiresAt time.Time `json:"expiresAt"`
	}
	out := e.run("--json", "elicit", "list")
	if err := json.Unmarshal([]byte(out), &listed); err != nil || len(listed) != 1 {
		t.Fatalf("one sampling question should be listed (%v):\n%s", err, out)
	}
	return listed[0].ExpiresAt.Sub(listed[0].Created)
}

// TestPromptSampleTimeoutBoundsASamplingQuestion is the effect probe for
// prompt.sampleTimeout: with model.timeout unset, the sampling question a
// generation raises lives for prompt.sampleTimeout, so setting it to an
// hour moves the question's expiry from the default to an hour out.
func TestPromptSampleTimeoutBoundsASamplingQuestion(t *testing.T) {
	def := samplingQuestionLife(t)
	if def <= 0 || def > 30*time.Minute {
		t.Fatalf("oracle: by default the question should live for the built-in sample timeout, lived %v", def)
	}
	got := samplingQuestionLife(t, "MCPX_PROMPT_SAMPLE_TIMEOUT=1h")
	if got < 59*time.Minute || got > 61*time.Minute {
		t.Fatalf("with prompt.sampleTimeout=1h the question should live an hour, lived %v (default %v)", got, def)
	}
}

// TestDaemonRootsAnswersAnUpstreamRootsRequest is the effect probe for
// daemon.roots. The ask fixture's need_roots tool sends roots/list; a plain
// call has no host to relay it to, so mcpx answers from its own configured
// roots. Oracle: by default no roots are exposed; with daemon.roots set the
// configured directory comes back as a file:// URI.
func TestDaemonRootsAnswersAnUpstreamRootsRequest(t *testing.T) {
	root := t.TempDir()
	call := func(env ...string) string {
		t.Helper()
		e := askWith(t, `{"sharing":"shared","scope":"global","callTimeout":"10s"}`)
		e.setenv(env...)
		e.run("refresh")
		out, _ := e.try("call", "ask.need_roots", "{}")
		return out
	}
	if out := call(); !strings.Contains(out, "roots") || strings.Contains(out, "file://") {
		t.Fatalf("oracle: by default the roots answer should list no roots:\n%s", out)
	}
	if out := call("MCPX_DAEMON_ROOTS=" + root); !strings.Contains(out, "file://"+root) {
		t.Fatalf("with daemon.roots=%s the roots answer should list it:\n%s", root, out)
	}
}

// napRecipe sleeps three seconds and then says so.
const napRecipe = `// Wait a moment, then report.
await new Promise((resolve) => setTimeout(resolve, 3000));
console.log("woke after the nap");
`

// TestPromptRunTimeoutBoundsARecipeRun is the effect probe for
// prompt.runTimeout. Oracle: under the default bound a recipe that sleeps
// three seconds finishes; with prompt.runTimeout=1s the run is cut off
// and its result says it timed out after 1s.
func TestPromptRunTimeoutBoundsARecipeRun(t *testing.T) {
	run := func(env ...string) string {
		t.Helper()
		e := newEnv(t, oneServer)
		writeRecipe(t, e, "nap", napRecipe)
		e.setenv(env...)
		e.run("ls")
		out, _ := e.try("--json", "recipes", "run", "nap", "--autonomy", "run")
		return out
	}
	if out := run(); strings.Contains(out, "timed out") || !strings.Contains(out, `"stdout"`) {
		t.Fatalf("oracle: under the default bound the recipe should finish:\n%s", out)
	}
	if out := run("MCPX_PROMPT_RUN_TIMEOUT=1s"); !strings.Contains(out, "script timed out after 1s") {
		t.Fatalf("with prompt.runTimeout=1s the recipe should be cut off:\n%s", out)
	}
}

// TestScriptImportsOnDynamicDecidesAComputedImport is the effect probe for
// script.importsOnDynamic. A script whose import() specifier is computed
// cannot be resolved statically. Oracle: by default (warn) the run names
// the computed import and goes on; fail stops the run before it starts;
// ignore runs it without a word.
func TestScriptImportsOnDynamicDecidesAComputedImport(t *testing.T) {
	const script = `const name = ["no", "such"].join("-"); try { await import(name) } catch { } console.log("ran on")`
	run := func(env ...string) (string, error) {
		t.Helper()
		e := newEnv(t, oneServer)
		e.setenv(env...)
		e.run("ls")
		return e.try("exec", script)
	}
	if out, err := run(); err != nil || !strings.Contains(out, "computed import(name)") || !strings.Contains(out, "ran on") {
		t.Fatalf("oracle: by default the computed import should be named and the run go on (err=%v):\n%s", err, out)
	}
	if out, err := run("MCPX_SCRIPT_IMPORTS_ON_DYNAMIC=fail"); err == nil || strings.Contains(out, "ran on") || !strings.Contains(out, "computed import(name)") {
		t.Fatalf("with script.importsOnDynamic=fail the run should stop before it starts (err=%v):\n%s", err, out)
	}
	if out, err := run("MCPX_SCRIPT_IMPORTS_ON_DYNAMIC=ignore"); err != nil || strings.Contains(out, "computed import") || !strings.Contains(out, "ran on") {
		t.Fatalf("with script.importsOnDynamic=ignore the run should go on silently (err=%v):\n%s", err, out)
	}
}

// TestScriptImportsOffSkipsTheImportPipeline is the effect probe for
// script.imports. Oracle: by default (report) the import pipeline names a
// computed import() before the run; with script.imports=off the pipeline
// does not run, so the same script runs with no word about its imports.
func TestScriptImportsOffSkipsTheImportPipeline(t *testing.T) {
	const script = `const name = ["no", "such"].join("-"); try { await import(name) } catch { } console.log("ran on")`
	run := func(env ...string) string {
		t.Helper()
		e := newEnv(t, oneServer)
		e.setenv(env...)
		e.run("ls")
		return e.run("exec", script)
	}
	if out := run(); !strings.Contains(out, "computed import(name)") {
		t.Fatalf("oracle: by default the import pipeline should name the computed import:\n%s", out)
	}
	if out := run("MCPX_SCRIPT_IMPORTS=off"); strings.Contains(out, "computed import") || !strings.Contains(out, "ran on") {
		t.Fatalf("with script.imports=off the pipeline should not run:\n%s", out)
	}
}

// TestPathsConfigReadsAConfigOnlyThatPathHolds is the effect probe for
// paths.config. A directory outside the project holds a config naming a
// server called extra. Oracle: by default the config chain finds only the
// project's config, so ls lists demo and no extra; with paths.config naming
// the directory its server is listed.
func TestPathsConfigReadsAConfigOnlyThatPathHolds(t *testing.T) {
	elsewhere := t.TempDir()
	cfg := fmt.Sprintf(`{"mcpServers":{"extra":{"command":%q}}}`, testsupport.FakeMCPBinary(t))
	if err := os.WriteFile(filepath.Join(elsewhere, ".mcpx.json"), []byte(cfg), 0o644); err != nil {
		t.Fatal(err)
	}
	ls := func(env ...string) string {
		t.Helper()
		e := newEnv(t, oneServer)
		// MCPX_CONFIG names one file and bypasses the chain, so the
		// harness's pin is lifted and the chain is searched instead.
		e.setenv(append([]string{"MCPX_CONFIG="}, env...)...)
		return e.run("ls")
	}
	if out := ls(); strings.Contains(out, "extra") || !strings.Contains(out, "demo") {
		t.Fatalf("oracle: by default the config elsewhere should not be read:\n%s", out)
	}
	if out := ls("MCPX_PATHS_CONFIG=" + elsewhere); !strings.Contains(out, "extra") {
		t.Fatalf("with paths.config=%s its server should be listed:\n%s", elsewhere, out)
	}
}

// TestLoggingMaxAgeRotatesAnAgedFile is the effect probe for
// logging.maxAge. Today's log file already holds a record from two hours
// ago. Oracle: at the default 24h the next run appends to it and keeps one
// file; at maxAge=1h the first new record rotates the aged file aside.
func TestLoggingMaxAgeRotatesAnAgedFile(t *testing.T) {
	files := func(env ...string) []string {
		t.Helper()
		e := newEnv(t, oneServer)
		dir := filepath.Join(t.TempDir(), "logs")
		if err := os.MkdirAll(dir, 0o700); err != nil {
			t.Fatal(err)
		}
		now := time.Now()
		aged := fmt.Sprintf(`{"ts":%q,"level":"info","msg":"aged","event":"harness.aged"}`+"\n",
			now.Add(-2*time.Hour).Format(time.RFC3339Nano))
		mustWrite(t, filepath.Join(dir, "mcpx-"+now.Format("2006-01-02")+".jsonl"), aged)
		e.setenv(append([]string{"MCPX_LOGGING_DIR=" + dir}, env...)...)
		e.run("ls")
		return logFiles(t, dir)
	}
	if got := files(); len(got) != 1 {
		t.Errorf("at the default logging.maxAge a two-hour-old file should keep taking records: %v", got)
	}
	if got := files("MCPX_LOGGING_MAX_AGE=1h"); len(got) < 2 {
		t.Errorf("logging.maxAge=1h should rotate the two-hour-old file aside: %v", got)
	}
}

// TestDaemonSocketProbeTimeoutDecidesWhetherALiveSocketIsStale is the effect
// probe for daemon.socketProbeTimeout. A daemon is running, and a second
// `mcpx daemon` starts for the same configuration. Before binding, the
// second one dials the socket within daemon.socketProbeTimeout to tell a
// live daemon from a crashed one's leftover file. Oracle: by default the
// dial answers, so it refuses to start; with a 1ns bound no dial can
// finish, so it takes the live socket for stale, removes it and listens.
func TestDaemonSocketProbeTimeoutDecidesWhetherALiveSocketIsStale(t *testing.T) {
	second := func(extra ...string) string {
		t.Helper()
		e := newEnv(t, oneServer)
		e.run("ls")
		state := e.daemonState(t)
		t.Cleanup(func() {
			// A first daemon whose socket was taken is unreachable, and so
			// is the server child it started; the test that orphaned them
			// retires them.
			for _, pid := range []int{state.PID, state.childOf("demo")} {
				if pid > 0 && processExists(pid) {
					if proc, err := os.FindProcess(pid); err == nil {
						_ = proc.Kill()
					}
				}
			}
		})
		var out syncBuffer
		cmd := exec.Command(e.mcpx, "daemon")
		cmd.Dir = e.dir
		cmd.Env = append(append([]string{}, e.envVars...), extra...)
		cmd.Stdout, cmd.Stderr = &out, &out
		if err := cmd.Start(); err != nil {
			t.Fatal(err)
		}
		done := make(chan struct{})
		go func() { _ = cmd.Wait(); close(done) }()
		defer func() {
			select {
			case <-done:
			default:
				_ = cmd.Process.Signal(os.Interrupt)
				select {
				case <-done:
				case <-time.After(10 * time.Second):
					_ = cmd.Process.Kill()
					<-done
				}
			}
		}()
		deadline := time.After(20 * time.Second)
		for {
			select {
			case <-done:
				return out.String()
			case <-deadline:
				t.Fatalf("the second daemon neither refused nor listened:\n%s", out.String())
			case <-time.After(50 * time.Millisecond):
			}
			if strings.Contains(out.String(), " listening on ") {
				return out.String()
			}
		}
	}

	if out := second(); !strings.Contains(out, "a daemon is already listening") {
		t.Fatalf("at the default daemon.socketProbeTimeout a second daemon should find the live one and refuse:\n%s", out)
	}
	out := second("MCPX_DAEMON_SOCKET_PROBE_TIMEOUT=1ns")
	if !strings.Contains(out, "removing stale socket") || !strings.Contains(out, " listening on ") {
		t.Fatalf("daemon.socketProbeTimeout=1ns should take the live socket for stale and listen on it:\n%s", out)
	}
}

// TestDaemonTakeoverTimeoutBoundsTheHandoverFromTheEnvironment is the
// effect probe for daemon.takeoverTimeout. A successor started with
// `mcpx daemon --takeover` dials the running daemon and exchanges the
// handover within this bound. Oracle: by default the successor takes over
// and serves; with a 1ns bound its dial cannot finish, so it gives up and
// the old daemon keeps serving with the same server child.
func TestDaemonTakeoverTimeoutBoundsTheHandoverFromTheEnvironment(t *testing.T) {
	e := newEnv(t, oneServer)
	e.setenv("MCPX_DAEMON_WATCH_CONFIG=false")
	e.run("call", "demo.echo", `{"message":"before"}`)
	s := startSuccessor(t, e)
	if after := e.awaitSuccessor(t, s); after.PID != s.cmd.Process.Pid {
		t.Fatalf("at the default daemon.takeoverTimeout the successor should take over")
	}

	e = newEnv(t, oneServer)
	e.setenv("MCPX_DAEMON_WATCH_CONFIG=false", "MCPX_DAEMON_TAKEOVER_TIMEOUT=1ns")
	e.run("call", "demo.echo", `{"message":"before"}`)
	before := e.daemonState(t)
	ctx, cancel := context.WithTimeout(context.Background(), 30*time.Second)
	defer cancel()
	cmd := exec.CommandContext(ctx, e.mcpx, "daemon", "--takeover")
	cmd.Dir = e.dir
	cmd.Env = append([]string{}, e.envVars...)
	out, err := cmd.CombinedOutput()
	if err == nil || ctx.Err() != nil || !strings.Contains(string(out), "i/o timeout") {
		t.Fatalf("daemon.takeoverTimeout=1ns should make the successor give up on its dial (err %v):\n%s", err, out)
	}
	after := e.daemonState(t)
	if after.PID != before.PID || after.childOf("demo") != before.childOf("demo") {
		t.Fatalf("the old daemon should keep serving after a failed takeover: pid %d -> %d", before.PID, after.PID)
	}
	if got := e.run("call", "demo.echo", `{"message":"after"}`); !strings.Contains(got, "after") {
		t.Fatalf("the old daemon should still answer:\n%s", got)
	}
}

// TestScriptCheckOnUnsupportedDecidesWhatANonDenoCheckDoes is the effect
// probe for script.checkOnUnsupported: with an explicit check on bun, the
// default (warn) runs the script with a notice that only Deno checks,
// skip runs it quietly, and switch, with no deno to check with, refuses.
func TestScriptCheckOnUnsupportedDecidesWhatANonDenoCheckDoes(t *testing.T) {
	testsupport.RequireBinary(t, "bun")
	e := newEnv(t, oneServer)
	script := filepath.Join(e.dir, "plain.ts")
	if err := os.WriteFile(script, []byte("export default function () { return \"ran-plain\"; }\n"), 0o644); err != nil {
		t.Fatal(err)
	}
	e.setenv("MCPX_SCRIPT_RUNTIME=bun")
	e.setenv("MCPX_SCRIPT_TYPECHECK=on")

	out, err := e.try("run", script)
	if err != nil || !strings.Contains(out, "ran-plain") {
		t.Fatalf("the default policy should still run the script: %v\n%s", err, out)
	}
	if !strings.Contains(out, "only available under Deno") {
		t.Errorf("the default policy should warn that only Deno checks:\n%s", out)
	}

	e.setenv("MCPX_SCRIPT_CHECK_ON_UNSUPPORTED=skip")
	out, err = e.try("run", script)
	if err != nil || !strings.Contains(out, "ran-plain") {
		t.Fatalf("skip should run the script: %v\n%s", err, out)
	}
	if strings.Contains(out, "only available under Deno") {
		t.Errorf("skip should not warn:\n%s", out)
	}

	if _, lookErr := exec.LookPath("deno"); lookErr == nil {
		return // switch would check with the real deno; the two legs above already tell the policies apart.
	}
	e.setenv("MCPX_SCRIPT_CHECK_ON_UNSUPPORTED=switch")
	out, err = e.try("run", script)
	if err == nil {
		t.Fatalf("switch with no deno on PATH should refuse:\n%s", out)
	}
	if !strings.Contains(out, "deno is not on PATH") || strings.Contains(out, "ran-plain") {
		t.Errorf("switch should refuse before running, naming the missing deno:\n%s", out)
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
	// A socket in the state directory, or the one a daemon's info record
	// says it bound. A long state path moves the socket to a private
	// runtime directory (daemon.socketPath), which is every nix sandbox
	// build: globbing the state directory alone found nothing there and
	// failed the aarch64-darwin build although the daemon was up.
	socketsIn := func(e *env) []string {
		socks, _ := filepath.Glob(filepath.Join(e.dir, "state", "*.sock"))
		infos, _ := filepath.Glob(filepath.Join(e.dir, "state", "daemon*.json"))
		for _, info := range infos {
			var rec struct {
				Socket string `json:"socket"`
			}
			b, err := os.ReadFile(info)
			if err != nil || json.Unmarshal(b, &rec) != nil || rec.Socket == "" {
				continue
			}
			if fi, err := os.Stat(rec.Socket); err == nil && fi.Mode()&os.ModeSocket != 0 {
				socks = append(socks, rec.Socket)
			}
		}
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
