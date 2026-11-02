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

// setupAPIRoutes builds a daemon whose paths.apis names two declaration
// files; the second already declares demo. Mirrors setupAdapterRoutes:
// the add/remove rules being tested are the same, for the APIs side of
// #2118.
func setupAPIRoutes(t *testing.T) (*Server, string, string) {
	t.Helper()
	s, _ := setupPersist(t, nil, false)
	dir := t.TempDir()
	first := filepath.Join(dir, "apis-a.json")
	second := filepath.Join(dir, "apis-b.json")
	if err := os.WriteFile(first, []byte(`{"apis":[]}`), 0o644); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(second, []byte(
		`{"apis":[{"name":"demo","spec":"./demo.json"}]}`), 0o644); err != nil {
		t.Fatal(err)
	}
	joined := first + string(os.PathListSeparator) + second
	if err := s.set.SetRuntime("paths.apis", joined); err != nil {
		t.Fatal(err)
	}
	return s, first, second
}

func postAPI(t *testing.T, s *Server, body string) *httptest.ResponseRecorder {
	t.Helper()
	req := httptest.NewRequest(http.MethodPost, "/v1/apis", strings.NewReader(body))
	rec := httptest.NewRecorder()
	s.handleAPIsAdd(rec, req)
	return rec
}

func deleteAPI(t *testing.T, s *Server, name string) *httptest.ResponseRecorder {
	t.Helper()
	req := httptest.NewRequest(http.MethodDelete, "/v1/apis/"+name, nil)
	req.SetPathValue("name", name)
	rec := httptest.NewRecorder()
	s.handleAPIsRemove(rec, req)
	return rec
}

// A declaration with no name or no spec is refused before anything is
// written: a reload would not find a nameless entry and nothing could
// call it.
func TestAPIAddRefusesDeclarationWithoutNameOrSpec(t *testing.T) {
	s, first, _ := setupAPIRoutes(t)
	for _, body := range []string{
		`{"spec":"./x.json"}`,
		`{"name":"noname"}`,
		`{"name":"","spec":"./x.json"}`,
	} {
		if rec := postAPI(t, s, body); rec.Code != http.StatusBadRequest {
			t.Fatalf("body %s: status %d, want 400", body, rec.Code)
		}
	}
	if got := readFile(t, first); strings.Contains(got, "x.json") {
		t.Fatalf("the refused declaration was written: %s", got)
	}
}

// With no apis file configured there is nowhere a reload would find a
// new declaration, so the request is refused with the remedy rather
// than written to a file nothing reads.
func TestAPIAddRefusesWhenNoAPIsFileIsConfigured(t *testing.T) {
	s, _ := setupPersist(t, nil, false)
	rec := postAPI(t, s, `{"name":"petstore","spec":"./petstore.json"}`)
	if rec.Code != http.StatusBadRequest {
		t.Fatalf("status %d, want 400: %s", rec.Code, rec.Body)
	}
	if !strings.Contains(rec.Body.String(), "paths.apis") {
		t.Fatalf("refusal names no remedy: %s", rec.Body)
	}
}

// A name the second file declares must not be written into the first:
// the reload re-runs augmentAPIs, which refuses names two files
// declare, so the duplicate would persist and break every later reload
// and start.
func TestAPIAddRefusesANameDeclaredInAnotherFile(t *testing.T) {
	s, first, second := setupAPIRoutes(t)
	rec := postAPI(t, s, `{"name":"demo","spec":"./other.json"}`)
	if rec.Code != http.StatusConflict {
		t.Fatalf("status %d, want 409: %s", rec.Code, rec.Body)
	}
	if got := readFile(t, first); strings.Contains(got, "demo") {
		t.Fatalf("the duplicate was written to the first file: %s", got)
	}
	if got := readFile(t, second); !strings.Contains(got, "demo.json") {
		t.Fatalf("the existing declaration was touched: %s", got)
	}
}

// A replacement edits the file that defines the API, not the first
// file on the list.
func TestAPIReplaceEditsTheDefiningFile(t *testing.T) {
	s, first, second := setupAPIRoutes(t)
	rec := postAPI(t, s, `{"name":"demo","spec":"./replaced.json","replace":true}`)
	if rec.Code != http.StatusOK {
		t.Fatalf("status %d, want 200: %s", rec.Code, rec.Body)
	}
	if got := readFile(t, second); !strings.Contains(got, "replaced.json") {
		t.Fatalf("the defining file was not updated: %s", got)
	}
	if got := readFile(t, first); strings.Contains(got, "demo") {
		t.Fatalf("a second definition was written to the first file: %s", got)
	}
}

// A fresh declaration lands in the first file and survives the reload
// the handler runs: reading the files back shows exactly one entry.
func TestAPIAddWritesTheFirstFileAndReloads(t *testing.T) {
	s, first, _ := setupAPIRoutes(t)
	rec := postAPI(t, s, `{"name":"petstore","spec":"./petstore.json","description":"pets"}`)
	if rec.Code != http.StatusOK {
		t.Fatalf("status %d, want 200: %s", rec.Code, rec.Body)
	}
	got := readFile(t, first)
	for _, want := range []string{`"petstore"`, `petstore.json`, `"pets"`} {
		if !strings.Contains(got, want) {
			t.Fatalf("the declaration is missing %s: %s", want, got)
		}
	}
	if n := strings.Count(got, `"petstore"`); n != 1 {
		t.Fatalf("the declaration appears %d times, want once: %s", n, got)
	}
}

// Removing an API edits the file that defines it.
func TestAPIRemoveDeletesFromTheDefiningFile(t *testing.T) {
	s, first, second := setupAPIRoutes(t)
	rec := deleteAPI(t, s, "demo")
	if rec.Code != http.StatusOK {
		t.Fatalf("status %d, want 200: %s", rec.Code, rec.Body)
	}
	if got := readFile(t, second); strings.Contains(got, "demo") {
		t.Fatalf("the declaration is still there: %s", got)
	}
	if got := readFile(t, first); strings.Contains(got, "demo") {
		t.Fatalf("remove touched the wrong file: %s", got)
	}
}

// Removing a name no file declares is a 404, not a success.
func TestAPIRemoveOfAnUnknownNameIsNotFound(t *testing.T) {
	s, _, _ := setupAPIRoutes(t)
	if rec := deleteAPI(t, s, "nosuchapi"); rec.Code != http.StatusNotFound {
		t.Fatalf("status %d, want 404: %s", rec.Code, rec.Body)
	}
}

// Concurrent live adds must not lose each other's file edits: the
// handler holds the reload lock over lookup, write and reload.
func TestConcurrentAPIAddsAllLand(t *testing.T) {
	s, first, _ := setupAPIRoutes(t)
	const n = 8
	var wg sync.WaitGroup
	codes := make([]int, n)
	for i := range n {
		wg.Add(1)
		go func() {
			defer wg.Done()
			body := fmt.Sprintf(`{"name":"a%d","spec":"./a%d.json"}`, i, i)
			req := httptest.NewRequest(http.MethodPost, "/v1/apis", strings.NewReader(body))
			rec := httptest.NewRecorder()
			s.handleAPIsAdd(rec, req)
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

// A name a configured server already owns is refused BEFORE anything
// is persisted: augmentAPIs rejects such an API at load, so writing it
// would fail the reload while leaving a declaration that breaks every
// later reload and start.
func TestAPIAddRefusesServerNamespaceCollision(t *testing.T) {
	s, _ := setupPersist(t, []byte(`{"mcpServers":{"clash":{"command":"/bin/true"}}}`), false)
	dir := t.TempDir()
	file := filepath.Join(dir, "apis.json")
	if err := os.WriteFile(file, []byte(`{"apis":[]}`), 0o644); err != nil {
		t.Fatal(err)
	}
	if err := s.set.SetRuntime("paths.apis", file); err != nil {
		t.Fatal(err)
	}
	rec := postAPI(t, s, `{"name":"clash","spec":"./clash.json"}`)
	if rec.Code != http.StatusConflict {
		t.Fatalf("status %d, want 409: %s", rec.Code, rec.Body)
	}
	if got := readFile(t, file); strings.Contains(got, "clash") {
		t.Fatalf("the colliding declaration was persisted: %s", got)
	}
}
