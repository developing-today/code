package e2e_test

import (
	"io"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
	"time"
)

func TestProxyUpstreamTimeoutIsReachableFromTheEnvironment(t *testing.T) {
	upstream := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		select {
		case <-r.Context().Done():
			return
		case <-time.After(200 * time.Millisecond):
		}
		w.Header().Set("Content-Type", "application/json")
		_, _ = io.WriteString(w, `{"id":"chatcmpl-proxy-timeout-probe"}`)
	}))
	defer upstream.Close()

	proxyStatus := func(t *testing.T, e *env) int {
		t.Helper()
		e.run("ls") // starts the isolated daemon with its final environment/config
		client := e.socketClient(t)
		// The behaviour under test is the daemon-side proxy timeout
		// (default 10m; 25ms below), not the harness client's own 3s
		// bound: under CPU contention a starved daemon needs longer
		// than that to forward a 200ms upstream response, and the
		// client timeout would mask the status this probe reads.
		client.Timeout = 60 * time.Second
		req, err := http.NewRequest(http.MethodPost, "http://mcpx/v1/chat/completions",
			strings.NewReader(`{"messages":[{"role":"user","content":"hello"}]}`))
		if err != nil {
			t.Fatal(err)
		}
		req.Header.Set("X-Upstream-Url", upstream.URL)
		resp, err := client.Do(req)
		if err != nil {
			t.Fatal(err)
		}
		defer func() { _ = resp.Body.Close() }()
		_, _ = io.Copy(io.Discard, resp.Body)
		return resp.StatusCode
	}

	defaultEnv := newEnv(t, oneServer)
	if got := proxyStatus(t, defaultEnv); got != http.StatusOK {
		t.Fatalf("default timeout status = %d, want 200", got)
	}

	shortEnv := newEnv(t, oneServer).withEnv("MCPX_PROXY_TIMEOUT=25ms")
	if got := proxyStatus(t, shortEnv); got != http.StatusBadGateway {
		t.Fatalf("25ms proxy timeout status = %d, want 502", got)
	}
}
