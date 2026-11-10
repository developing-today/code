package daemon

import (
	"fmt"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"strings"
	"sync"
	"testing"
)

// setupAdapterRoutes builds a daemon whose paths.adapters names two
// declaration files; the second already declares demo.
func setupAdapterRoutes(t *testing.T) (*Server, string, string) {
	t.Helper()
	s, _ := setupPersist(t, nil, false)
	dir := t.TempDir()
	first := filepath.Join(dir, "adapters-a.json")
	second := filepath.Join(dir, "adapters-b.json")
	if err := os.WriteFile(first, []byte(`{"adapters":[]}`), 0o644); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(second, []byte(
		`{"adapters":[{"name":"demo","command":"/bin/true","tools":[{"name":"run"}]}]}`), 0o644); err != nil {
		t.Fatal(err)
	}
	joined := first + string(os.PathListSeparator) + second
	if err := s.set.SetRuntime("paths.adapters", joined); err != nil {
		t.Fatal(err)
	}
	return s, first, second
}

func postAdapter(t *testing.T, s *Server, body string) *httptest.ResponseRecorder {
	t.Helper()
	req := httptest.NewRequest(http.MethodPost, "/v1/adapters", strings.NewReader(body))
	rec := httptest.NewRecorder()
	s.handleAdaptersAdd(rec, req)
	return rec
}

func readFile(t *testing.T, path string) string {
	t.Helper()
	b, err := os.ReadFile(path)
	if err != nil {
		t.Fatal(err)
	}
	return string(b)
}

// A name the second file declares must not be written into the first:
// loadAdapterFiles refuses names two files declare, so the duplicate
// would persist and break every later reload and start (Codex review,
// PR #2565).
func TestAdapterAddRefusesANameDeclaredInAnotherFile(t *testing.T) {
	s, first, second := setupAdapterRoutes(t)
	rec := postAdapter(t, s, `{"name":"demo","command":"/bin/false","tools":[{"name":"run"}]}`)
	if rec.Code != http.StatusConflict {
		t.Fatalf("status %d, want 409: %s", rec.Code, rec.Body)
	}
	if got := readFile(t, first); strings.Contains(got, "demo") {
		t.Fatalf("the duplicate was written to the first file: %s", got)
	}
	if got := readFile(t, second); !strings.Contains(got, "/bin/true") {
		t.Fatalf("the existing declaration was touched: %s", got)
	}
}

// A replacement edits the file that defines the adapter, not the first
// file on the list.
func TestAdapterReplaceEditsTheDefiningFile(t *testing.T) {
	s, first, second := setupAdapterRoutes(t)
	rec := postAdapter(t, s, `{"name":"demo","command":"/bin/false","replace":true,"tools":[{"name":"run"}]}`)
	if rec.Code != http.StatusOK {
		t.Fatalf("status %d, want 200: %s", rec.Code, rec.Body)
	}
	if got := readFile(t, second); !strings.Contains(got, "/bin/false") {
		t.Fatalf("the defining file was not updated: %s", got)
	}
	if got := readFile(t, first); strings.Contains(got, "demo") {
		t.Fatalf("a second definition was written to the first file: %s", got)
	}
}

// Concurrent live adds must not lose each other's file edits: the
// handler holds the reload lock over lookup, write and reload (Codex
// review, PR #2565).
func TestConcurrentAdapterAddsAllLand(t *testing.T) {
	s, first, _ := setupAdapterRoutes(t)
	const n = 8
	var wg sync.WaitGroup
	codes := make([]int, n)
	for i := range n {
		wg.Add(1)
		go func() {
			defer wg.Done()
			body := fmt.Sprintf(`{"name":"a%d","command":"/bin/true","tools":[{"name":"run"}]}`, i)
			req := httptest.NewRequest(http.MethodPost, "/v1/adapters", strings.NewReader(body))
			rec := httptest.NewRecorder()
			s.handleAdaptersAdd(rec, req)
			codes[i] = rec.Code
		}()
	}
	wg.Wait()
	for i, code := range codes {
		if code != http.StatusOK {
			t.Fatalf("add a%d: status %d, want 200", i, code)
		}
	}
	got := readFile(t, first)
	for i := range n {
		if !strings.Contains(got, fmt.Sprintf(`"a%d"`, i)) {
			t.Fatalf("add a%d is missing from the file: %s", i, got)
		}
	}
}

// A name a configured server already owns must be refused BEFORE anything
// is persisted: augmentAdapters rejects such an adapter at load, so
// writing it would return 500 on reload while leaving a declaration that
// breaks every later reload and start (Codex review, PR #2565).
func TestAdapterAddRefusesServerNamespaceCollision(t *testing.T) {
	s, _ := setupPersist(t, []byte(`{"mcpServers":{"clash":{"command":"/bin/true"}}}`), false)
	dir := t.TempDir()
	file := filepath.Join(dir, "adapters.json")
	if err := os.WriteFile(file, []byte(`{"adapters":[]}`), 0o644); err != nil {
		t.Fatal(err)
	}
	if err := s.set.SetRuntime("paths.adapters", file); err != nil {
		t.Fatal(err)
	}
	rec := postAdapter(t, s, `{"name":"clash","command":"/bin/false","tools":[{"name":"run"}]}`)
	if rec.Code != http.StatusConflict {
		t.Fatalf("status %d, want 409: %s", rec.Code, rec.Body)
	}
	if got := readFile(t, file); strings.Contains(got, "clash") {
		t.Fatalf("the colliding declaration was persisted: %s", got)
	}
}

// The collision rule is on resolved namespaces: foo_bar the adapter
// and foo-bar the configured server are different map keys but one
// namespace after SanitizeNamespace, and the reload rejects the pair
// (Codex review, PR #2565).
func TestAdapterAddRefusesResolvedNamespaceCollision(t *testing.T) {
	s, _ := setupPersist(t, []byte(`{"mcpServers":{"foo-bar":{"command":"/bin/true"}}}`), false)
	dir := t.TempDir()
	file := filepath.Join(dir, "adapters.json")
	if err := os.WriteFile(file, []byte(`{"adapters":[]}`), 0o644); err != nil {
		t.Fatal(err)
	}
	if err := s.set.SetRuntime("paths.adapters", file); err != nil {
		t.Fatal(err)
	}
	rec := postAdapter(t, s, `{"name":"foo_bar","command":"/bin/false","tools":[{"name":"run"}]}`)
	if rec.Code != http.StatusConflict {
		t.Fatalf("status %d, want 409: %s", rec.Code, rec.Body)
	}
	if got := readFile(t, file); strings.Contains(got, "foo_bar") {
		t.Fatalf("the colliding declaration was persisted: %s", got)
	}
}
