package daemon

import (
	"errors"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

func setupRecipeMutation(t *testing.T) (*Server, string, string) {
	t.Helper()
	s, _ := setupPersist(t, nil, false)
	first := filepath.Join(t.TempDir(), "project", ".config", "mcpx", "scripts")
	second := filepath.Join(t.TempDir(), "user", "scripts")
	if err := os.MkdirAll(first, 0o755); err != nil {
		t.Fatal(err)
	}
	if err := os.MkdirAll(second, 0o755); err != nil {
		t.Fatal(err)
	}
	s.consumer.dirs = []string{first, second}
	return s, first, second
}

func saveRecipeRequest(t *testing.T, s *Server, name, body string) *httptest.ResponseRecorder {
	t.Helper()
	req := httptest.NewRequest(http.MethodPost, "/v1/recipes/"+name, strings.NewReader(body))
	req.SetPathValue("name", name)
	rec := httptest.NewRecorder()
	s.handleRecipeSave(rec, req)
	return rec
}

func removeRecipeRequest(t *testing.T, s *Server, name string) *httptest.ResponseRecorder {
	t.Helper()
	req := httptest.NewRequest(http.MethodDelete, "/v1/recipes/"+name, nil)
	req.SetPathValue("name", name)
	rec := httptest.NewRecorder()
	s.handleRecipeRemove(rec, req)
	return rec
}

func TestRecipeSaveRefusesCrossDirectoryDuplicateAndOverwriteEditsVisibleFile(t *testing.T) {
	s, first, second := setupRecipeMutation(t)
	near := filepath.Join(first, "demo.ts")
	far := filepath.Join(second, "demo.ts")
	if err := os.WriteFile(near, []byte("const near = true;\n"), 0o644); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(far, []byte("const far = true;\n"), 0o644); err != nil {
		t.Fatal(err)
	}
	if rec := saveRecipeRequest(t, s, "demo", `{"source":"const changed = true;"}`); rec.Code != http.StatusConflict {
		t.Fatalf("duplicate status %d, want 409: %s", rec.Code, rec.Body)
	}
	if rec := saveRecipeRequest(t, s, "demo", `{"source":"const changed = true;","overwrite":true}`); rec.Code != http.StatusOK {
		t.Fatalf("overwrite status %d, want 200: %s", rec.Code, rec.Body)
	}
	gotNear, _ := os.ReadFile(near)
	gotFar, _ := os.ReadFile(far)
	if !strings.Contains(string(gotNear), "changed") || strings.Contains(string(gotFar), "changed") {
		t.Fatalf("overwrite did not edit visible definition: near=%q far=%q", gotNear, gotFar)
	}
}

func TestRecipeRemoveDeletesVisibleDefinitionAndNotFoundIs404(t *testing.T) {
	s, first, second := setupRecipeMutation(t)
	near := filepath.Join(first, "demo.ts")
	far := filepath.Join(second, "demo.ts")
	for path, source := range map[string]string{near: "const near = true;", far: "const far = true;"} {
		if err := os.WriteFile(path, []byte(source), 0o644); err != nil {
			t.Fatal(err)
		}
	}
	if rec := removeRecipeRequest(t, s, "demo"); rec.Code != http.StatusOK {
		t.Fatalf("remove status %d, want 200: %s", rec.Code, rec.Body)
	}
	if _, err := os.Stat(near); !os.IsNotExist(err) {
		t.Fatalf("visible definition still exists: %v", err)
	}
	if _, err := os.Stat(far); err != nil {
		t.Fatalf("shadowed definition was removed: %v", err)
	}
	if _, ok := s.findRecipe("demo"); !ok {
		t.Fatal("farther definition did not become visible after removal")
	}
	if rec := removeRecipeRequest(t, s, "missing"); rec.Code != http.StatusNotFound {
		t.Fatalf("missing remove status %d, want 404: %s", rec.Code, rec.Body)
	}
}

func TestRecipeSaveRejectsSymlinkOverwriteTarget(t *testing.T) {
	s, first, _ := setupRecipeMutation(t)
	target := filepath.Join(t.TempDir(), "outside.ts")
	if err := os.WriteFile(target, []byte("const outside = true;\n"), 0o644); err != nil {
		t.Fatal(err)
	}
	link := filepath.Join(first, "demo.ts")
	if err := os.Symlink(target, link); err != nil {
		t.Fatal(err)
	}
	if rec := saveRecipeRequest(t, s, "demo", `{"source":"const changed = true;","overwrite":true}`); rec.Code != http.StatusConflict {
		t.Fatalf("symlink overwrite status %d, want 409: %s", rec.Code, rec.Body)
	}
	got, err := os.ReadFile(target)
	if err != nil {
		t.Fatal(err)
	}
	if strings.Contains(string(got), "changed") {
		t.Fatalf("overwrite followed symlink and changed target: %q", got)
	}
	if _, err := os.Lstat(link); err != nil {
		t.Fatalf("symlink was removed: %v", err)
	}
}

func TestRecipeSaveFindsUnreadableFartherDuplicate(t *testing.T) {
	s, first, second := setupRecipeMutation(t)
	far := filepath.Join(second, "demo.ts")
	// A dangling symlink is a same-name filesystem entry that recipes.Load
	// cannot read or parse; save must still see it and refuse to shadow it.
	if err := os.Symlink(filepath.Join(second, "missing-target.ts"), far); err != nil {
		t.Fatal(err)
	}
	if rec := saveRecipeRequest(t, s, "demo", `{"source":"const new = true;"}`); rec.Code != http.StatusConflict {
		t.Fatalf("duplicate status %d, want 409: %s", rec.Code, rec.Body)
	}
	if _, err := os.Stat(filepath.Join(first, "demo.ts")); !os.IsNotExist(err) {
		t.Fatalf("save shadowed farther entry: %v", err)
	}
}

func TestCreateRecipeFileNoReplacePreservesExistingTarget(t *testing.T) {
	target := filepath.Join(t.TempDir(), "demo.ts")
	original := []byte("const original = true;\n")
	if err := os.WriteFile(target, original, 0o644); err != nil {
		t.Fatal(err)
	}

	err := createRecipeFileNoReplace(target, []byte("const replacement = true;\n"))
	if !errors.Is(err, os.ErrExist) {
		t.Fatalf("createRecipeFileNoReplace error = %v, want os.ErrExist", err)
	}
	got, err := os.ReadFile(target)
	if err != nil {
		t.Fatal(err)
	}
	if string(got) != string(original) {
		t.Fatalf("existing target changed: got %q, want %q", got, original)
	}
}

func TestRecipeRemoveRejectsUnsafeNames(t *testing.T) {
	s, _, _ := setupRecipeMutation(t)
	for _, name := range []string{"", "../secret", ".hidden", `a\\b`} {
		if rec := removeRecipeRequest(t, s, name); rec.Code != http.StatusBadRequest {
			t.Errorf("name %q: status %d, want 400", name, rec.Code)
		}
	}
}
