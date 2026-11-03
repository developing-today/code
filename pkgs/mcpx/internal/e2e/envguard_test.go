package e2e_test

import (
	"path/filepath"
	"regexp"
	"strings"
	"testing"

	"github.com/developing-today/code/pkgs/mcpx/internal/settings"
)

// The guard for #1992, Approach C: every MCPX_* variable the settings
// registry declares reaches a test child either as a value the harness chose
// or not at all. A parent value leaking through would run the suite under
// the developer's settings -- or, for the daemon endpoint and socket,
// against the developer's real daemon -- and fail on one machine and nowhere
// else.
//
// A new setting is covered the day it is added: TestEveryRegistryVariable
// below fails until its variables are classified here, into exactly one of
// the two tables. "Set by the harness" means the harness sets the variable
// somewhere in this package (newEnv for every child, or a test's withEnv for
// its own), so a test that depends on the value chooses it. "Safe" does not
// mean inherited: the allowlist builder drops every MCPX_* variable from the
// parent, so safe means safe under the harness's inherit-nothing policy --
// the variable keeps its default unless the harness sets it, and no caller
// value can change what a test observes. The behaviour half
// (TestTheHarnessBuildsChildrenFromTheAllowlist) proves the drop for every
// name in both tables at once, so a stale entry in either direction fails
// loudly instead of drifting.

// harnessSetMCPX is every registry variable the harness sets: newEnv for
// every child, or a test's withEnv/append for its own.
var harnessSetMCPX = map[string]struct{}{
	"MCPX_ARTIFACTS_MAX_BYTES":          {},
	"MCPX_AUTONOMY_MAX":                 {},
	"MCPX_AUTOSTART_ARGS":               {},
	"MCPX_AUTOSTART_IDLE_EXIT":          {},
	"MCPX_CACHE_DIR":                    {},
	"MCPX_CONFIG":                       {},
	"MCPX_DAEMON_AUTOSTART":             {},
	"MCPX_DAEMON_ENDPOINT":              {},
	"MCPX_DAEMON_INLINE":                {},
	"MCPX_DAEMON_WARM":                  {},
	"MCPX_DAEMON_WATCH_CONFIG":          {},
	"MCPX_DEPS_MANIFEST":                {},
	"MCPX_DEPS_PACKAGES":                {},
	"MCPX_DEPS_SOURCE":                  {},
	"MCPX_ELICIT_ASK_TIMEOUT":           {},
	"MCPX_ELICIT_CONFIRM_DESTRUCTIVE":   {},
	"MCPX_HOOKS_AUTONOMY":               {},
	"MCPX_LOGGING_FILE":                 {},
	"MCPX_MCP_PASSTHROUGH":              {},
	"MCPX_MCP_SPEC":                     {},
	"MCPX_OUTPUT_JSON":                  {},
	"MCPX_PATHS_ADAPTERS":               {},
	"MCPX_PATHS_DEPS":                   {},
	"MCPX_PATHS_STATE":                  {},
	"MCPX_PERMISSIONS":                  {},
	"MCPX_PLUMBING_STRICT_UNKNOWN_KEYS": {},
	"MCPX_POLICY_WRITE":                 {},
	"MCPX_POOL_MAX":                     {},
	"MCPX_POOL_SCOPE":                   {},
	"MCPX_POOL_SHARING":                 {},
	"MCPX_PRESET":                       {},
	"MCPX_PROMPT_AUTONOMY":              {},
	"MCPX_PROTO_NATIVE":                 {},
	"MCPX_PROTO_TASKS_POLL_INTERVAL":    {},
	"MCPX_REGISTRY_LIMIT":               {},
	"MCPX_REGISTRY_MAX_PAGES":           {},
	"MCPX_REGISTRY_PAGE_SIZE":           {},
	"MCPX_REGISTRY_URL":                 {},
	"MCPX_SCRIPT_CAPTURE_CONSOLE":       {},
	"MCPX_SCRIPT_CWD":                   {},
	"MCPX_SCRIPT_ENV":                   {},
	"MCPX_SCRIPT_INTERPRETER":           {},
	"MCPX_SCRIPT_PERMISSIONS":           {},
	"MCPX_SCRIPT_TYPECHECK":             {},
	"MCPX_SEARCH_LIMIT":                 {},
	"MCPX_SPEC_LENIENT":                 {},
	"MCPX_STATE_DIR":                    {},
	"MCPX_TRACE":                        {},
	"MCPX_TRANSPORT_ALLOWED_ORIGINS":    {},
	"MCPX_UPSTREAM_ERA_CACHE":           {},
	"MCPX_UPSTREAM_PROBE_TIMEOUT":       {},
	"MCPX_UPSTREAM_PROTOCOL":            {},
}

// safeMCPX is every other registry variable: safe under the harness's
// inherit-nothing policy, because the allowlist builder never inherits a
// parent MCPX_* value and the behaviour test below proves the drop for each
// one. A variable the harness must control belongs in harnessSetMCPX, not
// here.
var safeMCPX = map[string]struct{}{
	"MCPX_ARTIFACTS_CHUNK_BYTES":                {},
	"MCPX_ARTIFACTS_DELIVERY":                   {},
	"MCPX_ARTIFACTS_DIR":                        {},
	"MCPX_ARTIFACTS_ENABLED":                    {},
	"MCPX_ARTIFACTS_GC_INTERVAL":                {},
	"MCPX_ARTIFACTS_IDLE_TTL":                   {},
	"MCPX_ARTIFACTS_INLINE_MAX_BYTES":           {},
	"MCPX_ARTIFACTS_INTERCEPT_IMAGES":           {},
	"MCPX_ARTIFACTS_MAX_AGE":                    {},
	"MCPX_ARTIFACTS_QUOTA":                      {},
	"MCPX_ARTIFACTS_TTL":                        {},
	"MCPX_AUTOSTART_BIN":                        {},
	"MCPX_AUTOSTART_CONNECT_TIMEOUT":            {},
	"MCPX_AUTOSTART_LOG_TAIL":                   {},
	"MCPX_AUTOSTART_PING_TIMEOUT":               {},
	"MCPX_AUTOSTART_POLL_INTERVAL":              {},
	"MCPX_CATALOG_BIAS":                         {},
	"MCPX_CATALOG_BUDGET":                       {},
	"MCPX_CATALOG_INSTRUCTIONS":                 {},
	"MCPX_CATALOG_PINNED_TOOLS":                 {},
	"MCPX_COMPLETION_MAX_VALUES":                {},
	"MCPX_COMPLETION_PER_MINUTE":                {},
	"MCPX_COMPLETION_SERVER_PER_MINUTE":         {},
	"MCPX_DAEMON_ADDRESS":                       {},
	"MCPX_DAEMON_IDLE_EXIT":                     {},
	"MCPX_DAEMON_INLINE_START_POLL":             {},
	"MCPX_DAEMON_INLINE_START_TIMEOUT":          {},
	"MCPX_DAEMON_LEASE_TTL":                     {},
	"MCPX_DAEMON_PORT":                          {},
	"MCPX_DAEMON_PROBE_TIMEOUT":                 {},
	"MCPX_DAEMON_REAP_INTERVAL":                 {},
	"MCPX_DAEMON_REFRESH_TIMEOUT":               {},
	"MCPX_DAEMON_ROOTS":                         {},
	"MCPX_DAEMON_SAVE_INTERVAL":                 {},
	"MCPX_DAEMON_SOCKET_PROBE_TIMEOUT":          {},
	"MCPX_DAEMON_TAKEOVER_TIMEOUT":              {},
	"MCPX_DAEMON_TSNET_HOSTNAME":                {},
	"MCPX_DAEMON_TSNET_UP_TIMEOUT":              {},
	"MCPX_DAEMON_UI":                            {},
	"MCPX_DAEMON_UI_DIR":                        {},
	"MCPX_DAEMON_WARM_TIMEOUT":                  {},
	"MCPX_DEPS_AUTO_INSTALL":                    {},
	"MCPX_DIAGNOSE_HISTORY":                     {},
	"MCPX_DIAGNOSE_PREFLIGHT":                   {},
	"MCPX_DOCTOR_TIMEOUT":                       {},
	"MCPX_ELICIT_DISAMBIGUATE":                  {},
	"MCPX_ELICIT_DISAMBIGUATE_DEFAULT":          {},
	"MCPX_ELICIT_MAX_AGE":                       {},
	"MCPX_ELICIT_PENDING_LIMIT":                 {},
	"MCPX_ELICIT_TTL":                           {},
	"MCPX_EMBEDDINGS_API_KEY":                   {},
	"MCPX_EMBEDDINGS_BACKEND":                   {},
	"MCPX_EMBEDDINGS_MODEL":                     {},
	"MCPX_EMBEDDINGS_URL":                       {},
	"MCPX_EMBEDDINGS_WASM_PATH":                 {},
	"MCPX_EVENTS_HISTORY":                       {},
	"MCPX_EVENTS_RECONNECT":                     {},
	"MCPX_EXEC_MAX_OUTPUT_BYTES":                {},
	"MCPX_EXEC_OUTPUT":                          {},
	"MCPX_EXEC_TIMEOUT":                         {},
	"MCPX_EXEC_WHERE":                           {},
	"MCPX_HOOKS":                                {},
	"MCPX_HOOKS_TIMEOUT":                        {},
	"MCPX_HTTP_BODY_LIMIT":                      {},
	"MCPX_HTTP_CALL_BODY_LIMIT":                 {},
	"MCPX_HTTP_CONTROL_BODY_LIMIT":              {},
	"MCPX_HTTP_IDLE_CONNS":                      {},
	"MCPX_HTTP_IDLE_CONN_TIMEOUT":               {},
	"MCPX_HTTP_READ_HEADER_TIMEOUT":             {},
	"MCPX_HTTP_REMOTE_IDLE_CONNS":               {},
	"MCPX_HTTP_REQUEST_TIMEOUT":                 {},
	"MCPX_HTTP_SHUTDOWN_GRACE":                  {},
	"MCPX_HTTP_SSE_PING":                        {},
	"MCPX_HTTP_SSE_RETRY":                       {},
	"MCPX_HTTP_STREAM_BUFFER_INIT":              {},
	"MCPX_HTTP_STREAM_BUFFER_MAX":               {},
	"MCPX_LOGGING_DIR":                          {},
	"MCPX_LOGGING_FORMAT":                       {},
	"MCPX_LOGGING_INCLUDE":                      {},
	"MCPX_LOGGING_KEEP":                         {},
	"MCPX_LOGGING_LEVEL":                        {},
	"MCPX_LOGGING_MAX_AGE":                      {},
	"MCPX_LOGGING_MAX_BYTES":                    {},
	"MCPX_LOGGING_MAX_LINES":                    {},
	"MCPX_LOGGING_SOURCE":                       {},
	"MCPX_LOG_LEVEL":                            {},
	"MCPX_LOGSTORE_FOLLOW_BACKLOG":              {},
	"MCPX_LOGSTORE_QUERY_LIMIT":                 {},
	"MCPX_MCP_PAGE_SIZE":                        {},
	"MCPX_MODEL_API_KEY_ENV":                    {},
	"MCPX_MODEL_BASE_URL":                       {},
	"MCPX_MODEL_BUDGET":                         {},
	"MCPX_MODEL_DEFAULT":                        {},
	"MCPX_MODEL_PROVIDER":                       {},
	"MCPX_MODEL_TIMEOUT":                        {},
	"MCPX_OUTPUT_COLOR":                         {},
	"MCPX_PATHS_APIS":                           {},
	"MCPX_PATHS_CACHE":                          {},
	"MCPX_PATHS_CONFIG":                         {},
	"MCPX_PATHS_PLACEHOLDERS":                   {},
	"MCPX_PATHS_PLUGINS":                        {},
	"MCPX_PATHS_SCRIPTS":                        {},
	"MCPX_PLUGIN_ANNOTATE":                      {},
	"MCPX_PLUGIN_BACKEND":                       {},
	"MCPX_PLUGIN_BIN":                           {},
	"MCPX_PLUGIN_BIN_ARGS":                      {},
	"MCPX_PLUGIN_DAEMON_TOOLS":                  {},
	"MCPX_PLUGIN_DISCOVERY_RETRY":               {},
	"MCPX_PLUGIN_ENV":                           {},
	"MCPX_PLUGIN_HEADLESS":                      {},
	"MCPX_PLUGIN_INSTRUCTIONS":                  {},
	"MCPX_PLUGIN_REMEMBER":                      {},
	"MCPX_PLUGIN_TOOLS":                         {},
	"MCPX_PLUGIN_TOOL_TIMING":                   {},
	"MCPX_PLUMBING_ALLOW_TS_JS_OVERLAP":         {},
	"MCPX_PLUMBING_INDEX_ON_QUERY":              {},
	"MCPX_PLUMBING_LAUNCHER_PLACEHOLDER_REPEAT": {},
	"MCPX_PLUMBING_SOURCE_DIR_ALLOWED":          {},
	"MCPX_PLUMBING_SOURCE_DIR_RECURSIVE":        {},
	"MCPX_PLUMBING_SOURCE_PROBE_PATHS":          {},
	"MCPX_PLUMBING_VALIDATE_PATHS":              {},
	"MCPX_POOL_CALL_TIMEOUT":                    {},
	"MCPX_POOL_IDLE_TIMEOUT":                    {},
	"MCPX_POOL_MIN":                             {},
	"MCPX_POOL_START_TIMEOUT":                   {},
	"MCPX_PRESETS":                              {},
	"MCPX_PROMPT_CATALOG_BUDGET":                {},
	"MCPX_PROMPT_MAX_TOKENS":                    {},
	"MCPX_PROMPT_RUN_TIMEOUT":                   {},
	"MCPX_PROMPT_SAMPLE":                        {},
	"MCPX_PROMPT_SAMPLE_TIMEOUT":                {},
	"MCPX_PROTO_ASK_POLL":                       {},
	"MCPX_PROTO_ASK_ROUNDS":                     {},
	"MCPX_PROTO_ASK_TIMEOUT":                    {},
	"MCPX_PROTO_ASK_TTL":                        {},
	"MCPX_PROTO_ELICITATION_PER_MINUTE":         {},
	"MCPX_PROTO_MCP_PATH":                       {},
	"MCPX_PROTO_MESSAGES_LIST_MAX_AGE":          {},
	"MCPX_PROTO_MESSAGES_READ_MAX_AGE":          {},
	"MCPX_PROTO_MESSAGES_TASK_AFTER":            {},
	"MCPX_PROTO_SAMPLING_PER_MINUTE":            {},
	"MCPX_PROTO_SAMPLING_SESSION_PER_MINUTE":    {},
	"MCPX_PROTO_SERVE_MCP":                      {},
	"MCPX_PROTO_SESSION_IDLE":                   {},
	"MCPX_PROTO_STATE_TTL":                      {},
	"MCPX_PROTO_TASKS_CANCEL_GRACE":             {},
	"MCPX_PROTO_TASKS_MAX_PER_REQUESTOR":        {},
	"MCPX_PROTO_TASKS_MAX_TTL":                  {},
	"MCPX_PROTO_TASKS_PER_MINUTE":               {},
	"MCPX_RECIPES_LIMIT":                        {},
	"MCPX_RECIPES_MATCH_MARGIN":                 {},
	"MCPX_RECIPES_MIN_SCORE":                    {},
	"MCPX_REGISTRY_TIMEOUT":                     {},
	"MCPX_REPAIR_AUTONOMY":                      {},
	"MCPX_RETENTION_MAX_TOOLS":                  {},
	"MCPX_RETENTION_STICKY_WINDOW":              {},
	"MCPX_SCRIPT_BEFORE":                        {},
	"MCPX_SCRIPT_IMPORTS":                       {},
	"MCPX_SCRIPT_IMPORTS_ON_DYNAMIC":            {},
	"MCPX_SCRIPT_LAUNCHER":                      {},
	"MCPX_SCRIPT_ON_ERROR":                      {},
	"MCPX_SCRIPT_ON_SUCCESS":                    {},
	"MCPX_SCRIPT_PREFIX":                        {},
	"MCPX_SCRIPT_PROFILES":                      {},
	"MCPX_SCRIPT_RUNTIME":                       {},
	"MCPX_SCRIPT_RUNTIMES":                      {},
	"MCPX_SCRIPT_RUNTIME_ORDER":                 {},
	"MCPX_SCRIPT_SUFFIX":                        {},
	"MCPX_SEARCH_SEMANTIC":                      {},
	"MCPX_SERVE_MODE":                           {},
	"MCPX_SESSION_RELEASE_TIMEOUT":              {},
	"MCPX_SPEC_PRECEDENCE":                      {},
	"MCPX_STATS_TOP":                            {},
	"MCPX_TASKS_RESULT_WAIT":                    {},
	"MCPX_TASKS_TTL":                            {},
	"MCPX_TRANSPORT_BEARER_TOKEN":               {},
	"MCPX_TRANSPORT_SESSION_META_KEYS":          {},
	"MCPX_TRANSPORT_SSE_KEEP_ALIVE":             {},
	"MCPX_TRANSPORT_STDIO_DRAIN":                {},
}

// registryEnvNames is every MCPX_* spelling one setting answers to: its
// derived name and its aliases.
func registryEnvNames(set settings.Setting) []string {
	return append([]string{set.EnvName()}, set.EnvAliases...)
}

// TestEveryRegistryVariableIsHarnessSetOrSafe is the static half of the
// guard: every variable the registry declares is classified into exactly one
// of the two tables above. Adding a setting without classifying it fails
// here; removing one without deleting its entry fails here too.
func TestEveryRegistryVariableIsHarnessSetOrSafe(t *testing.T) {
	sch, err := settings.New(settings.Registry())
	if err != nil {
		t.Fatalf("the registry should be internally consistent: %v", err)
	}
	seen := map[string]bool{}
	for _, set := range sch.All() {
		for _, name := range registryEnvNames(set) {
			seen[name] = true
			_, inHarness := harnessSetMCPX[name]
			_, inSafe := safeMCPX[name]
			switch {
			case inHarness && inSafe:
				t.Errorf("%s (%s) is in both tables; it belongs in exactly one", name, set.Path)
			case !inHarness && !inSafe:
				t.Errorf("%s (%s) is in neither table; classify it: harnessSetMCPX if the harness sets it, safeMCPX if the default governs", name, set.Path)
			}
		}
	}
	for name := range harnessSetMCPX {
		if !seen[name] {
			t.Errorf("harnessSetMCPX names %s, which no setting declares; delete the stale entry", name)
		}
	}
	for name := range safeMCPX {
		if !seen[name] {
			t.Errorf("safeMCPX names %s, which no setting declares; delete the stale entry", name)
		}
	}
}

// alwaysSetMCPX is what newEnv sets for every child, so a parent value for
// one of these must never survive: the behaviour test plants canaries for
// them and requires the harness value to win.
var alwaysSetMCPX = []string{
	"MCPX_REGISTRY_URL",
	"MCPX_STATE_DIR",
	"MCPX_CACHE_DIR",
	"MCPX_CONFIG",
	"MCPX_AUTOSTART_IDLE_EXIT",
	"MCPX_POLICY_WRITE",
}

// callerLeakParent are caller variable classes the old inherit-everything
// harness leaked into tests: XDG base directories, outer session scoping,
// proxies and NODE_OPTIONS. (HOME is planted separately: it must exist for
// testsupport.TempDir to resolve it.)
func callerLeakParent() []string {
	return []string{
		"XDG_CONFIG_HOME=/nonexistent-mcpx-guard/XDG_CONFIG_HOME",
		"XDG_DATA_HOME=/nonexistent-mcpx-guard/XDG_DATA_HOME",
		"XDG_STATE_HOME=/nonexistent-mcpx-guard/XDG_STATE_HOME",
		"XDG_CACHE_HOME=/nonexistent-mcpx-guard/XDG_CACHE_HOME",
		"XDG_RUNTIME_DIR=/nonexistent-mcpx-guard/XDG_RUNTIME_DIR",
		"OPENCODE_TEST_GUARD=e2e-guard-canary",
		"OPENCODE_DB=/nonexistent-mcpx-guard/opencode.db",
		"HTTP_PROXY=http://nonexistent-mcpx-guard:9",
		"HTTPS_PROXY=http://nonexistent-mcpx-guard:9",
		"ALL_PROXY=http://nonexistent-mcpx-guard:9",
		"NO_PROXY=nonexistent-mcpx-guard",
		"http_proxy=http://nonexistent-mcpx-guard:9",
		"https_proxy=http://nonexistent-mcpx-guard:9",
		"all_proxy=http://nonexistent-mcpx-guard:9",
		"no_proxy=nonexistent-mcpx-guard",
		"NODE_OPTIONS=--require /nonexistent-mcpx-guard",
	}
}

// TestTheHarnessBuildsChildrenFromTheAllowlist is the behaviour half of the
// guard: with a hostile parent -- a canary for every registry variable plus
// every caller variable class the old harness leaked -- a fresh child
// carries the harness values where the harness sets them, the test dir's
// XDG roots, and nothing else from the caller.
func TestTheHarnessBuildsChildrenFromTheAllowlist(t *testing.T) {
	sch, err := settings.New(settings.Registry())
	if err != nil {
		t.Fatalf("the registry should be internally consistent: %v", err)
	}
	for _, set := range sch.All() {
		for _, name := range registryEnvNames(set) {
			switch name {
			case "MCPX_DAEMON_ENDPOINT":
				t.Setenv(name, "http://127.0.0.1:1")
			case "DENO_DIR", "BUN_INSTALL_CACHE_DIR", "NPM_CONFIG_CACHE":
				// Allowlisted runtime-cache passthroughs by design; the
				// harness sets the first two explicitly, so no canary.
			default:
				t.Setenv(name, "e2e-guard-canary")
			}
		}
	}
	// MCPX_SOCKET is not a registry variable -- no setting declares it -- but
	// it is the daemon's raw socket override, read at the connect/bind call
	// sites, so it gets the same canary treatment.
	t.Setenv("MCPX_SOCKET", "/nonexistent-mcpx-guard.sock")
	// The runtime caches are allowlisted passthroughs: the child must see
	// the parent's values, chosen here rather than inherited by accident.
	t.Setenv("DENO_DIR", "/nonexistent-mcpx-guard/deno")
	t.Setenv("BUN_INSTALL_CACHE_DIR", "/nonexistent-mcpx-guard/bun")
	t.Setenv("NPM_CONFIG_CACHE", "/nonexistent-mcpx-guard/npm")
	for _, kv := range callerLeakParent() {
		name, value, _ := strings.Cut(kv, "=")
		t.Setenv(name, value)
	}
	// HOME must exist: testsupport.TempDir resolves it, so the canary is a
	// real directory that is still unmistakably not the test's own home.
	fakeHome := t.TempDir()
	t.Setenv("HOME", fakeHome)

	e := newEnv(t, oneServer)
	// Last wins in os/exec, so the map holds what the child resolves -- but
	// the counts prove the parent value is gone, not merely overridden: an
	// inherited-then-overridden duplicate would still be a leak to catch.
	got := map[string]string{}
	counts := map[string]int{}
	for _, kv := range e.envVars {
		name, value, ok := strings.Cut(kv, "=")
		if !ok {
			t.Fatalf("harness env entry %q has no =", kv)
		}
		got[name] = value
		counts[name]++
	}

	// The harness-controlled variables win over any parent value, exactly
	// once each.
	for _, name := range alwaysSetMCPX {
		v, ok := got[name]
		if !ok {
			t.Errorf("%s: newEnv must set it for every child", name)
			continue
		}
		if counts[name] != 1 {
			t.Errorf("%s appears %d times: the parent value reaches the child and is only overridden", name, counts[name])
		}
		if strings.Contains(v, "canary") || strings.Contains(v, "nonexistent") || v == "http://127.0.0.1:1" {
			t.Errorf("%s=%q: the parent canary reached the child", name, v)
		}
	}
	if v := got["MCPX_STATE_DIR"]; v != filepath.Join(e.dir, "state") {
		t.Errorf("MCPX_STATE_DIR=%q, want the test's own state dir", v)
	}
	if v := got["MCPX_CACHE_DIR"]; v != filepath.Join(e.dir, "cache") {
		t.Errorf("MCPX_CACHE_DIR=%q, want the test's own cache dir", v)
	}
	if v := got["MCPX_CONFIG"]; v != filepath.Join(e.dir, ".mcpx.json") {
		t.Errorf("MCPX_CONFIG=%q, want the test's own config", v)
	}

	// No other registry variable reaches the child, including the ones
	// individual tests set for themselves: a fresh child leaves them at
	// their defaults. MCPX_SOCKET is not registry-declared but is checked
	// the same way.
	always := map[string]bool{}
	for _, name := range alwaysSetMCPX {
		always[name] = true
	}
	for _, set := range sch.All() {
		for _, name := range registryEnvNames(set) {
			if always[name] {
				continue
			}
			if v, ok := got[name]; ok {
				t.Errorf("%s=%q: a parent registry value reached the child", name, v)
			}
		}
	}
	if v, ok := got["MCPX_SOCKET"]; ok {
		t.Errorf("MCPX_SOCKET=%q: the caller's socket override reached the child", v)
	}

	// The runtime-cache passthroughs carry the parent's values.
	for name, want := range map[string]string{
		"DENO_DIR":              "/nonexistent-mcpx-guard/deno",
		"BUN_INSTALL_CACHE_DIR": "/nonexistent-mcpx-guard/bun",
		"NPM_CONFIG_CACHE":      "/nonexistent-mcpx-guard/npm",
	} {
		if got[name] != want {
			t.Errorf("%s=%q, want the parent passthrough %q", name, got[name], want)
		}
	}

	// XDG base directories resolve under the test's own dir.
	for name, want := range map[string]string{
		"XDG_CONFIG_HOME": filepath.Join(e.dir, ".config"),
		"XDG_DATA_HOME":   filepath.Join(e.dir, ".local", "share"),
		"XDG_STATE_HOME":  filepath.Join(e.dir, ".local", "state"),
		"XDG_CACHE_HOME":  filepath.Join(e.dir, ".cache"),
		"XDG_RUNTIME_DIR": filepath.Join(e.dir, "run"),
	} {
		if got[name] != want {
			t.Errorf("%s=%q, want %q", name, got[name], want)
		}
	}

	// No other caller variable class reaches the child.
	if got["HOME"] != filepath.Join(e.dir, "home") {
		t.Errorf("HOME=%q, want the test's own home", got["HOME"])
	}
	if got["HOME"] == fakeHome {
		t.Errorf("HOME=%q: the parent home reached the child", got["HOME"])
	}
	for name := range got {
		upper := strings.ToUpper(name)
		switch {
		case strings.HasPrefix(upper, "OPENCODE_"):
			t.Errorf("%s=%q: outer session scoping reached the child", name, got[name])
		case upper == "NODE_OPTIONS":
			t.Errorf("%s=%q: caller node flags reached the child", name, got[name])
		case strings.HasSuffix(upper, "_PROXY") || upper == "HTTP_PROXY" || upper == "HTTPS_PROXY" || upper == "ALL_PROXY":
			t.Errorf("%s=%q: a caller proxy reached the child", name, got[name])
		case strings.HasPrefix(name, "XDG_"):
			switch name {
			case "XDG_CONFIG_HOME", "XDG_DATA_HOME", "XDG_STATE_HOME", "XDG_CACHE_HOME", "XDG_RUNTIME_DIR":
			default:
				t.Errorf("%s=%q: a caller XDG variable reached the child", name, got[name])
			}
		}
	}
	if got["PATH"] == "" {
		t.Error("PATH is empty: the child would find no runtime")
	}
}

// TestTheHarnessInheritsNoMCPXVariable is the regression test #1992
// prescribes: with a hostile parent -- a daemon endpoint and socket pointing
// at the caller's daemon, a bogus script runtime, a HOME that does not
// exist -- the suite's own commands stay local, default and home-free.
func TestTheHarnessInheritsNoMCPXVariable(t *testing.T) {
	t.Setenv("MCPX_DAEMON_ENDPOINT", "http://127.0.0.1:1")
	t.Setenv("MCPX_SOCKET", "/nonexistent-mcpx-guard.sock")
	t.Setenv("MCPX_SCRIPT_RUNTIME", "bogus-runtime")
	// HOME must exist: testsupport.TempDir resolves it, so the canary is a
	// real directory that is still unmistakably not the test's own home.
	fakeHome := t.TempDir()
	t.Setenv("HOME", fakeHome)
	e := newEnv(t, oneServer)
	// The harness's own stub registry is a 127.0.0.1 URL, so the endpoint
	// canary is checked with the stub exempted: a stub port starting with
	// 1 would otherwise read as a leak.
	stub := gotRegistryURL(t, e.envVars)

	canaries := []string{"/nonexistent-mcpx-guard", "bogus-runtime"}
	// The endpoint canary needs a port boundary: the stub registry is
	// itself a 127.0.0.1 URL, and a stub port starting with 1 would
	// otherwise read as a leak -- as would any 127.0.0.1:1xxx port a
	// diagnostic prints. Port 1 followed by a non-digit is only the canary.
	scrubbed := func(out string) string { return strings.ReplaceAll(out, stub, "") }
	endpointCanary := regexp.MustCompile(`127\.0\.0\.1:1([^0-9]|$)`)
	leaksEndpoint := func(out string) bool { return endpointCanary.MatchString(scrubbed(out)) }
	for _, kv := range e.envVars {
		if strings.HasPrefix(kv, "MCPX_REGISTRY_URL=") {
			if kv != "MCPX_REGISTRY_URL="+stub {
				t.Fatalf("MCPX_REGISTRY_URL entry %q is not the harness stub", kv)
			}
			continue
		}
		if kv == "HOME="+fakeHome {
			t.Fatalf("the parent HOME reached the child environment in %q", kv)
		}
		if leaksEndpoint(kv) {
			t.Fatalf("parent canary %q reached the child environment in %q", "http://127.0.0.1:1", kv)
		}
		for _, c := range canaries {
			if strings.Contains(kv, c) {
				t.Fatalf("parent canary %q reached the child environment in %q", c, kv)
			}
		}
	}
	// settings list and recipes are read-only and deterministic: they must
	// succeed, and stay canary-free. doctor's exit depends on machine
	// health, not isolation, so it must only stay canary-free.
	for _, args := range [][]string{{"settings", "list"}, {"recipes"}} {
		out, err := e.try(args...)
		if err != nil {
			t.Fatalf("mcpx %s failed: %v\n%s", strings.Join(args, " "), err, out)
		}
		if leaksEndpoint(out) {
			t.Errorf("mcpx %s shows the parent endpoint canary:\n%s", strings.Join(args, " "), out)
		}
		for _, c := range canaries {
			if strings.Contains(scrubbed(out), c) {
				t.Errorf("mcpx %s shows the parent canary %q:\n%s", strings.Join(args, " "), c, out)
			}
		}
	}
	out, _ := e.try("doctor", "-v")
	if leaksEndpoint(out) {
		t.Errorf("mcpx doctor -v shows the parent endpoint canary:\n%s", out)
	}
	for _, c := range canaries {
		if strings.Contains(scrubbed(out), c) {
			t.Errorf("mcpx doctor -v shows the parent canary %q:\n%s", c, out)
		}
	}
}

// gotRegistryURL is the harness stub URL newEnv sets for every child.
func gotRegistryURL(t *testing.T, env []string) string {
	t.Helper()
	seen := 0
	var stub string
	for _, kv := range env {
		if v, ok := strings.CutPrefix(kv, "MCPX_REGISTRY_URL="); ok {
			seen++
			stub = v
		}
	}
	if seen != 1 || !strings.HasPrefix(stub, "http://127.0.0.1:") {
		t.Fatalf("want exactly one harness stub registry URL, got %d entries (last %q)", seen, stub)
	}
	return stub
}
