package cli

import (
	"net/http"
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
