package e2e_test

import (
	"encoding/json"
	"io"
	"net/http"
	"strings"
	"testing"
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

// TestAdaptersRemoveAnswersOverTheSocket exercises adapters_remove
// through the coverage-recording client: removing a name no configured
// adapters file declares must be answered by the remove handler itself
// (its own 404 naming the declaration), not by the router. The success
// path — deleting from the defining file and reloading — is covered in
// the daemon package; the guard (#1978) flagged the op as never
// exercised at this level (#8466).
func TestAdaptersRemoveAnswersOverTheSocket(t *testing.T) {
	e := newEnv(t, "{}")
	e.run("ls") // anything that needs a daemon starts one
	client := e.socketClient(t)

	req, err := http.NewRequest(http.MethodDelete, "http://mcpx/v1/adapters/coverage-probe", nil)
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
	if !strings.Contains(string(body), "no configured adapters file declares") || !strings.Contains(string(body), "coverage-probe") {
		t.Fatalf("remove: not the handler's answer: %s", body)
	}
}

// TestRefreshAnswersOverTheSocket exercises refresh through the
// coverage-recording client: a refresh re-reads the configuration and
// re-warms every namespace, answering with the namespace list and the
// reload delta. On an empty configuration the delta is empty and no
// namespace reports a warm error. The guard (#1978) flagged the op as
// never exercised at this level (#8466).
func TestRefreshAnswersOverTheSocket(t *testing.T) {
	e := newEnv(t, "{}")
	e.run("ls") // anything that needs a daemon starts one
	client := e.socketClient(t)

	resp, err := client.Post("http://mcpx/v1/refresh", "application/json", strings.NewReader(`{}`))
	if err != nil {
		t.Fatal(err)
	}
	body, _ := io.ReadAll(resp.Body)
	_ = resp.Body.Close()
	if resp.StatusCode != http.StatusOK {
		t.Fatalf("refresh: status %d: %s", resp.StatusCode, body)
	}
	var out struct {
		Namespaces []json.RawMessage `json:"namespaces"`
		Added      []string          `json:"added"`
		Removed    []string          `json:"removed"`
		Errors     map[string]string `json:"errors"`
	}
	if err := json.Unmarshal(body, &out); err != nil {
		t.Fatalf("refresh: %v: %s", err, body)
	}
	if len(out.Namespaces) != 0 || len(out.Added) != 0 || len(out.Removed) != 0 || len(out.Errors) != 0 {
		t.Fatalf("refresh: unexpected answer for an empty configuration: %s", body)
	}
}

// TestAuthzDecisionsClearOverTheSocket exercises authz_decisions_clear
// through the coverage-recording client: the clear answers with the
// decision report as it stands after clearing — an empty record, in
// the permissive stage. The guard (#1978) flagged the op as never
// exercised at this level (#8466).
func TestAuthzDecisionsClearOverTheSocket(t *testing.T) {
	e := newEnv(t, "{}")
	e.run("ls") // anything that needs a daemon starts one
	client := e.socketClient(t)

	req, err := http.NewRequest(http.MethodDelete, "http://mcpx/v1/authz/decisions", nil)
	if err != nil {
		t.Fatal(err)
	}
	resp, err := client.Do(req)
	if err != nil {
		t.Fatal(err)
	}
	body, _ := io.ReadAll(resp.Body)
	_ = resp.Body.Close()
	if resp.StatusCode != http.StatusOK {
		t.Fatalf("clear: status %d: %s", resp.StatusCode, body)
	}
	var report struct {
		Stage string `json:"stage"`
		Total int    `json:"total"`
	}
	if err := json.Unmarshal(body, &report); err != nil {
		t.Fatalf("clear: %v: %s", err, body)
	}
	if report.Stage != "permissive" || report.Total != 0 {
		t.Fatalf("clear: the answer is not the emptied report: %s", body)
	}
}
