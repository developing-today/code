package daemon

import (
	"io"
	"log"
	"net/http"
	"net/http/httptest"
	"testing"

	"github.com/developing-today/code/pkgs/mcpx/internal/settings"
)

// #1937 (CODE-16) Codex finding: exec.maxOutputBytes is call-scoped, but
// the daemon's handleExec built its service from cached startup
// settings, so a per-request --exec-max-output-bytes was silently
// ignored for remote executions. handleExec now applies
// s.callSettings(r).Bytes("exec.maxOutputBytes") to the service it runs;
// this pins the resolution that assignment relies on: the header value
// wins for the one request, and without a header the daemon's own
// (default) value stands.
func TestCallSettingsExecMaxOutputBytes(t *testing.T) {
	sch, err := settings.New(settings.Registry())
	if err != nil {
		t.Fatal(err)
	}
	s := &Server{
		set:    settings.NewSet(sch),
		logger: log.New(io.Discard, "", 0),
	}

	req := httptest.NewRequest(http.MethodPost, "/v1/exec", nil)
	req.Header.Set(CallSettingsHeader, `{"exec.maxOutputBytes":"1024"}`)
	if got := s.callSettings(req).Bytes("exec.maxOutputBytes"); got != 1024 {
		t.Errorf("request-scoped exec.maxOutputBytes = %d, want 1024", got)
	}

	bare := httptest.NewRequest(http.MethodPost, "/v1/exec", nil)
	if got := s.callSettings(bare).Bytes("exec.maxOutputBytes"); got != 64*1024 {
		t.Errorf("default exec.maxOutputBytes = %d, want %d (64KiB)", got, 64*1024)
	}
}
