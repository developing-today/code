package daemon

import (
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/developing-today/code/pkgs/mcpx/internal/settings"
)

// #2082: every daemon handler that reads a request body goes through
// http.MaxBytesReader at its route class's configured limit. These tests
// pin the over-limit refusal shape — 400 via the handler's existing error
// path, matching the routes that were already bounded — and a source
// guard so a new unbounded reader fails CI instead of shipping.

func bodyLimitSet(t *testing.T) *settings.Set {
	t.Helper()
	sch, err := settings.New(settings.Registry())
	if err != nil {
		t.Fatal(err)
	}
	return settings.NewSet(sch).WithOverrides(map[string]string{
		"http.bodyLimit":        "64B",
		"http.callBodyLimit":    "64B",
		"http.controlBodyLimit": "64B",
	}, "test")
}

func post(t *testing.T, s *Server, path, body string, h http.HandlerFunc) *httptest.ResponseRecorder {
	t.Helper()
	req := httptest.NewRequest(http.MethodPost, path, strings.NewReader(body))
	w := httptest.NewRecorder()
	h(w, req)
	return w
}

func TestBodyLimitsRefuseOverLimit(t *testing.T) {
	s := &Server{set: bodyLimitSet(t)}
	big := strings.Repeat("x", 256)
	cases := []struct {
		name    string
		path    string
		body    string
		handler http.HandlerFunc
	}{
		{"chat completions", "/v1/chat/completions", `{"model":"m","messages":[{"role":"user","content":"` + big + `"}]}`, s.handleChatCompletions},
		{"exec", "/v1/exec", `{"source":"` + big + `"}`, s.handleExec},
		{"intent", "/v1/intent", `{"prompt":"` + big + `"}`, s.handleIntent},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			w := post(t, s, tc.path, tc.body, tc.handler)
			if w.Code != http.StatusBadRequest {
				t.Fatalf("expected 400, got %d: %s", w.Code, w.Body.String())
			}
			if !strings.Contains(w.Body.String(), "request body too large") {
				t.Fatalf("expected MaxBytesReader refusal, got: %s", w.Body.String())
			}
		})
	}
	// handleUpgrade answers over-limit reads with its fixed 400 message.
	w := post(t, s, "/v1/upgrade", `{"binary":"/bin/`+big+`"}`, s.handleUpgrade)
	if w.Code != http.StatusBadRequest {
		t.Fatalf("upgrade: expected 400, got %d: %s", w.Code, w.Body.String())
	}
}

func TestBodyLimitsUnderLimitUnchanged(t *testing.T) {
	s := &Server{set: bodyLimitSet(t)}
	// Under the limit the decode succeeds and the handler reaches its
	// normal validation: an intent with no prompt is refused for the
	// prompt, not for its size.
	w := post(t, s, "/v1/intent", `{}`, s.handleIntent)
	if w.Code != http.StatusBadRequest || !strings.Contains(w.Body.String(), "prompt is required") {
		t.Fatalf("expected prompt-required 400, got %d: %s", w.Code, w.Body.String())
	}
}

// TestNoUnboundedBodyReaders is the #2082 guard: a non-test daemon source
// file may touch r.Body only through a ceiling (MaxBytesReader /
// LimitReader) or a bounded handoff (the artifact store enforces its own
// size and quota caps), or to close it.
func TestNoUnboundedBodyReaders(t *testing.T) {
	entries, err := os.ReadDir(".")
	if err != nil {
		t.Fatal(err)
	}
	allowed := []string{"MaxBytesReader", "LimitReader", "store.Put(r.Body", ".Close()"}
	for _, e := range entries {
		name := e.Name()
		if !strings.HasSuffix(name, ".go") || strings.HasSuffix(name, "_test.go") {
			continue
		}
		data, err := os.ReadFile(filepath.Join(".", name))
		if err != nil {
			t.Fatal(err)
		}
		for i, line := range strings.Split(string(data), "\n") {
			if !strings.Contains(line, "r.Body") {
				continue
			}
			ok := false
			for _, a := range allowed {
				if strings.Contains(line, a) {
					ok = true
					break
				}
			}
			if !ok {
				t.Errorf("%s:%d: r.Body read without a size ceiling: %s", name, i+1, strings.TrimSpace(line))
			}
		}
	}
}
