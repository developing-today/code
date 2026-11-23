package e2e_test

import (
	"bytes"
	_ "embed"
	"encoding/json"
	"io"
	"net/http"
	"net/http/httptest"
	"net/url"
	"os"
	"path/filepath"
	"regexp"
	"slices"
	"strings"
	"sync"
	"testing"

	"github.com/developing-today/code/pkgs/mcpx/internal/testsupport"
)

// Running the e2e suite against a daemon the test did not start (#13386).
//
// Remote mode is on when the test process runs with MCPX_E2E_REMOTE=1 and
// MCPX_DAEMON_ENDPOINT naming the shared daemon. Selection fails closed,
// in two layers:
//
//   - The CI leg (remote-e2e in .github/workflows/mcpx-test.yml) builds
//     its -run pattern from remote_safe_tests.txt, so an undeclared test
//     is never even started.
//   - newEnv refuses to build an env in remote mode for a test that is
//     not in the registry, so a manual full-suite run against a shared
//     daemon skips everything undeclared instead of pointing host-local
//     fixtures at it.
//
// A test joins the subset by adding its name to remote_safe_tests.txt.
// That is a claim about the test, not about the harness: the test must
// not need a host-local server binary (the FAKE fixture path does not
// exist on the daemon's host), must not stop or restart the daemon, and
// must not depend on a per-test daemon config file. A test that needs a
// server on the daemon adds it with addRemoteServer, which registers it
// over POST /v1/servers under a per-test suffixed name and removes it in
// cleanup.

//go:embed remote_safe_tests.txt
var remoteSafeRegistry string

// remoteMode reports whether this test process was asked to run against
// a shared remote daemon.
func remoteMode() bool {
	return os.Getenv("MCPX_E2E_REMOTE") == "1" && os.Getenv("MCPX_DAEMON_ENDPOINT") != ""
}

// remoteSafeTests parses the registry: one test name per line, blank
// lines and #-comments ignored.
func remoteSafeTests() []string {
	var names []string
	for _, line := range strings.Split(remoteSafeRegistry, "\n") {
		line = strings.TrimSpace(line)
		if line == "" || strings.HasPrefix(line, "#") {
			continue
		}
		names = append(names, line)
	}
	return names
}

// remoteSafeTest reports whether a test — named in full or by a subtest
// path under it — is declared remote-safe.
func remoteSafeTest(name string) bool {
	name, _, _ = strings.Cut(name, "/")
	return slices.Contains(remoteSafeTests(), name)
}

// remoteRunPattern builds the anchored -run pattern matching exactly the
// registered tests and nothing else.
func remoteRunPattern() string {
	names := remoteSafeTests()
	quoted := make([]string, len(names))
	for i, name := range names {
		quoted[i] = regexp.QuoteMeta(name)
	}
	return "^(" + strings.Join(quoted, "|") + ")$"
}

// newRemoteEnv is newEnv for remote mode: the CLI children are pointed at
// the shared daemon over MCPX_DAEMON_ENDPOINT and nothing is started or
// stopped locally. The per-test config the local harness would seed names
// host-local server binaries, which do not exist on the daemon's host, so
// the remote env writes an empty server set instead; cfgBody is accepted
// for signature parity with newEnv and deliberately not transplanted. A
// test that needs a server on the daemon adds it with addRemoteServer.
func newRemoteEnv(t *testing.T, cfgBody string) *env {
	t.Helper()
	endpoint := os.Getenv("MCPX_DAEMON_ENDPOINT")
	dir := testsupport.TempDir(t)
	fake := testsupport.FakeMCPBinary(t)
	mcpx := testsupport.MCPXBinary(t)

	if strings.Contains(cfgBody, `"mcpServers"`) {
		t.Logf("remote mode: the local config's servers are not transplanted to the shared daemon; add any server the test needs with addRemoteServer")
	}
	if err := os.WriteFile(filepath.Join(dir, ".mcpx.json"), []byte("{}\n"), 0o644); err != nil {
		t.Fatal(err)
	}

	e := &env{
		t:      t,
		dir:    dir,
		mcpx:   mcpx,
		fake:   fake,
		runner: binaryRunner{},
		// The child environment is built from the same allowlist as the
		// local harness (see newEnv); the endpoint is the one value that
		// differs, chosen by the operator who launched the run, never
		// inherited from the caller's shell.
		envVars: testsupport.ChildEnv(os.Environ(), dir,
			"MCPX_REGISTRY_URL="+stubRegistry(t),
			"HOME="+filepath.Join(dir, "home"),
			"DENO_DIR="+toolCache(t, "DENO_DIR", "Library", "Caches", "deno"),
			"BUN_INSTALL_CACHE_DIR="+toolCache(t, "BUN_INSTALL_CACHE_DIR", ".bun", "install", "cache"),
			"MCPX_STATE_DIR="+filepath.Join(dir, "state"),
			"MCPX_CACHE_DIR="+filepath.Join(dir, "cache"),
			"MCPX_CONFIG="+filepath.Join(dir, ".mcpx.json"),
			"MCPX_DAEMON_ENDPOINT="+endpoint,
			// Same reasoning as newEnv: the write policy gate defaults to
			// ask, and no human answers for a test child.
			"MCPX_POLICY_WRITE=allow",
		),
	}
	recordSettingProbes(t)
	// No cleanup: the daemon is shared and outlives the test. Servers a
	// test added with addRemoteServer remove themselves in its cleanup.
	return e
}

// addRemoteServer registers a server on the remote daemon for this test,
// under the requested name plus a per-test suffix, so tests sharing one
// daemon cannot collide, and removes it in cleanup. The command or URL in
// body must exist on the daemon's host — a host-local fixture path does
// not. body is the POST /v1/servers request minus the name, which this
// function owns. It returns the registered name.
func addRemoteServer(t *testing.T, name string, body map[string]any) string {
	t.Helper()
	if !remoteMode() {
		t.Fatalf("addRemoteServer called outside remote mode")
	}
	endpoint := strings.TrimSuffix(os.Getenv("MCPX_DAEMON_ENDPOINT"), "/")
	full := name + "-" + remoteNameSuffix(t)

	payload := make(map[string]any, len(body)+1)
	for k, v := range body {
		payload[k] = v
	}
	payload["name"] = full
	raw, err := json.Marshal(payload)
	if err != nil {
		t.Fatal(err)
	}
	req, err := http.NewRequest(http.MethodPost, endpoint+"/v1/servers", bytes.NewReader(raw))
	if err != nil {
		t.Fatal(err)
	}
	req.Header.Set("Content-Type", "application/json")
	resp, err := http.DefaultClient.Do(req)
	if err != nil {
		t.Fatalf("add remote server %s: %v", full, err)
	}
	defer resp.Body.Close()
	if resp.StatusCode < 200 || resp.StatusCode > 299 {
		msg, _ := io.ReadAll(io.LimitReader(resp.Body, 4096))
		t.Fatalf("add remote server %s: %s: %s", full, resp.Status, strings.TrimSpace(string(msg)))
	}
	t.Cleanup(func() {
		req, err := http.NewRequest(http.MethodDelete, endpoint+"/v1/servers/"+url.PathEscape(full), nil)
		if err != nil {
			return
		}
		resp, err := http.DefaultClient.Do(req)
		if err != nil {
			t.Logf("cleanup: remove remote server %s: %v", full, err)
			return
		}
		resp.Body.Close()
	})
	return full
}

// remoteNameSuffix turns a test name into a server-name-safe suffix:
// lowercase alphanumerics and single dashes.
func remoteNameSuffix(t *testing.T) string {
	t.Helper()
	var b strings.Builder
	lastDash := false
	for _, r := range strings.ToLower(t.Name()) {
		switch {
		case r >= 'a' && r <= 'z' || r >= '0' && r <= '9':
			b.WriteRune(r)
			lastDash = false
		default:
			if !lastDash {
				b.WriteByte('-')
				lastDash = true
			}
		}
	}
	return strings.Trim(b.String(), "-")
}

func TestRemoteSafeSelectionFailsClosed(t *testing.T) {
	names := remoteSafeTests()
	if len(names) == 0 {
		t.Fatal("remote_safe_tests.txt is empty: the remote subset must name at least its canonical member")
	}
	seen := make(map[string]bool, len(names))
	for _, name := range names {
		if seen[name] {
			t.Errorf("remote_safe_tests.txt lists %s twice", name)
		}
		seen[name] = true
		if !testFunctionExists(name) {
			t.Errorf("remote_safe_tests.txt lists %s, but no such test function exists in this package", name)
		}
	}
	if !seen["TestExecOnTheDaemonRunsWhereTheDaemonIs"] {
		t.Error("remote_safe_tests.txt must list TestExecOnTheDaemonRunsWhereTheDaemonIs, the canonical member named by #13386")
	}

	pattern := regexp.MustCompile(remoteRunPattern())
	for _, name := range names {
		if !pattern.MatchString(name) {
			t.Errorf("run pattern %q does not match registered test %s", remoteRunPattern(), name)
		}
		if !remoteSafeTest(name) {
			t.Errorf("remoteSafeTest(%s) = false for a registered test", name)
		}
	}

	// Fail closed: a real test that is not registered is excluded by both
	// the pattern the CI leg runs and the predicate newEnv checks.
	const undeclared = "TestLsStartsDaemonAndListsNamespaces"
	if pattern.MatchString(undeclared) {
		t.Errorf("run pattern %q admits undeclared test %s", remoteRunPattern(), undeclared)
	}
	if remoteSafeTest(undeclared) {
		t.Errorf("remoteSafeTest(%s) = true for an undeclared test", undeclared)
	}
	// A subtest path resolves to its top-level test's declaration.
	if !remoteSafeTest("TestExecOnTheDaemonRunsWhereTheDaemonIs/a_subtest") {
		t.Error("remoteSafeTest does not resolve a subtest path to its top-level test")
	}
}

// testFunctionExists reports whether a top-level test function with this
// name is defined in one of this package's test files.
func testFunctionExists(name string) bool {
	files, err := filepath.Glob("*_test.go")
	if err != nil {
		return false
	}
	needle := "func " + name + "("
	for _, file := range files {
		raw, err := os.ReadFile(file)
		if err != nil {
			continue
		}
		for _, line := range strings.Split(string(raw), "\n") {
			if strings.HasPrefix(line, needle) {
				return true
			}
		}
	}
	return false
}

func TestAddRemoteServerRegistersSuffixedAndRemovesOnCleanup(t *testing.T) {
	var mu sync.Mutex
	var posted map[string]any
	var deletedPath string
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		mu.Lock()
		defer mu.Unlock()
		switch r.Method {
		case http.MethodPost:
			if r.URL.Path != "/v1/servers" {
				t.Errorf("POST path = %s, want /v1/servers", r.URL.Path)
			}
			if err := json.NewDecoder(r.Body).Decode(&posted); err != nil {
				t.Errorf("decode POST body: %v", err)
			}
			w.WriteHeader(http.StatusCreated)
		case http.MethodDelete:
			deletedPath = r.URL.Path
			w.WriteHeader(http.StatusNoContent)
		default:
			t.Errorf("unexpected method %s", r.Method)
		}
	}))
	defer srv.Close()

	t.Setenv("MCPX_E2E_REMOTE", "1")
	t.Setenv("MCPX_DAEMON_ENDPOINT", srv.URL)

	var full string
	t.Run("register", func(t *testing.T) {
		full = addRemoteServer(t, "demo", map[string]any{"command": "some-daemon-side-command"})
	})

	mu.Lock()
	defer mu.Unlock()
	if posted == nil {
		t.Fatal("no POST reached the daemon")
	}
	if posted["name"] != full {
		t.Errorf("posted name = %v, want the registered name %v", posted["name"], full)
	}
	if posted["command"] != "some-daemon-side-command" {
		t.Errorf("posted command = %v, want the caller's body carried through", posted["command"])
	}
	if full == "demo" || !strings.HasPrefix(full, "demo-") {
		t.Errorf("registered name %q carries no per-test suffix", full)
	}
	if deletedPath != "/v1/servers/"+full {
		t.Errorf("cleanup DELETE path = %q, want /v1/servers/%s", deletedPath, full)
	}
}
