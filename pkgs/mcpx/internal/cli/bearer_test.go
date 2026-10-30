package cli

import (
	"encoding/json"
	"net/http"
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/dezren39/mcpx/internal/daemon"
	"github.com/dezren39/mcpx/internal/settings"
)

// #1975: the CLI attaches the same transport.bearerToken the daemon
// requires, from its own resolved settings; unset, no header is added.

func TestClientAuthorize(t *testing.T) {
	const fixture = "fixture-token-0123456789abcdef01"
	build := func(tok string) *Client {
		sch, err := settings.New(settings.Registry())
		if err != nil {
			t.Fatal(err)
		}
		ov := map[string]string{}
		if tok != "" {
			ov[daemon.BearerTokenSetting] = tok
		}
		return &Client{set: settings.NewSet(sch).WithOverrides(ov, "test")}
	}

	req, err := http.NewRequest(http.MethodGet, "http://mcpx/v1/log", nil)
	if err != nil {
		t.Fatal(err)
	}
	build(fixture).authorize(req)
	if got := req.Header.Get("Authorization"); got != "Bearer "+fixture {
		t.Errorf("Authorization = %q, want the configured bearer token", got)
	}

	req2, err := http.NewRequest(http.MethodGet, "http://mcpx/v1/log", nil)
	if err != nil {
		t.Fatal(err)
	}
	build("").authorize(req2)
	if got := req2.Header.Get("Authorization"); got != "" {
		t.Errorf("Authorization = %q with no token configured, want no header", got)
	}
}

// TestEveryDaemonRequestIsAuthorized pins the Codex P2 on PR #2711:
// authorizing open()/Stream() was not enough — Client.stream (exec.go),
// Client.streamOp (opcmd.go) and the direct requests in feedback.go and
// embeddings.go each built daemon requests that reached Do() without
// authorize(), so they 401'd whenever transport.bearerToken was set.
// Every request-construction site in this package must call authorize()
// before the request is sent; a new unauthenticated path fails here.
func TestEveryDaemonRequestIsAuthorized(t *testing.T) {
	files, err := filepath.Glob("*.go")
	if err != nil {
		t.Fatal(err)
	}
	for _, file := range files {
		if strings.HasSuffix(file, "_test.go") {
			continue
		}
		data, err := os.ReadFile(file)
		if err != nil {
			t.Fatal(err)
		}
		lines := strings.Split(string(data), "\n")
		for i, ln := range lines {
			if !strings.Contains(ln, "http.NewRequestWithContext") {
				continue
			}
			found := false
			for j := i + 1; j < len(lines) && j <= i+20; j++ {
				if strings.Contains(lines[j], ".Do(") {
					break
				}
				if strings.Contains(lines[j], "authorize(") {
					found = true
					break
				}
			}
			if !found {
				t.Errorf("%s:%d: daemon request created without authorize() before it is sent", file, i+1)
			}
		}
	}
}

// TestTargetBearerTokenResolvesTheTargetDaemon pins the Codex P2 on
// PR #2711 (round on 4b73ecbf): stop --all and daemons build clients
// from each daemon record's ConfigPath, but the client presented the
// invoking project's token, so a daemon configured with its own
// file token 401'd the probe and was skipped / reported stopped.
func TestTargetBearerTokenResolvesTheTargetDaemon(t *testing.T) {
	const targetTok = "target-token-0123456789abcdef0123"
	const invokerTok = "invoker-token-0123456789abcdef01"
	write := func(name, tok string) string {
		path := filepath.Join(t.TempDir(), name)
		doc := map[string]any{"transport": map[string]any{"bearerToken": tok}}
		b, err := json.Marshal(doc)
		if err != nil {
			t.Fatal(err)
		}
		if err := os.WriteFile(path, b, 0o600); err != nil {
			t.Fatal(err)
		}
		return path
	}
	target := write("target.json", targetTok)
	invoker := write("invoker.json", invokerTok)

	sch, err := settings.New(settings.Registry())
	if err != nil {
		t.Fatal(err)
	}
	own := settings.NewSet(sch)
	doc, err := readJSONFile(invoker)
	if err != nil {
		t.Fatal(err)
	}
	if err := sch.ApplyFile(own, doc, invoker, 0); err != nil {
		t.Fatal(err)
	}
	if got := daemon.BearerToken(own); got != invokerTok {
		t.Fatalf("setup: invoker set token = %q, want the invoker fixture", got)
	}

	// The invoking project's file token must not be presented to the
	// target daemon; the target's own file token is the credential.
	tok, ok := targetBearerToken(target, own)
	if !ok || tok != targetTok {
		t.Errorf("targetBearerToken = %q, %v; want the target daemon's token", tok, ok)
	}

	// An invocation-scoped (runtime-layer) token still wins: it is
	// the token this process was told to use.
	scoped := settings.NewSet(sch).WithOverrides(
		map[string]string{daemon.BearerTokenSetting: invokerTok}, "test")
	tok, ok = targetBearerToken(target, scoped)
	if !ok || tok != invokerTok {
		t.Errorf("scoped targetBearerToken = %q, %v; want the invocation token", tok, ok)
	}

	// And a client built for the recorded daemon authorizes with the
	// target token end to end.
	c := NewClientAt(daemon.Paths{}, target, "unix:///nonexistent")
	req, err := http.NewRequest(http.MethodGet, "http://mcpx/v1/log", nil)
	if err != nil {
		t.Fatal(err)
	}
	c.authorize(req)
	if got := req.Header.Get("Authorization"); got != "Bearer "+targetTok {
		t.Errorf("record-targeted Authorization = %q, want the target token", got)
	}
}
