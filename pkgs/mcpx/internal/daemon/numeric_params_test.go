package daemon

import (
	"encoding/json"
	"fmt"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"
)

// invalidNumerics are present-but-unparseable integer spellings (#3485):
// empty, alphabetic, fractional, and scientific notation. All must be a 400
// naming the parameter and the bad value, never a 200 with the default.
var invalidNumerics = []string{"", "banana", "2.7", "1e3"}

// assertInvalidNumeric checks one handler rejects every invalid spelling.
func assertInvalidNumeric(t *testing.T, param string, do func(raw string) *httptest.ResponseRecorder) {
	t.Helper()
	for _, raw := range invalidNumerics {
		rec := do(raw)
		if rec.Code != http.StatusBadRequest {
			t.Errorf("%s=%q: status = %d, want 400 (body %s)", param, raw, rec.Code, rec.Body.String())
			continue
		}
		body := rec.Body.String()
		if !strings.Contains(body, param) {
			t.Errorf("%s=%q: 400 body %q does not name the parameter", param, raw, body)
		}
		if raw != "" && !strings.Contains(body, raw) {
			t.Errorf("%s=%q: 400 body %q does not name the bad value", param, raw, body)
		}
	}
}

// writeJSONLLog seeds dir/logs with one JSONL file carrying the given lines.
func writeJSONLLog(t *testing.T, stateDir string, lines ...string) {
	t.Helper()
	logDir := filepath.Join(stateDir, "logs")
	if err := os.MkdirAll(logDir, 0o755); err != nil {
		t.Fatal(err)
	}
	body := strings.Join(lines, "\n")
	if body != "" {
		body += "\n"
	}
	if err := os.WriteFile(filepath.Join(logDir, "mcpx-2026-09-27.jsonl"), []byte(body), 0o600); err != nil {
		t.Fatal(err)
	}
}

func logLine(ts time.Time, level, msg string, extra string) string {
	obj := map[string]any{
		"ts":    ts.Format(time.RFC3339Nano),
		"level": level,
		"msg":   msg,
	}
	if extra != "" {
		var m map[string]any
		if err := json.Unmarshal([]byte(extra), &m); err != nil {
			panic(err)
		}
		for k, v := range m {
			obj[k] = v
		}
	}
	b, err := json.Marshal(obj)
	if err != nil {
		panic(err)
	}
	return string(b)
}

func decodeCount(t *testing.T, body []byte, key string) int {
	t.Helper()
	var doc map[string]json.RawMessage
	if err := json.Unmarshal(body, &doc); err != nil {
		t.Fatalf("decode body: %v (%s)", err, body)
	}
	raw, ok := doc[key]
	if !ok {
		t.Fatalf("body has no %q key: %s", key, body)
	}
	var arr []json.RawMessage
	if err := json.Unmarshal(raw, &arr); err != nil {
		t.Fatalf("decode %s: %v", key, err)
	}
	return len(arr)
}

var logBase = time.Date(2026, 9, 27, 10, 0, 0, 0, time.UTC)

// TestNumeric_log_limit pins #3485 for GET /v1/log limit.
func TestNumeric_log_limit(t *testing.T) {
	newServer := func(t *testing.T) *Server {
		state := t.TempDir()
		writeJSONLLog(t, state,
			logLine(logBase, "info", "first", ""),
			logLine(logBase.Add(time.Second), "info", "second", ""),
			logLine(logBase.Add(2*time.Second), "info", "third", ""),
		)
		return &Server{set: gapTestSettings(t), paths: Paths{State: state}}
	}
	do := func(s *Server, query string) *httptest.ResponseRecorder {
		req := httptest.NewRequest(http.MethodGet, "/v1/log"+query, nil)
		rec := httptest.NewRecorder()
		s.handleLogQuery(rec, req)
		return rec
	}

	// Invalid spellings are 400s naming the parameter and value.
	s := newServer(t)
	assertInvalidNumeric(t, "limit", func(raw string) *httptest.ResponseRecorder {
		return do(s, "?limit="+raw)
	})

	// Absent retains the configured default (all three seeded rows).
	s = newServer(t)
	absent := do(s, "")
	if absent.Code != http.StatusOK {
		t.Fatalf("absent limit: status = %d, want 200 (%s)", absent.Code, absent.Body.String())
	}
	if n := decodeCount(t, absent.Body.Bytes(), "records"); n != 3 {
		t.Errorf("absent limit: got %d records, want 3 (the default)", n)
	}

	// Valid positive keeps existing behavior.
	s = newServer(t)
	if rec := do(s, "?limit=2"); rec.Code != http.StatusOK {
		t.Errorf("limit=2: status = %d, want 200 (%s)", rec.Code, rec.Body.String())
	} else if n := decodeCount(t, rec.Body.Bytes(), "records"); n != 2 {
		t.Errorf("limit=2: got %d records, want 2", n)
	}

	// Zero and negative resolve to the configured default: same body as absent.
	for _, raw := range []string{"0", "-5"} {
		s = newServer(t)
		rec := do(s, "?limit="+raw)
		if rec.Code != http.StatusOK {
			t.Errorf("limit=%s: status = %d, want 200 (%s)", raw, rec.Code, rec.Body.String())
			continue
		}
		if rec.Body.String() != absent.Body.String() {
			t.Errorf("limit=%s: body differs from absent (default); want identical.\ngot:  %s\nwant: %s", raw, rec.Body.String(), absent.Body.String())
		}
	}
}

// TestNumeric_stats_top pins #3485 for GET /v1/stats top (errors dimension,
// which honors top via capped()).
func TestNumeric_stats_top(t *testing.T) {
	newServer := func(t *testing.T) *Server {
		state := t.TempDir()
		writeJSONLLog(t, state,
			logLine(logBase, "error", "boom-a", `{"template":"boom-a"}`),
			logLine(logBase.Add(time.Second), "error", "boom-b", `{"template":"boom-b"}`),
			logLine(logBase.Add(2*time.Second), "error", "boom-c", `{"template":"boom-c"}`),
		)
		return &Server{set: gapTestSettings(t), paths: Paths{State: state}}
	}
	do := func(s *Server, query string) *httptest.ResponseRecorder {
		req := httptest.NewRequest(http.MethodGet, "/v1/stats"+query, nil)
		rec := httptest.NewRecorder()
		s.handleStats(rec, req)
		return rec
	}
	const dim = "?by=errors"

	s := newServer(t)
	for _, raw := range invalidNumerics {
		rec := do(s, dim+"&top="+raw)
		if rec.Code != http.StatusBadRequest {
			t.Errorf("top=%q: status = %d, want 400 (body %s)", raw, rec.Code, rec.Body.String())
			continue
		}
		if !strings.Contains(rec.Body.String(), "top") {
			t.Errorf("top=%q: 400 body %q does not name the parameter", raw, rec.Body.String())
		}
		if raw != "" && !strings.Contains(rec.Body.String(), raw) {
			t.Errorf("top=%q: 400 body %q does not name the bad value", raw, rec.Body.String())
		}
	}

	s = newServer(t)
	absent := do(s, dim)
	if absent.Code != http.StatusOK {
		t.Fatalf("absent top: status = %d, want 200 (%s)", absent.Code, absent.Body.String())
	}
	if n := decodeCount(t, absent.Body.Bytes(), "rows"); n != 3 {
		t.Errorf("absent top: got %d rows, want 3 (the default)", n)
	}

	s = newServer(t)
	if rec := do(s, dim+"&top=2"); rec.Code != http.StatusOK {
		t.Errorf("top=2: status = %d, want 200 (%s)", rec.Code, rec.Body.String())
	} else if n := decodeCount(t, rec.Body.Bytes(), "rows"); n != 2 {
		t.Errorf("top=2: got %d rows, want 2", n)
	}

	for _, raw := range []string{"0", "-5"} {
		s = newServer(t)
		rec := do(s, dim+"&top="+raw)
		if rec.Code != http.StatusOK {
			t.Errorf("top=%s: status = %d, want 200 (%s)", raw, rec.Code, rec.Body.String())
			continue
		}
		if rec.Body.String() != absent.Body.String() {
			t.Errorf("top=%s: body differs from absent (default); want identical.\ngot:  %s\nwant: %s", raw, rec.Body.String(), absent.Body.String())
		}
	}
}

// fakeRegistry serves a fixed list of servers for the registry-search test.
func fakeRegistry(t *testing.T, names ...string) *httptest.Server {
	t.Helper()
	type entry struct {
		Server map[string]string `json:"server"`
	}
	servers := make([]entry, 0, len(names))
	for _, n := range names {
		servers = append(servers, entry{Server: map[string]string{
			"name": n, "description": "test server " + n, "version": "1.0.0",
		}})
	}
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.Header().Set("Content-Type", "application/json")
		_ = json.NewEncoder(w).Encode(map[string]any{
			"servers":  servers,
			"metadata": map[string]any{"nextCursor": "", "count": len(servers)},
		})
	}))
	t.Cleanup(srv.Close)
	return srv
}

// TestNumeric_registry_limit pins #3485 for GET /v1/registry/search limit.
func TestNumeric_registry_limit(t *testing.T) {
	names := []string{"srv-a", "srv-b", "srv-c", "srv-d", "srv-e"}
	newServer := func(t *testing.T) *Server {
		fake := fakeRegistry(t, names...)
		set := gapTestSettings(t).WithOverrides(map[string]string{"registry.url": fake.URL}, "test")
		return &Server{set: set}
	}
	do := func(s *Server, query string) *httptest.ResponseRecorder {
		req := httptest.NewRequest(http.MethodGet, "/v1/registry/search"+query, nil)
		rec := httptest.NewRecorder()
		s.handleRegistrySearch(rec, req)
		return rec
	}

	s := newServer(t)
	assertInvalidNumeric(t, "limit", func(raw string) *httptest.ResponseRecorder {
		return do(s, "?q=x&limit="+raw)
	})

	s = newServer(t)
	absent := do(s, "?q=x")
	if absent.Code != http.StatusOK {
		t.Fatalf("absent limit: status = %d, want 200 (%s)", absent.Code, absent.Body.String())
	}
	if n := decodeCount(t, absent.Body.Bytes(), "servers"); n != len(names) {
		t.Errorf("absent limit: got %d servers, want %d (the default)", n, len(names))
	}

	s = newServer(t)
	if rec := do(s, "?q=x&limit=2"); rec.Code != http.StatusOK {
		t.Errorf("limit=2: status = %d, want 200 (%s)", rec.Code, rec.Body.String())
	} else if n := decodeCount(t, rec.Body.Bytes(), "servers"); n != 2 {
		t.Errorf("limit=2: got %d servers, want 2", n)
	}

	for _, raw := range []string{"0", "-5"} {
		s = newServer(t)
		rec := do(s, "?q=x&limit="+raw)
		if rec.Code != http.StatusOK {
			t.Errorf("limit=%s: status = %d, want 200 (%s)", raw, rec.Code, rec.Body.String())
			continue
		}
		if rec.Body.String() != absent.Body.String() {
			t.Errorf("limit=%s: body differs from absent (default); want identical.\ngot:  %s\nwant: %s", raw, rec.Body.String(), absent.Body.String())
		}
	}
}

// TestNumeric_recipes_limit pins #3485 for GET /v1/recipes limit (q present).
func TestNumeric_recipes_limit(t *testing.T) {
	newServer := func(t *testing.T) *Server {
		dir := t.TempDir()
		for _, name := range []string{"deploy-alpha", "deploy-beta", "deploy-gamma"} {
			src := fmt.Sprintf("// deploy %s recipe\nconst x = 1;\n", name)
			if err := os.WriteFile(filepath.Join(dir, name+".ts"), []byte(src), 0o600); err != nil {
				t.Fatal(err)
			}
		}
		return &Server{
			set:      gapTestSettings(t),
			consumer: &consumerState{dirs: []string{dir}},
		}
	}
	do := func(s *Server, query string) *httptest.ResponseRecorder {
		req := httptest.NewRequest(http.MethodGet, "/v1/recipes"+query, nil)
		rec := httptest.NewRecorder()
		s.handleRecipesList(rec, req)
		return rec
	}

	s := newServer(t)
	assertInvalidNumeric(t, "limit", func(raw string) *httptest.ResponseRecorder {
		return do(s, "?q=deploy&limit="+raw)
	})

	s = newServer(t)
	absent := do(s, "?q=deploy")
	if absent.Code != http.StatusOK {
		t.Fatalf("absent limit: status = %d, want 200 (%s)", absent.Code, absent.Body.String())
	}
	if n := decodeCount(t, absent.Body.Bytes(), "candidates"); n != 3 {
		t.Errorf("absent limit: got %d candidates, want 3 (the default)", n)
	}

	s = newServer(t)
	if rec := do(s, "?q=deploy&limit=1"); rec.Code != http.StatusOK {
		t.Errorf("limit=1: status = %d, want 200 (%s)", rec.Code, rec.Body.String())
	} else if n := decodeCount(t, rec.Body.Bytes(), "candidates"); n != 1 {
		t.Errorf("limit=1: got %d candidates, want 1", n)
	}

	for _, raw := range []string{"0", "-5"} {
		s = newServer(t)
		rec := do(s, "?q=deploy&limit="+raw)
		if rec.Code != http.StatusOK {
			t.Errorf("limit=%s: status = %d, want 200 (%s)", raw, rec.Code, rec.Body.String())
			continue
		}
		// Paths embed the test's temp dir, so compare candidate names —
		// not raw bodies — to pin that zero/negative resolve to the default.
		var wantDoc, gotDoc struct {
			Candidates []struct {
				Recipe struct {
					Name string `json:"name"`
				} `json:"recipe"`
			} `json:"candidates"`
		}
		if err := json.Unmarshal(absent.Body.Bytes(), &wantDoc); err != nil {
			t.Fatalf("decode absent recipes: %v", err)
		}
		if err := json.Unmarshal(rec.Body.Bytes(), &gotDoc); err != nil {
			t.Fatalf("decode limit=%s recipes: %v", raw, err)
		}
		if len(gotDoc.Candidates) != len(wantDoc.Candidates) {
			t.Errorf("limit=%s: got %d candidates, want %d (the default)", raw, len(gotDoc.Candidates), len(wantDoc.Candidates))
			continue
		}
		for i := range gotDoc.Candidates {
			if gotDoc.Candidates[i].Recipe.Name != wantDoc.Candidates[i].Recipe.Name {
				t.Errorf("limit=%s: candidate %d = %q, want %q (the default)", raw, i, gotDoc.Candidates[i].Recipe.Name, wantDoc.Candidates[i].Recipe.Name)
			}
		}
	}
}

// TestNumeric_search_limit pins #3485 for GET /v1/search limit.
func TestNumeric_search_limit(t *testing.T) {
	tools := []ToolInfo{
		{Namespace: "ns", Tool: "alpha", Function: "ns.alpha", Description: "alpha tool"},
		{Namespace: "ns", Tool: "beta", Function: "ns.beta", Description: "beta tool"},
		{Namespace: "ns", Tool: "gamma", Function: "ns.gamma", Description: "gamma tool"},
	}
	newServer := func(t *testing.T) *Server {
		return &Server{set: gapTestSettings(t), reg: &Registry{testTools: tools}}
	}
	do := func(s *Server, query string) *httptest.ResponseRecorder {
		req := httptest.NewRequest(http.MethodGet, "/v1/search"+query, nil)
		rec := httptest.NewRecorder()
		s.handleSearch(rec, req)
		return rec
	}

	s := newServer(t)
	assertInvalidNumeric(t, "limit", func(raw string) *httptest.ResponseRecorder {
		return do(s, "?q=&limit="+raw)
	})

	s = newServer(t)
	absent := do(s, "?q=")
	if absent.Code != http.StatusOK {
		t.Fatalf("absent limit: status = %d, want 200 (%s)", absent.Code, absent.Body.String())
	}
	var absentList []ToolInfo
	if err := json.Unmarshal(absent.Body.Bytes(), &absentList); err != nil {
		t.Fatalf("decode absent search: %v (%s)", err, absent.Body.String())
	}
	if len(absentList) != 3 {
		t.Errorf("absent limit: got %d tools, want 3 (the default)", len(absentList))
	}

	s = newServer(t)
	if rec := do(s, "?q=&limit=1"); rec.Code != http.StatusOK {
		t.Errorf("limit=1: status = %d, want 200 (%s)", rec.Code, rec.Body.String())
	} else {
		var list []ToolInfo
		if err := json.Unmarshal(rec.Body.Bytes(), &list); err != nil {
			t.Fatalf("decode limit=1 search: %v", err)
		}
		if len(list) != 1 {
			t.Errorf("limit=1: got %d tools, want 1", len(list))
		}
	}

	for _, raw := range []string{"0", "-5"} {
		s = newServer(t)
		rec := do(s, "?q=&limit="+raw)
		if rec.Code != http.StatusOK {
			t.Errorf("limit=%s: status = %d, want 200 (%s)", raw, rec.Code, rec.Body.String())
			continue
		}
		if rec.Body.String() != absent.Body.String() {
			t.Errorf("limit=%s: body differs from absent (default); want identical.\ngot:  %s\nwant: %s", raw, rec.Body.String(), absent.Body.String())
		}
	}
}

// TestNumeric_catalog_budget pins #3485 for GET /v1/catalog budget.
func TestNumeric_catalog_budget(t *testing.T) {
	tools := []ToolInfo{
		{Namespace: "ns", Tool: "alpha", Function: "ns.alpha", Description: "alpha tool with a somewhat long description to spend budget"},
		{Namespace: "ns", Tool: "beta", Function: "ns.beta", Description: "beta tool with a somewhat long description to spend budget"},
		{Namespace: "ns", Tool: "gamma", Function: "ns.gamma", Description: "gamma tool with a somewhat long description to spend budget"},
	}
	newServer := func(t *testing.T) *Server {
		return &Server{set: gapTestSettings(t), reg: &Registry{testTools: tools}}
	}
	do := func(s *Server, query string) *httptest.ResponseRecorder {
		req := httptest.NewRequest(http.MethodGet, "/v1/catalog"+query, nil)
		rec := httptest.NewRecorder()
		s.handleCatalog(rec, req)
		return rec
	}

	s := newServer(t)
	assertInvalidNumeric(t, "budget", func(raw string) *httptest.ResponseRecorder {
		return do(s, "?budget="+raw)
	})

	s = newServer(t)
	absent := do(s, "")
	if absent.Code != http.StatusOK {
		t.Fatalf("absent budget: status = %d, want 200 (%s)", absent.Code, absent.Body.String())
	}
	if absent.Body.String() == "" {
		t.Errorf("absent budget: empty catalog body")
	}

	// Valid positive keeps existing behavior (200 with a catalog).
	s = newServer(t)
	if rec := do(s, "?budget=500"); rec.Code != http.StatusOK {
		t.Errorf("budget=500: status = %d, want 200 (%s)", rec.Code, rec.Body.String())
	} else if rec.Body.String() == "" {
		t.Errorf("budget=500: empty catalog body")
	}

	// Zero and negative resolve to the configured default: same body as absent.
	for _, raw := range []string{"0", "-5"} {
		s = newServer(t)
		rec := do(s, "?budget="+raw)
		if rec.Code != http.StatusOK {
			t.Errorf("budget=%s: status = %d, want 200 (%s)", raw, rec.Code, rec.Body.String())
			continue
		}
		if rec.Body.String() != absent.Body.String() {
			t.Errorf("budget=%s: body differs from absent (default); want identical.\ngot:  %s\nwant: %s", raw, rec.Body.String(), absent.Body.String())
		}
	}
}
