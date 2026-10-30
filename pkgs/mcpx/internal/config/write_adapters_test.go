package config

import (
	"os"
	"path/filepath"
	"strings"
	"testing"
)

func TestAddAdapterEntryCreatesObjectFile(t *testing.T) {
	path := filepath.Join(t.TempDir(), "adapters.json")
	entry := map[string]any{"name": "jq", "command": "jq",
		"tools": []any{map[string]any{"name": "run"}}}
	if err := AddAdapterEntry(path, entry, false); err != nil {
		t.Fatal(err)
	}
	b, err := os.ReadFile(path)
	if err != nil {
		t.Fatal(err)
	}
	if !strings.Contains(string(b), `"adapters"`) || !strings.Contains(string(b), `"jq"`) {
		t.Fatalf("written file does not hold the declaration:\n%s", b)
	}
	if err := AddAdapterEntry(path, entry, false); err == nil {
		t.Fatal("a duplicate name was accepted without replace")
	}
	entry["command"] = "jaq"
	if err := AddAdapterEntry(path, entry, true); err != nil {
		t.Fatalf("replace refused: %v", err)
	}
	where, found, err := AdapterDefinedIn([]string{path}, "jq")
	if err != nil || !found || where != path {
		t.Fatalf("AdapterDefinedIn = %q, %v, %v", where, found, err)
	}
}

func TestAdapterEntriesKeepBareArrayShape(t *testing.T) {
	path := filepath.Join(t.TempDir(), "adapters.json")
	if err := os.WriteFile(path, []byte(`[{"name":"old","command":"old"}]`), 0o644); err != nil {
		t.Fatal(err)
	}
	if err := AddAdapterEntry(path,
		map[string]any{"name": "new", "command": "new"}, false); err != nil {
		t.Fatal(err)
	}
	b, _ := os.ReadFile(path)
	if !strings.HasPrefix(strings.TrimSpace(string(b)), "[") {
		t.Fatalf("bare-array file was rewritten as an object:\n%s", b)
	}
	removed, err := RemoveAdapterEntry(path, "old")
	if err != nil || !removed {
		t.Fatalf("RemoveAdapterEntry = %v, %v", removed, err)
	}
	removed, err = RemoveAdapterEntry(path, "old")
	if err != nil || removed {
		t.Fatalf("second remove = %v, %v; want false, nil", removed, err)
	}
	b, _ = os.ReadFile(path)
	if strings.Contains(string(b), `"old"`) || !strings.Contains(string(b), `"new"`) {
		t.Fatalf("file after remove is wrong:\n%s", b)
	}
}

// TestAdapterEditsPreserveSiblingSections: a declaration file may be a
// combined document carrying configuration sections beside its adapters.
// Adding or removing an adapter must update only the adapters field, never
// rewrite the whole document and delete the siblings (Codex review,
// PR #2565).
func TestAdapterEditsPreserveSiblingSections(t *testing.T) {
	path := filepath.Join(t.TempDir(), "combined.json")
	doc := `{"mcpServers":{"srv":{"command":"x"}},"paths":{"adapters":["a.json"]},` +
		`"adapters":[{"name":"old","command":"old"}]}`
	if err := os.WriteFile(path, []byte(doc), 0o644); err != nil {
		t.Fatal(err)
	}
	if err := AddAdapterEntry(path, map[string]any{"name": "new", "command": "new"}, false); err != nil {
		t.Fatal(err)
	}
	b, _ := os.ReadFile(path)
	for _, want := range []string{`"mcpServers"`, `"srv"`, `"paths"`, `"old"`, `"new"`} {
		if !strings.Contains(string(b), want) {
			t.Fatalf("add lost %s:\n%s", want, b)
		}
	}
	removed, err := RemoveAdapterEntry(path, "old")
	if err != nil || !removed {
		t.Fatalf("RemoveAdapterEntry = %v, %v", removed, err)
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
