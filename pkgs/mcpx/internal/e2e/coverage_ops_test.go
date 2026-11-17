package e2e_test

import (
	"context"
	"encoding/json"
	"io"
	"net"
	"net/http"
	"strings"
	"testing"
	"time"
)

// TestPluginSessionLifecycleOverTheSocket exercises the plugin-session
// operations end to end through the coverage-recording client: open a
// session, see it listed, close it, and find it gone. The coverage
// guard (#1978) flagged plugin_session_close as never exercised by an
// integration test — the generated-command test reaches it in a
// subprocess, whose requests the ledger cannot see (#8457).
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

	req, err := http.NewRequest(http.MethodDelete, "http://mcpx/v1/plugin-sessions/"+id, nil)
	if err != nil {
		t.Fatal(err)
	}
	resp, err = client.Do(req)
	if err != nil {
		t.Fatal(err)
	}
	body, _ = io.ReadAll(resp.Body)
	_ = resp.Body.Close()
	if resp.StatusCode != http.StatusOK {
		t.Fatalf("close: status %d: %s", resp.StatusCode, body)
	}
	if !strings.Contains(string(body), id) {
		t.Fatalf("close: the answer is not the dropped record: %s", body)
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
// declares must be answered by the remove handler itself (its own 404
// naming the declaration), not by the router. The success path —
// deleting from the defining file and reloading — is covered in the
// daemon package (TestAPIRemoveDeletesFromTheDefiningFile); the guard
// (#1978) flagged the op as never exercised at this level (#8457).
func TestAPIsRemoveAnswersOverTheSocket(t *testing.T) {
	e := newEnv(t, "{}")
	e.run("ls") // anything that needs a daemon starts one
	client := e.socketClient(t)

	req, err := http.NewRequest(http.MethodDelete, "http://mcpx/v1/apis/coverage-probe", nil)
	if err != nil {
		t.Fatal(err)
	}
	resp, err := client.Do(req)
	if err != nil {
		t.Fatal(err)
	}
	body, _ := io.ReadAll(resp.Body)
	_ = resp.Body.Close()
	if resp.StatusCode != http.StatusNotFound {
		t.Fatalf("remove: status %d: %s", resp.StatusCode, body)
	}
	if !strings.Contains(string(body), "no configured apis file declares") || !strings.Contains(string(body), "coverage-probe") {
		t.Fatalf("remove: not the handler's answer: %s", body)
	}
}

// The #8466 ops group: the remaining /v1 operations with no
// integration-test exercise. Each test below drives its op through
// the coverage-recording client so the ledger gains a row, and
// asserts the handler's own answer — the same standard as the
// plugin-session and apis_remove exercises above.

func doOp(t *testing.T, client *http.Client, method, path, body string) (int, []byte) {
	t.Helper()
	var rdr io.Reader
	if body != "" {
		rdr = strings.NewReader(body)
	}
	req, err := http.NewRequest(method, "http://mcpx"+path, rdr)
	if err != nil {
		t.Fatal(err)
	}
	if body != "" {
		req.Header.Set("content-type", "application/json")
	}
	resp, err := client.Do(req)
	if err != nil {
		t.Fatal(err)
	}
	defer resp.Body.Close()
	out, _ := io.ReadAll(resp.Body)
	return resp.StatusCode, out
}

// TestOpsRefreshAndRestartOverTheSocket exercises refresh and
// restart against a daemon with an empty configuration: refresh
// re-reads the (empty) config and reports no errors, and a global
// restart with nothing running stops and starts nothing, cleanly.
func TestOpsRefreshAndRestartOverTheSocket(t *testing.T) {
	e := newEnv(t, "{}")
	e.run("ls")
	client := e.socketClient(t)

	code, body := doOp(t, client, http.MethodPost, "/v1/refresh", "")
	if code != http.StatusOK {
		t.Fatalf("refresh: status %d: %s", code, body)
	}
	var refreshed struct {
		Namespaces []string          `json:"namespaces"`
		Errors     map[string]string `json:"errors"`
	}
	if err := json.Unmarshal(body, &refreshed); err != nil {
		t.Fatalf("refresh: %v: %s", err, body)
	}
	if len(refreshed.Errors) != 0 {
		t.Fatalf("refresh of an empty config reported errors: %s", body)
	}

	code, body = doOp(t, client, http.MethodPost, "/v1/restart", `{}`)
	if code != http.StatusOK {
		t.Fatalf("restart: status %d: %s", code, body)
	}
	var restarted struct {
		Stopped int `json:"stopped"`
		Started int `json:"started"`
		Failed  int `json:"failed"`
	}
	if err := json.Unmarshal(body, &restarted); err != nil {
		t.Fatalf("restart: %v: %s", err, body)
	}
	if restarted.Stopped != 0 || restarted.Started != 0 || restarted.Failed != 0 {
		t.Fatalf("restart with nothing running: %s", body)
	}
}

// TestAuthzDecisionsClearOverTheSocket exercises
// authz_decisions_clear: the socket calls this test makes are
// themselves recorded decisions, so the report is non-empty before
// the clear and empty after it.
func TestAuthzDecisionsClearOverTheSocket(t *testing.T) {
	e := newEnv(t, "{}")
	e.run("ls")
	client := e.socketClient(t)

	report := func() int {
		code, body := doOp(t, client, http.MethodGet, "/v1/authz/decisions", "")
		if code != http.StatusOK {
			t.Fatalf("decisions report: status %d: %s", code, body)
		}
		var rep struct {
			Total int `json:"total"`
		}
		if err := json.Unmarshal(body, &rep); err != nil {
			t.Fatalf("decisions report: %v: %s", err, body)
		}
		return rep.Total
	}
	if report() == 0 {
		t.Fatal("no decisions recorded before the clear; the exercise premise is gone")
	}

	code, body := doOp(t, client, http.MethodDelete, "/v1/authz/decisions", "")
	if code != http.StatusOK {
		t.Fatalf("clear: status %d: %s", code, body)
	}
	var cleared struct {
		Total int `json:"total"`
	}
	if err := json.Unmarshal(body, &cleared); err != nil {
		t.Fatalf("clear: %v: %s", err, body)
	}
	if cleared.Total != 0 {
		t.Fatalf("clear answered with a non-empty report: %s", body)
	}
	if got := report(); got != 1 {
		// Only the report read itself has been recorded since.
		t.Fatalf("decisions after clear + one read = %d, want 1", got)
	}
}

// TestSecretsSetAndUnsetOverTheSocket exercises secrets_set and
// secrets_unset: a runtime value is given, listed as set from
// runtime, then forgotten and listed as unset. The value is a
// probe string for a name no configured server needs; it lives
// only in the test daemon's memory.
func TestSecretsSetAndUnsetOverTheSocket(t *testing.T) {
	e := newEnv(t, "{}")
	e.run("ls")
	client := e.socketClient(t)

	const name = "COVERAGE_PROBE_TOKEN"
	listed := func() (bool, string) {
		code, body := doOp(t, client, http.MethodGet, "/v1/secrets", "")
		if code != http.StatusOK {
			t.Fatalf("secrets list: status %d: %s", code, body)
		}
		var list struct {
			Secrets []struct {
				Name   string  `json:"name"`
				Set    bool    `json:"set"`
				Source *string `json:"source"`
			} `json:"secrets"`
		}
		if err := json.Unmarshal(body, &list); err != nil {
			t.Fatalf("secrets list: %v: %s", err, body)
		}
		for _, s := range list.Secrets {
			if s.Name == name {
				src := ""
				if s.Source != nil {
					src = *s.Source
				}
				return s.Set, src
			}
		}
		return false, ""
	}

	code, body := doOp(t, client, http.MethodPut, "/v1/secrets/"+name, `{"value":"coverage-probe-value"}`)
	if code != http.StatusOK {
		t.Fatalf("secrets set: status %d: %s", code, body)
	}
	if set, src := listed(); !set || src != "runtime" {
		t.Fatalf("after set: listed set=%v source=%q, want true/runtime", set, src)
	}

	code, body = doOp(t, client, http.MethodDelete, "/v1/secrets/"+name, "")
	if code != http.StatusOK {
		t.Fatalf("secrets unset: status %d: %s", code, body)
	}
	var unset struct {
		Name string `json:"name"`
		Set  bool   `json:"set"`
	}
	if err := json.Unmarshal(body, &unset); err != nil {
		t.Fatalf("secrets unset: %v: %s", err, body)
	}
	if unset.Name != name || unset.Set {
		t.Fatalf("secrets unset answered: %s", body)
	}
	if set, _ := listed(); set {
		t.Fatal("secret still listed as set after unset")
	}
}

// TestSecretRequestDeclineUnknownOverTheSocket exercises
// secret_request_decline against an id no request carries: the
// decline handler itself answers 404 naming the id, not the
// router. Declining a genuinely pending request is covered in the
// daemon package; the guard (#1978) flagged the op as never
// exercised at this level (#8466).
func TestSecretRequestDeclineUnknownOverTheSocket(t *testing.T) {
	e := newEnv(t, "{}")
	e.run("ls")
	client := e.socketClient(t)

	code, body := doOp(t, client, http.MethodDelete, "/v1/secret-requests/coverage-probe", "")
	if code != http.StatusNotFound {
		t.Fatalf("decline unknown: status %d: %s", code, body)
	}
	if !strings.Contains(string(body), "coverage-probe") {
		t.Fatalf("decline unknown: the answer does not name the id: %s", body)
	}
}

// TestAdaptersRemoveUnknownOverTheSocket exercises
// adapters_remove against a name no configured adapters file
// declares: the remove handler itself answers 404 naming the
// declaration. The success path — deleting from the defining
// file and reloading — is covered in the daemon package.
func TestAdaptersRemoveUnknownOverTheSocket(t *testing.T) {
	e := newEnv(t, "{}")
	e.run("ls")
	client := e.socketClient(t)

	code, body := doOp(t, client, http.MethodDelete, "/v1/adapters/coverage-probe", "")
	if code != http.StatusNotFound {
		t.Fatalf("adapters remove unknown: status %d: %s", code, body)
	}
	if !strings.Contains(string(body), "coverage-probe") {
		t.Fatalf("adapters remove unknown: the answer does not name the adapter: %s", body)
	}
}

// TestDaemonRestartOverTheSocket exercises daemon_restart: the
// handler answers "restarting", re-executes the daemon process,
// and the successor answers on the socket again. The successor is
// then shut down over the socket: a restarted daemon has removed
// its info file before re-executing, so the harness's stop --all
// cleanup cannot find it, and leaving it running would leak it
// past the suite's process sweep (#1971).
func TestDaemonRestartOverTheSocket(t *testing.T) {
	e := newEnv(t, "{}")
	e.run("ls")

	var st struct {
		Socket string `json:"socket"`
	}
	if err := json.Unmarshal([]byte(jsonOf(t, e.run("--json", "status"))), &st); err != nil {
		t.Fatal(err)
	}
	if st.Socket == "" {
		t.Fatal("status did not report a socket")
	}
	client := e.socketClient(t)

	code, body := doOp(t, client, http.MethodPost, "/v1/daemon/restart", "")
	if code != http.StatusOK {
		t.Fatalf("daemon restart: status %d: %s", code, body)
	}
	if !strings.Contains(string(body), `"restarting"`) {
		t.Fatalf("daemon restart answered: %s", body)
	}

	deadline := time.Now().Add(60 * time.Second)
	for {
		req, err := http.NewRequest(http.MethodGet, "http://mcpx/v1/status", nil)
		if err != nil {
			t.Fatal(err)
		}
		resp, err := client.Do(req)
		if err == nil {
			_ = resp.Body.Close()
			if resp.StatusCode == http.StatusOK {
				break
			}
		}
		if time.Now().After(deadline) {
			t.Fatal("the daemon did not answer on its socket within 60s of the restart")
		}
		time.Sleep(250 * time.Millisecond)
	}

	// The successor is a working daemon: it answers a shutdown and
	// leaves its socket.
	code, body = doOp(t, client, http.MethodPost, "/v1/shutdown", "")
	if code != http.StatusOK {
		t.Fatalf("shutdown of the restarted daemon: status %d: %s", code, body)
	}
	deadline = time.Now().Add(120 * time.Second)
	for {
		conn, err := (&net.Dialer{}).DialContext(context.Background(), "unix", st.Socket)
		if err != nil {
			return // the restarted daemon is gone from its socket
		}
		_ = conn.Close()
		if time.Now().After(deadline) {
			t.Fatal("the restarted daemon still answers on its socket 120s after shutdown")
		}
		time.Sleep(250 * time.Millisecond)
	}
}

// TestShutdownOverTheSocket exercises shutdown: the handler
// answers "stopping" and the daemon then stops answering on its
// socket. The socket path is captured before the call, because
// asking the CLI afterwards would start a fresh daemon and mask
// the stop.
func TestShutdownOverTheSocket(t *testing.T) {
	e := newEnv(t, "{}")
	e.run("ls")

	var st struct {
		Socket string `json:"socket"`
	}
	if err := json.Unmarshal([]byte(jsonOf(t, e.run("--json", "status"))), &st); err != nil {
		t.Fatal(err)
	}
	if st.Socket == "" {
		t.Fatal("status did not report a socket")
	}
	client := e.socketClient(t)

	code, body := doOp(t, client, http.MethodPost, "/v1/shutdown", "")
	if code != http.StatusOK {
		t.Fatalf("shutdown: status %d: %s", code, body)
	}
	if !strings.Contains(string(body), `"stopping"`) {
		t.Fatalf("shutdown answered: %s", body)
	}

	deadline := time.Now().Add(120 * time.Second)
	for {
		conn, err := (&net.Dialer{}).DialContext(context.Background(), "unix", st.Socket)
		if err != nil {
			return // the daemon is gone from its socket
		}
		_ = conn.Close()
		if time.Now().After(deadline) {
			t.Fatal("the daemon still answers on its socket 120s after shutdown")
		}
		time.Sleep(250 * time.Millisecond)
	}
}
