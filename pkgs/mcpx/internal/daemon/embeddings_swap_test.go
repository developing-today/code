package daemon

import (
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"reflect"
	"strings"
	"testing"

	"github.com/developing-today/code/pkgs/mcpx/internal/catalog"
	"github.com/developing-today/code/pkgs/mcpx/internal/config"
)

// Oracle: the two committed WASM guests export distinct constant vectors;
// a live update must equal a fresh daemon and must not retain old-space rows.
func TestEmbeddingsHotSwap(t *testing.T) {
	a, err := filepath.Abs("../catalog/testdata/embed_a.wasm")
	if err != nil { t.Fatal(err) }
	b, err := filepath.Abs("../catalog/testdata/embed_b.wasm")
	if err != nil { t.Fatal(err) }
	s, _ := setupPersist(t, []byte(`{"mcpServers":{}}`), false)
	put := func(path, value string) *httptest.ResponseRecorder {
		t.Helper()
		body, _ := json.Marshal(map[string]string{"value": value})
		req := httptest.NewRequest(http.MethodPut, "/v1/settings/"+path, strings.NewReader(string(body)))
		req.SetPathValue("path", path)
		rec := httptest.NewRecorder()
		s.handleSettingsSet(rec, req)
		return rec
	}
	apply := func(path, value string) {
		t.Helper()
		rec := put(path, value)
		if rec.Code != http.StatusOK || !strings.Contains(rec.Body.String(), `"applied":true`) {
			t.Fatalf("%s: %d %s", path, rec.Code, rec.Body)
		}
	}
	vector := func(server *Server) []float32 {
		t.Helper()
		rec := httptest.NewRecorder()
		server.handleEmbeddings(rec, httptest.NewRequest(http.MethodPost, "/v1/embeddings", strings.NewReader(`{"input":"same input"}`)))
		var response embeddingResponse
		if err := json.Unmarshal(rec.Body.Bytes(), &response); err != nil { t.Fatal(err) }
		if len(response.Data) != 1 { t.Fatalf("response: %s", rec.Body) }
		return response.Data[0].Embedding
	}
	apply("embeddings.backend", "wasm")
	apply("embeddings.wasmPath", a)
	before := vector(s)
	if len(before) != catalog.VectorDim || before[0] != 1 || before[1] != 0.25 { t.Fatalf("not fixture A: %v", before[:2]) }
	idx := s.reg.VectorIndex()
	if err := idx.Put("old-space", catalog.Vector(before)); err != nil { t.Fatal(err) }
	apply("embeddings.wasmPath", b)
	after := vector(s)
	if reflect.DeepEqual(before, after) || after[0] != 0.25 || after[1] != 1 { t.Fatalf("not fixture B: %v", after[:2]) }
	if _, ok := idx.Get("old-space"); ok { t.Fatal("old model vector survived swap") }
	freshOpts := handoffOptions(handoffPaths(t), &config.Config{MCPServers: map[string]*config.Server{}}, "fresh")
	freshOpts.Settings = s.set
	fresh, err := NewServer(freshOpts)
	if err != nil { t.Fatal(err) }
	t.Cleanup(fresh.reg.Close)
	if !reflect.DeepEqual(after, vector(fresh)) { t.Fatal("live swap differs from fresh daemon") }
	invalidDir := t.TempDir()
	invalidWasm := filepath.Join(invalidDir, "invalid.wasm")
	if err := os.WriteFile(invalidWasm, []byte("not a wasm module"), 0o640); err != nil { t.Fatal(err) }
	for _, bad := range []string{filepath.Join(t.TempDir(), "missing.wasm"), invalidWasm} {
		rec := put("embeddings.wasmPath", bad)
		if rec.Code == http.StatusOK { t.Fatalf("unreadable model reported applied: %s", rec.Body) }
		if s.set.String("embeddings.wasmPath") != b || !reflect.DeepEqual(after, vector(s)) { t.Fatal("refusal did not preserve settings and active model") }
	}
}
