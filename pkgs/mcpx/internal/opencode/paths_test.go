package opencode

import (
	"os"
	"path/filepath"
	"testing"
	"time"
)

// PM-21 (#1942): a v1 opencode on a non-latest channel writes
// opencode-<channel>.db, and stats must find it instead of reporting
// that no database exists.
func TestFindLocatesChannelDatabase(t *testing.T) {
	data := t.TempDir()
	dir := filepath.Join(data, "opencode")
	if err := os.MkdirAll(dir, 0o755); err != nil {
		t.Fatal(err)
	}
	want := filepath.Join(dir, "opencode-next.db")
	if err := os.WriteFile(want, []byte("db"), 0o644); err != nil {
		t.Fatal(err)
	}
	t.Setenv("HOME", t.TempDir())
	t.Setenv("OPENCODE_DB", "")
	t.Setenv("XDG_DATA_HOME", data)
	t.Setenv("OPENCODE_DISABLE_CHANNEL_DB", "")
	got, err := Find("")
	if err != nil {
		t.Fatalf("Find: %v", err)
	}
	if got != want {
		t.Errorf("Find = %q, want the channel database %q", got, want)
	}
}

// The plain database stays first when both exist, and among channel
// files the most recently written wins.
func TestDefaultPathsOrderPlainThenNewestChannel(t *testing.T) {
	data := t.TempDir()
	dir := filepath.Join(data, "opencode")
	if err := os.MkdirAll(dir, 0o755); err != nil {
		t.Fatal(err)
	}
	old := time.Now().Add(-time.Hour)
	for name, mod := range map[string]time.Time{
		"opencode.db":       old,
		"opencode-dev.db":   old,
		"opencode-next.db":  time.Now(),
		"opencode-next.dbx": time.Now(), // not a database file; ignored
		"other.db":          time.Now(), // not opencode's; ignored
	} {
		p := filepath.Join(dir, name)
		if err := os.WriteFile(p, []byte("db"), 0o644); err != nil {
			t.Fatal(err)
		}
		if err := os.Chtimes(p, mod, mod); err != nil {
			t.Fatal(err)
		}
	}
	t.Setenv("HOME", t.TempDir())
	t.Setenv("OPENCODE_DB", "")
	t.Setenv("XDG_DATA_HOME", data)
	t.Setenv("OPENCODE_DISABLE_CHANNEL_DB", "")
	paths := DefaultPaths()
	want := []string{
		filepath.Join(dir, "opencode.db"),
		filepath.Join(dir, "opencode-next.db"),
		filepath.Join(dir, "opencode-dev.db"),
	}
	if len(paths) < len(want) {
		t.Fatalf("DefaultPaths = %v, want prefix %v", paths, want)
	}
	for i, w := range want {
		if paths[i] != w {
			t.Errorf("DefaultPaths[%d] = %q, want %q (all: %v)", i, paths[i], w, paths)
		}
	}
}

// OPENCODE_DISABLE_CHANNEL_DB, which both opencodes honour, leaves only
// the plain filename.
func TestDefaultPathsChannelDisabled(t *testing.T) {
	data := t.TempDir()
	dir := filepath.Join(data, "opencode")
	if err := os.MkdirAll(dir, 0o755); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(dir, "opencode-next.db"), []byte("db"), 0o644); err != nil {
		t.Fatal(err)
	}
	t.Setenv("HOME", t.TempDir())
	t.Setenv("OPENCODE_DB", "")
	t.Setenv("XDG_DATA_HOME", data)
	t.Setenv("OPENCODE_DISABLE_CHANNEL_DB", "1")
	paths := DefaultPaths()
	for _, p := range paths {
		if filepath.Base(p) == "opencode-next.db" {
			t.Errorf("channel database listed despite OPENCODE_DISABLE_CHANNEL_DB: %v", paths)
		}
	}
	if _, err := Find(""); err == nil {
		t.Error("Find succeeded with only a channel database present and channel DBs disabled")
	}
}
