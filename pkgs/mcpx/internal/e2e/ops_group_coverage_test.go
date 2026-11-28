package e2e_test

// Integration-test exercises for the /v1 ops group (#8466): adapters_remove,
// authz_decisions_clear, daemon_restart, refresh, restart,
// secret_request_decline, secrets_set, secrets_unset and shutdown.
//
// Each test drives the real route over the daemon's socket, so the
// feature-coverage ledger gains an `op` row for it; that row is what lets the
// matching line come out of feature_coverage_exemptions.txt (the guard fails
// on an exemption for a covered feature, so the deletions are
// self-verifying).
//
// Careful: an exercise must not record ledger rows for features that still
// carry exemptions of their own. Only ops are exercised here — every other
// `op` already has a ledger row — and the CLI invocations are kept to
// commands that are already covered (`call`, `ls`, `status`, `stop`). The
// `adapter` and `daemon` CLI commands belong to other exemption groups and are
// deliberately not driven.

import (
	"encoding/json"
	"io"
	"net/http"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"github.com/developing-today/code/pkgs/mcpx/internal/testsupport"
)

// doJSON sends one request over the coverage-recording socket client and
// returns the status and decoded body. The coverage row is recorded by the
// client's transport on any answer, so a refusal counts as an exercise too.
func doJSON(t *testing.T, c *http.Client, method, path, body string) (int, map[string]any) {
	t.Helper()
	var reader io.Reader
	if body != "" {
		reader = strings.NewReader(body)
	}
	req, err := http.NewRequest(method, "http://mcpx"+path, reader)
	if err != nil {
		t.Fatal(err)
	}
	if body != "" {
		req.Header.Set("Content-Type", "application/json")
	}
	resp, err := c.Do(req)
	if err != nil {
		t.Fatalf("%s %s: %v", method, path, err)
	}
	defer func() { _ = resp.Body.Close() }()
	raw, _ := io.ReadAll(resp.Body)
	var doc map[string]any
	_ = json.Unmarshal(raw, &doc)
	return resp.StatusCode, doc
}

// TestAdaptersRemoveDeletesTheDeclarationOverTheSocket exercises
// adapters_remove through the coverage-recording client: the declaration is
// removed from the file that defines it, the answer names that file, and the
// adapter is gone from the registry afterwards.
func TestAdaptersRemoveDeletesTheDeclarationOverTheSocket(t *testing.T) {
	e := newEnv(t, oneServer)
	spec := filepath.Join(e.dir, "adapters.json")
	if err := os.WriteFile(spec, []byte(`{"adapters":[{"name":"greet","command":"echo",
      "tools":[{"name":"once","args":["hello"]}]}]}`), 0o644); err != nil {
		t.Fatal(err)
	}
	e.envVars = append(e.envVars, "MCPX_PATHS_ADAPTERS="+spec)
	e.run("call", "demo.echo", `{"message":"warm"}`) // starts the daemon and its demo child
	retireSupervisedChild(t, e)
	client := e.socketClient(t)

	status, doc := doJSON(t, client, http.MethodDelete, "/v1/adapters/greet", "")
	if status != http.StatusOK {
		t.Fatalf("remove: status %d: %v", status, doc)
	}
	if doc["name"] != "greet" {
		t.Fatalf("remove: the answer should name what it removed: %v", doc)
	}
	if doc["written"] != spec {
		t.Fatalf("remove: written = %v, want the declaring file %s", doc["written"], spec)
	}
	if removed, _ := doc["removed"].([]any); len(removed) == 0 {
		t.Fatalf("remove: the reload should have dropped the adapter: %v", doc)
	}

	// The file no longer declares it, so a second remove is the handler's
	// own 404 rather than a silent success.
	body, err := os.ReadFile(spec)
	if err != nil {
		t.Fatal(err)
	}
	if strings.Contains(string(body), `"greet"`) {
		t.Fatalf("remove: the declaration is still in %s:\n%s", spec, body)
	}
	status, doc = doJSON(t, client, http.MethodDelete, "/v1/adapters/greet", "")
	if status != http.StatusNotFound {
		t.Fatalf("second remove: status %d, want 404: %v", status, doc)
	}
}

// TestAuthzDecisionsClearEmptiesTheRecordOverTheSocket exercises
// authz_decisions_clear: the answer is the now-empty report, and the next
// read covers only what happened after the clear.
func TestAuthzDecisionsClearEmptiesTheRecordOverTheSocket(t *testing.T) {
	e := newEnv(t, oneServer)
	e.run("call", "demo.echo", `{"message":"warm"}`)
	retireSupervisedChild(t, e)
	client := e.socketClient(t)

	// Something to clear: reading the report is itself a recorded decision.
	if status, doc := doJSON(t, client, http.MethodGet, "/v1/authz/decisions", ""); status != http.StatusOK {
		t.Fatalf("report: status %d: %v", status, doc)
	}

	status, doc := doJSON(t, client, http.MethodDelete, "/v1/authz/decisions", "")
	if status != http.StatusOK {
		t.Fatalf("clear: status %d: %v", status, doc)
	}
	if total, _ := doc["total"].(float64); total != 0 {
		t.Fatalf("clear: the answer should be the empty report, got total %v: %v", doc["total"], doc)
	}
	if doc["stage"] != "permissive" {
		t.Fatalf("clear: the report should still name its stage: %v", doc)
	}

	// After the clear, the record starts again from this read.
	if status, doc := doJSON(t, client, http.MethodGet, "/v1/authz/decisions", ""); status != http.StatusOK {
		t.Fatalf("report after clear: status %d: %v", status, doc)
	} else if total, _ := doc["total"].(float64); total == 0 {
		t.Fatalf("a read after the clear should be recorded again: %v", doc)
	}
}

// TestRefreshPicksUpAHandEditedConfigOverTheSocket exercises refresh: a
// server added to the configuration file after the daemon started becomes
// visible without restarting the daemon.
func TestRefreshPicksUpAHandEditedConfigOverTheSocket(t *testing.T) {
	e := newEnv(t, oneServer)
	// With the config watcher on (its default) the edit is picked up before
	// the refresh arrives, so "added" would be empty for a reason that has
	// nothing to do with the route.
	e.envVars = append(e.envVars, "MCPX_DAEMON_WATCH_CONFIG=false")
	// A daemon has to exist before the edit, or it starts afterwards and
	// reads the edited file at startup, leaving "added" empty for a reason
	// that has nothing to do with the route. `ls` alone may not start one.
	e.run("call", "demo.echo", `{"message":"warm"}`)
	if e.daemonState(t).PID == 0 {
		t.Fatal("no daemon before the edit")
	}
	retireSupervisedChild(t, e)
	client := e.socketClient(t)

	cfgPath := filepath.Join(e.dir, ".mcpx.json")
	body, err := os.ReadFile(cfgPath)
	if err != nil {
		t.Fatal(err)
	}
	var cfg map[string]any
	if err := json.Unmarshal(body, &cfg); err != nil {
		t.Fatal(err)
	}
	servers, _ := cfg["mcpServers"].(map[string]any)
	// An env entry makes this a different process identity from demo: two
	// entries with identical commands are one pool, and a shared pool is
	// not reported as added.
	servers["extra"] = map[string]any{"command": e.fake, "env": map[string]any{"MCPX_E2E_EXTRA": "1"}}
	if err := os.WriteFile(cfgPath, mustJSON(t, cfg), 0o644); err != nil {
		t.Fatal(err)
	}

	status, doc := doJSON(t, client, http.MethodPost, "/v1/refresh", "")
	if status != http.StatusOK {
		t.Fatalf("refresh: status %d: %v", status, doc)
	}
	added, _ := doc["added"].([]any)
	var names []string
	for _, a := range added {
		if s, ok := a.(string); ok {
			names = append(names, s)
		}
	}
	if !strings.Contains(strings.Join(names, ","), "extra") {
		t.Fatalf("refresh should report the hand-added server as added, got %v: %v", names, doc)
	}
	if ns, _ := doc["namespaces"].([]any); len(ns) == 0 {
		t.Fatalf("refresh should report the namespaces it warmed: %v", doc)
	}

	// The refreshed daemon can call it: the reload reached the registry, not
	// only the answer.
	if out := e.run("call", "extra.echo", `{"message":"after-refresh"}`); !strings.Contains(out, "after-refresh") {
		t.Fatalf("the hand-added server is callable after refresh:\n%s", out)
	}
	retireSupervisedChild(t, e, "demo", "extra")
}

// TestRestartReplacesTheServerInstancesOverTheSocket exercises restart: the
// running instance is stopped and a replacement answers with the same call.
func TestRestartReplacesTheServerInstancesOverTheSocket(t *testing.T) {
	e := newEnv(t, oneServer)
	e.run("call", "demo.echo", `{"message":"before"}`)
	retireSupervisedChild(t, e)
	client := e.socketClient(t)

	status, doc := doJSON(t, client, http.MethodPost, "/v1/restart", `{"server":"demo"}`)
	if status != http.StatusOK {
		t.Fatalf("restart: status %d: %v", status, doc)
	}
	stopped, _ := doc["stopped"].(float64)
	started, _ := doc["started"].(float64)
	failed, _ := doc["failed"].(float64)
	if stopped < 1 || started < 1 || failed != 0 {
		t.Fatalf("restart should stop one and start its replacement: %v", doc)
	}
	if out := e.run("call", "demo.echo", `{"message":"after"}`); !strings.Contains(out, "after") {
		t.Fatalf("the replacement should answer the same call:\n%s", out)
	}
	retireSupervisedChild(t, e)

	// Unknown server names are refused by the handler, not by the router.
	status, doc = doJSON(t, client, http.MethodPost, "/v1/restart", `{"server":"nowhere"}`)
	if status != http.StatusBadRequest {
		t.Fatalf("restart of an unknown server: status %d, want 400: %v", status, doc)
	}
}

// TestSecretSetAndUnsetOverTheSocket exercises secrets_set and
// secrets_unset through the coverage-recording client: the value is accepted
// and held at runtime, the listing says where it came from, and unsetting
// returns the name to unset without touching the environment.
func TestSecretSetAndUnsetOverTheSocket(t *testing.T) {
	// Quarantined per #1973: intermittent hang on aarch64-darwin in the
	// daemon config-reload path (#16312); secret set/unset exercises the
	// reload path and the test binary times out at 30m. Root-cause stays
	// open on #16312 — this quarantine only stops the CI timeouts meanwhile.
	testsupport.Quarantine(t, "https://github.com/developing-today/code/issues/16312", "2026-12-31")
	e := newEnv(t, secretServer)
	e.run("ls")
	client := e.socketClient(t)

	status, doc := doJSON(t, client, http.MethodPut, "/v1/secrets/MCPX_E2E_TOKEN", `{"value":"`+e2eValue+`"}`)
	if status != http.StatusOK {
		t.Fatalf("set: status %d: %v", status, doc)
	}
	if doc["set"] != true || doc["source"] != "runtime" {
		t.Fatalf("set: the answer should say the value is held at runtime: %v", doc)
	}

	status, doc = doJSON(t, client, http.MethodGet, "/v1/secrets", "")
	if status != http.StatusOK {
		t.Fatalf("list: status %d: %v", status, doc)
	}
	if raw := mustJSON(t, doc); strings.Contains(string(raw), e2eValue) {
		t.Fatalf("the listing must never carry the value:\n%s", raw)
	}

	// Unset: the runtime value is gone, and nothing in the environment
	// answers for the name.
	status, doc = doJSON(t, client, http.MethodDelete, "/v1/secrets/MCPX_E2E_TOKEN", "")
	if status != http.StatusOK {
		t.Fatalf("unset: status %d: %v", status, doc)
	}
	if doc["set"] != false {
		t.Fatalf("unset: the answer should say nothing answers for the name: %v", doc)
	}

	// The value is refused when empty, by the handler, with a 400.
	status, doc = doJSON(t, client, http.MethodPut, "/v1/secrets/MCPX_E2E_TOKEN", `{"value":""}`)
	if status != http.StatusBadRequest {
		t.Fatalf("set with an empty value: status %d, want 400: %v", status, doc)
	}
}

// TestSecretRequestDeclineOverTheSocket exercises secret_request_decline: a
// request made by a refused call is declined, and a caller polling it sees
// the decline instead of a pending wait.
func TestSecretRequestDeclineOverTheSocket(t *testing.T) {
	e := newEnv(t, secretServer)
	if _, err := e.try("call", "demo.echo", `{"message":"x"}`); err == nil {
		t.Fatal("a call to a server waiting for its secret must be refused")
	}
	client := e.socketClient(t)

	status, doc := doJSON(t, client, http.MethodGet, "/v1/secret-requests", "")
	if status != http.StatusOK {
		t.Fatalf("requests: status %d: %v", status, doc)
	}
	requests, _ := doc["requests"].([]any)
	if len(requests) == 0 {
		t.Fatalf("the refused call should have asked for its secret: %v", doc)
	}
	first, _ := requests[0].(map[string]any)
	id, _ := first["id"].(string)
	if id == "" {
		t.Fatalf("a secret request must carry an id: %v", first)
	}

	status, doc = doJSON(t, client, http.MethodDelete, "/v1/secret-requests/"+id, "")
	if status != http.StatusOK {
		t.Fatalf("decline: status %d: %v", status, doc)
	}
	if doc["status"] != "declined" {
		t.Fatalf("decline: the answer should be the declined request: %v", doc)
	}

	// Polling the request now reports the decline; an id nothing asked for
	// is the handler's own 404.
	status, doc = doJSON(t, client, http.MethodGet, "/v1/secret-requests/"+id, "")
	if status != http.StatusOK || doc["status"] != "declined" {
		t.Fatalf("get after decline: status %d: %v", status, doc)
	}
	status, doc = doJSON(t, client, http.MethodDelete, "/v1/secret-requests/sr-nothing", "")
	if status != http.StatusNotFound {
		t.Fatalf("decline of an unknown request: status %d, want 404: %v", status, doc)
	}
}

// healthOf reads /v1/health over the socket. ok is false while the socket is
// between a re-exec tearing it down and the successor binding it again.
func healthOf(c *http.Client) (pid int, uptime time.Duration, ok bool) {
	resp, err := c.Get("http://mcpx/v1/health")
	if err != nil {
		return 0, 0, false
	}
	defer func() { _ = resp.Body.Close() }()
	var doc struct {
		PID    int    `json:"pid"`
		Uptime string `json:"uptime"`
	}
	if json.NewDecoder(resp.Body).Decode(&doc) != nil {
		return 0, 0, false
	}
	d, err := time.ParseDuration(doc.Uptime)
	if err != nil {
		return 0, 0, false
	}
	return doc.PID, d, true
}

// TestDaemonRestartReexecutesTheDaemonOverTheSocket exercises daemon_restart:
// the route answers first, then the daemon re-executes in place. exec keeps
// the pid, so the replacement shows as an uptime that starts again from zero.
func TestDaemonRestartReexecutesTheDaemonOverTheSocket(t *testing.T) {
	e := newEnv(t, oneServer)
	e.run("call", "demo.echo", `{"message":"before"}`)
	retireSupervisedChild(t, e)
	client := e.socketClient(t)
	time.Sleep(3 * time.Second) // so the uptime before the restart is well above zero
	pid, before, ok := healthOf(client)
	if !ok || pid == 0 {
		t.Fatal("no health answer before the restart")
	}

	status, doc := doJSON(t, client, http.MethodPost, "/v1/daemon/restart", "")
	if status != http.StatusOK {
		t.Fatalf("daemon restart: status %d: %v", status, doc)
	}
	if doc["status"] != "restarting" {
		t.Fatalf("daemon restart: the answer should say it is restarting: %v", doc)
	}

	deadline := time.Now().Add(30 * time.Second)
	for {
		if p, up, ok := healthOf(client); ok && up < before {
			if p != pid {
				t.Fatalf("a re-exec keeps the pid: %d -> %d", pid, p)
			}
			break
		}
		if time.Now().After(deadline) {
			t.Fatalf("the daemon was never re-executed (pid %d, uptime still past %v)", pid, before)
		}
		time.Sleep(50 * time.Millisecond)
	}
	if out := e.run("call", "demo.echo", `{"message":"after"}`); !strings.Contains(out, "after") {
		t.Fatalf("the re-executed daemon should answer the same call:\n%s", out)
	}
}

// TestShutdownStopsTheDaemonOverTheSocket exercises shutdown: the route
// answers, then the daemon process ends and its socket stops answering. The
// supervised child is deliberately NOT asserted gone: its supervisor outlives
// the daemon by design (#9962), so a child outliving the daemon is the
// documented behaviour, not a leak.
func TestShutdownStopsTheDaemonOverTheSocket(t *testing.T) {
	e := newEnv(t, oneServer)
	e.run("call", "demo.echo", `{"message":"x"}`)
	retireSupervisedChild(t, e)
	before := e.daemonState(t)
	if before.PID == 0 {
		t.Fatal("no daemon pid before the shutdown")
	}
	client := e.socketClient(t)

	status, doc := doJSON(t, client, http.MethodPost, "/v1/shutdown", "")
	if status != http.StatusOK {
		t.Fatalf("shutdown: status %d: %v", status, doc)
	}
	if doc["status"] != "stopping" {
		t.Fatalf("shutdown: the answer should say it is stopping: %v", doc)
	}

	awaitExit(t, before.PID, "the daemon")
	// The socket is removed on the way out, so nothing can reach the world
	// the daemon was serving.
	if _, err := client.Get("http://mcpx/v1/health"); err == nil {
		t.Fatal("the socket still answers after the daemon exited")
	}
}

// retireSupervisedChild kills the supervised demo child a call started. The
// child outlives every daemon by design, so the test that starts one owns
// retiring it, as takeover_test.go does.
func retireSupervisedChild(t *testing.T, e *env, namespaces ...string) {
	t.Helper()
	if len(namespaces) == 0 {
		namespaces = []string{"demo"}
	}
	state := e.daemonState(t)
	for _, ns := range namespaces {
		child := state.childOf(ns)
		t.Cleanup(func() {
			if child != 0 && processExists(child) {
				if proc, err := os.FindProcess(child); err == nil {
					_ = proc.Kill()
				}
			}
		})
	}
}

// mustJSON encodes v for a fixture write; a fixture that will not encode is a
// test bug, not a product behaviour.
func mustJSON(t *testing.T, v any) []byte {
	t.Helper()
	b, err := json.MarshalIndent(v, "", "  ")
	if err != nil {
		t.Fatal(err)
	}
	return b
}

// TestPluginSessionLifecycleOverTheSocket exercises plugin_session_close
// through the coverage-recording client: open a session, close it, and
// find it gone. The generated-command test reaches the op only in a
// subprocess, whose requests the ledger cannot see. (Restored: #8466
// dropped coverage_ops_test.go, which carried this exercise, and the op
// lost its ledger row.)
func TestPluginSessionLifecycleOverTheSocket(t *testing.T) {
	e := newEnv(t, "{}")
	e.run("ls") // anything that needs a daemon starts one
	client := e.socketClient(t)

	resp, err := client.Post("http://mcpx/v1/plugin-sessions", "application/json",
		strings.NewReader(`{"plugin":"coverage-probe","capabilities":["probe"],"hooks":[]}`))
	if err != nil {
		t.Fatal(err)
	}
	body, _ := io.ReadAll(resp.Body)
	_ = resp.Body.Close()
	if resp.StatusCode != http.StatusCreated {
		t.Fatalf("open: status %d: %s", resp.StatusCode, body)
	}
	var opened struct {
		Session struct {
			ID     string `json:"id"`
			Plugin string `json:"plugin"`
		} `json:"session"`
	}
	if err := json.Unmarshal(body, &opened); err != nil {
		t.Fatalf("open: %v: %s", err, body)
	}
	if opened.Session.ID == "" || opened.Session.Plugin != "coverage-probe" {
		t.Fatalf("open: unexpected session record: %s", body)
	}
	id := opened.Session.ID

	status, closed := deleteOverSocket(t, client, "/v1/plugin-sessions/"+id)
	if status != http.StatusOK {
		t.Fatalf("close: status %d: %s", status, closed)
	}
	if !strings.Contains(closed, id) {
		t.Fatalf("close: the answer is not the dropped record: %s", closed)
	}

	resp, err = client.Get("http://mcpx/v1/plugin-sessions/" + id)
	if err != nil {
		t.Fatal(err)
	}
	_ = resp.Body.Close()
	if resp.StatusCode != http.StatusNotFound {
		t.Fatalf("get after close: status %d, want 404", resp.StatusCode)
	}
}

// TestAPIsRemoveAnswersOverTheSocket exercises apis_remove through the
// coverage-recording client: removing a name no configured apis file
// declares is answered by the remove handler itself (its own 404 naming
// the declaration), not by the router. The success path is covered in
// the daemon package (TestAPIRemoveDeletesFromTheDefiningFile).
// (Restored with the plugin-session exercise above.)
func TestAPIsRemoveAnswersOverTheSocket(t *testing.T) {
	e := newEnv(t, "{}")
	e.run("ls") // anything that needs a daemon starts one
	client := e.socketClient(t)

	status, body := deleteOverSocket(t, client, "/v1/apis/coverage-probe")
	if status != http.StatusNotFound {
		t.Fatalf("remove: status %d: %s", status, body)
	}
	if !strings.Contains(body, "no configured apis file declares") || !strings.Contains(body, "coverage-probe") {
		t.Fatalf("remove: not the handler's answer: %s", body)
	}
}

// TestRecipeRemoveDeletesTheRecipeOverTheSocket exercises recipe_remove:
// a recipe file the daemon can see is deleted by the route, the answer
// names the file, and a second remove is the handler's own 404.
func TestRecipeRemoveDeletesTheRecipeOverTheSocket(t *testing.T) {
	e := newEnv(t, "{}")
	writeRecipe(t, e, "doomed", "// A recipe that is only ever removed.\nconsole.log(\"never\");\n")
	path := filepath.Join(e.dir, ".config", "mcpx", "scripts", "doomed.ts")
	e.run("ls") // anything that needs a daemon starts one
	client := e.socketClient(t)

	status, body := deleteOverSocket(t, client, "/v1/recipes/doomed")
	if status != http.StatusOK {
		t.Fatalf("remove: status %d: %s", status, body)
	}
	if !strings.Contains(body, "doomed.ts") {
		t.Fatalf("remove: the answer does not name the removed file: %s", body)
	}
	if _, err := os.Stat(path); !os.IsNotExist(err) {
		t.Fatalf("remove: %s is still there (stat err %v)", path, err)
	}

	status, body = deleteOverSocket(t, client, "/v1/recipes/doomed")
	if status != http.StatusNotFound || !strings.Contains(body, "no recipe named") {
		t.Fatalf("second remove: status %d, want the handler's 404: %s", status, body)
	}
}

func deleteOverSocket(t *testing.T, client *http.Client, path string) (int, string) {
	t.Helper()
	req, err := http.NewRequest(http.MethodDelete, "http://mcpx"+path, nil)
	if err != nil {
		t.Fatal(err)
	}
	resp, err := client.Do(req)
	if err != nil {
		t.Fatal(err)
	}
	body, _ := io.ReadAll(resp.Body)
	_ = resp.Body.Close()
	return resp.StatusCode, string(body)
}
