package config

import (
	"os"
	"path/filepath"
	"strings"
	"testing"
)

func TestAddAPIEntryCreatesObjectFile(t *testing.T) {
	path := filepath.Join(t.TempDir(), "apis.json")
	entry := map[string]any{"name": "petstore", "spec": "./petstore.json",
		"include": []any{"pets"}}
	if err := AddAPIEntry(path, entry, false); err != nil {
		t.Fatal(err)
	}
	b, err := os.ReadFile(path)
	if err != nil {
		t.Fatal(err)
	}
	if !strings.Contains(string(b), `"apis"`) || !strings.Contains(string(b), `"petstore"`) {
		t.Fatalf("written file does not hold the declaration:\n%s", b)
	}
	if err := AddAPIEntry(path, entry, false); err == nil {
		t.Fatal("a duplicate name was accepted without replace")
	}
	entry["spec"] = "./petstore-v2.json"
	if err := AddAPIEntry(path, entry, true); err != nil {
		t.Fatalf("replace refused: %v", err)
	}
	where, found, err := APIDefinedIn([]string{path}, "petstore")
	if err != nil || !found || where != path {
		t.Fatalf("APIDefinedIn = %q, %v, %v", where, found, err)
	}
}

func TestAPIEntriesKeepBareArrayShape(t *testing.T) {
	path := filepath.Join(t.TempDir(), "apis.json")
	if err := os.WriteFile(path, []byte(`[{"name":"old","spec":"./old.json"}]`), 0o644); err != nil {
		t.Fatal(err)
	}
	if err := AddAPIEntry(path,
		map[string]any{"name": "new", "spec": "./new.json"}, false); err != nil {
		t.Fatal(err)
	}
	b, _ := os.ReadFile(path)
	if !strings.HasPrefix(strings.TrimSpace(string(b)), "[") {
		t.Fatalf("bare-array file was rewritten as an object:\n%s", b)
	}
	removed, err := RemoveAPIEntry(path, "old")
	if err != nil || !removed {
		t.Fatalf("RemoveAPIEntry = %v, %v", removed, err)
	}
	removed, err = RemoveAPIEntry(path, "old")
	if err != nil || removed {
		t.Fatalf("second remove = %v, %v; want false, nil", removed, err)
	}
	b, _ = os.ReadFile(path)
	if strings.Contains(string(b), `"old"`) || !strings.Contains(string(b), `"new"`) {
		t.Fatalf("file after remove is wrong:\n%s", b)
	}
}

// TestAPIEditsPreserveSiblingSections: a declaration file may be a
// combined document carrying configuration sections beside its APIs.
// Adding or removing an API must update only the apis field, never
// rewrite the whole document and delete the siblings -- the same rule
// the adapter writers follow.
func TestAPIEditsPreserveSiblingSections(t *testing.T) {
	path := filepath.Join(t.TempDir(), "combined.json")
	doc := `{"mcpServers":{"srv":{"command":"x"}},"paths":{"apis":["a.json"]},` +
		`"apis":[{"name":"old","spec":"./old.json"}]}`
	if err := os.WriteFile(path, []byte(doc), 0o644); err != nil {
		t.Fatal(err)
	}
	if err := AddAPIEntry(path, map[string]any{"name": "new", "spec": "./new.json"}, false); err != nil {
		t.Fatal(err)
	}
	b, _ := os.ReadFile(path)
	for _, want := range []string{`"mcpServers"`, `"srv"`, `"paths"`, `"old"`, `"new"`} {
		if !strings.Contains(string(b), want) {
			t.Fatalf("add lost %s:\n%s", want, b)
		}
	}
	removed, err := RemoveAPIEntry(path, "old")
	if err != nil || !removed {
		t.Fatalf("RemoveAPIEntry = %v, %v", removed, err)
	}
	b, _ = os.ReadFile(path)
	for _, want := range []string{`"mcpServers"`, `"srv"`, `"paths"`, `"new"`} {
		if !strings.Contains(string(b), want) {
			t.Fatalf("remove lost %s:\n%s", want, b)
		}
	}
	if strings.Contains(string(b), `"old"`) {
		t.Fatalf("remove left the entry behind:\n%s", b)
	}
}

// TestAPIEditsPreserveExactNumbers: a combined declaration document can
// carry sibling values that do not survive float64 — a tool-view pinned
// ID above 2^53 is the known case (#1933). Editing the apis list must
// not round them (Codex P2, PR #4637; oracle: the exact decimal literal
// in the source document must appear verbatim after add and remove).
func TestAPIEditsPreserveExactNumbers(t *testing.T) {
	path := filepath.Join(t.TempDir(), "combined.json")
	doc := `{"toolViews":{"v":{"pin":9007199254740993}},"apis":[{"name":"old","spec":"./old.json"}]}`
	if err := os.WriteFile(path, []byte(doc), 0o644); err != nil {
		t.Fatal(err)
	}
	if err := AddAPIEntry(path, map[string]any{"name": "new", "spec": "./new.json"}, false); err != nil {
		t.Fatal(err)
	}
	b, _ := os.ReadFile(path)
	if !strings.Contains(string(b), "9007199254740993") {
		t.Fatalf("add rounded the pinned ID:\n%s", b)
	}
	if _, err := RemoveAPIEntry(path, "old"); err != nil {
		t.Fatal(err)
	}
	b, _ = os.ReadFile(path)
	if !strings.Contains(string(b), "9007199254740993") {
		t.Fatalf("remove rounded the pinned ID:\n%s", b)
	}
}
