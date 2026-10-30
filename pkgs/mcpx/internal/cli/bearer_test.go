package cli

import (
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
