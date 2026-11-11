//go:build realsoftware

package e2e_test

import (
	"fmt"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"

	"github.com/developing-today/code/pkgs/mcpx/internal/testsupport"
)

// Real mcp-server-fetch (#6903): tools/list through mcpx plus one real
// fetch. The page fetched is served by the test itself on loopback, so
// coverage needs no external network; the server is started with its
// own --ignore-robots-txt flag because a loopback test page has no
// robots.txt to honour, and --allow-private-ips because loopback is a
// private address (the pinned server refuses those by default).
func TestRealFetchServerTools(t *testing.T) {
	server := testsupport.RealServer(t, "mcp-server-fetch")

	page := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.Header().Set("Content-Type", "text/html")
		fmt.Fprint(w, "<html><head><title>realsoftware probe</title></head><body><p>real software, pinned by the flake</p></body></html>")
	}))
	defer page.Close()

	e := newEnv(t, realConfig(server, "fetch", "the real mcp-server-fetch, pinned by the flake", "--ignore-robots-txt", "--allow-private-ips"))
	// The pinned fetch tool's own annotations say it can write
	// (destructiveHint), so mcpx's destructive-confirmation floor stops
	// the call unless the test confirms the way the filesystem test does.
	e.setenv("MCPX_ELICIT_CONFIRM_DESTRUCTIVE=true", "MCPX_ELICIT_ASK_TIMEOUT=60s")
	out := e.run("ls")
	if !strings.Contains(out, "fetch") {
		t.Fatalf("namespace missing from ls:\n%s", out)
	}
	out = e.run("types", "fetch")
	if !strings.Contains(out, "fetch") {
		t.Fatalf("types output missing real tool %q:\n%s", "fetch", out)
	}
	out = realConfirmedCall(t, e, "call", "fetch.fetch", `{"url":`+jsonString(page.URL)+`}`)
	if !strings.Contains(out, "real software, pinned by the flake") {
		t.Fatalf("fetch did not return the test page's content:\n%s", out)
	}
}
