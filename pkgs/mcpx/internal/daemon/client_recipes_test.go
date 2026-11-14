package daemon

import (
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

// CODE-36 (#1937; Codex review, PR #9299): the HTTP-generated
// TypeScript client -- the one local mcpx exec/run fetch -- carries
// the daemon's saved scripts in search(), not only daemon-hosted
// execution.
func TestHandleClientIncludesRecipesInSearch(t *testing.T) {
	dir := t.TempDir()
	src := "// Screenshot a page and diff it against a baseline\n" +
		"// @param url:string the page to capture\n" +
		"// @param baseline?:string the baseline image\n" +
		"export default () => 1;\n"
	if err := os.WriteFile(filepath.Join(dir, "screenshot-and-diff.ts"), []byte(src), 0o644); err != nil {
		t.Fatal(err)
	}
	s := &Server{reg: &Registry{}, consumer: &consumerState{dirs: []string{dir}}}
	req := httptest.NewRequest(http.MethodGet, "/v1/client.ts", nil)
	w := httptest.NewRecorder()
	s.handleClient(w, req)
	if w.Code != http.StatusOK {
		t.Fatalf("expected 200, got %d: %s", w.Code, w.Body.String())
	}
	body := w.Body.String()
	for _, want := range []string{
		`"name": "screenshot-and-diff"`,
		`"description": "Screenshot a page and diff it against a baseline"`,
		`"baseline"`,
	} {
		if !strings.Contains(body, want) {
			t.Errorf("client module missing %q", want)
		}
	}
}
