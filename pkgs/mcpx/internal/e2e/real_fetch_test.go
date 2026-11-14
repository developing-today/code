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
	// The pinned fetch tool's own annotations say it is a read
	// (readOnlyHint=true, destructiveHint=false — verified against the
	// flake-pinned server's tools/list on 2026-11-12), so no
	// confirmation floor stops the call and it is driven directly, the
	// way the time test drives its reads. An earlier pin annotated
	// fetch destructive, and this test confirmed through
	// realConfirmedCall; when the nixpkgs pin moved, the question the
	// helper waits for stopped existing and the test timed out with
	// "no question was raised" (#8929). The destructive-confirmation
	// floor's real-server coverage lives in the filesystem round-trip
	// (write_file is still annotated destructive).
	out := e.run("ls")
	if !strings.Contains(out, "fetch") {
		t.Fatalf("namespace missing from ls:\n%s", out)
	}
	out = e.run("types", "fetch")
	if !strings.Contains(out, "fetch") {
		t.Fatalf("types output missing real tool %q:\n%s", "fetch", out)
	}
	out = e.run("call", "fetch.fetch", `{"url":`+jsonString(page.URL)+`}`)
	if !strings.Contains(out, "real software, pinned by the flake") {
		t.Fatalf("fetch did not return the test page's content:\n%s", out)
	}
}
