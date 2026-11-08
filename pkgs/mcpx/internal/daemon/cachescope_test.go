package daemon

import (
	"encoding/json"
	"os"
	"path/filepath"
	"testing"
	"time"

	"github.com/developing-today/code/pkgs/mcpx/internal/config"
	"github.com/developing-today/code/pkgs/mcpx/internal/mcpclient"
)

// #1603: cacheScope at the disk boundary. SaveCache never persists a
// private catalogue and writes a public one with its absolute deadline
// and scope; loadCache never seeds a private entry, even from a file
// this daemon did not write, and seeds a public entry's deadline so a
// past-deadline seed starts expired.

func cacheScopeRegistry(t *testing.T, dir string) (*Registry, *config.Config) {
	t.Helper()
	cfg := &config.Config{MCPServers: map[string]*config.Server{
		"pub":  {Name: "pub", Command: filepath.Join(dir, "never-run")},
		"priv": {Name: "priv", Command: filepath.Join(dir, "never-run-either")},
	}}
	r, err := NewRegistry(cfg, Paths{State: dir, Cache: dir}, nil)
	if err != nil {
		t.Fatal(err)
	}
	return r, cfg
}

func TestSaveCacheSkipsPrivatePools(t *testing.T) {
	dir := t.TempDir()
	r, _ := cacheScopeRegistry(t, dir)
	fetched := time.Date(2026, 10, 28, 12, 0, 0, 0, time.UTC)
	deadline := fetched.Add(2 * time.Hour)

	pub, _ := r.Pool("pub")
	pub.SetSchemas([]mcpclient.Tool{{Name: "pub-tool"}}, nil, nil, "", fetched, deadline, false)
	priv, _ := r.Pool("priv")
	priv.SetSchemas([]mcpclient.Tool{{Name: "priv-tool"}}, nil, nil, "", fetched, deadline, true)

	if err := r.SaveCache(); err != nil {
		t.Fatal(err)
	}
	b, err := os.ReadFile(r.paths.SchemaCachePath(r.hash))
	if err != nil {
		t.Fatal(err)
	}
	var cf cacheFile
	if err := json.Unmarshal(b, &cf); err != nil {
		t.Fatal(err)
	}
	if cf.Version != cacheVersion {
		t.Fatalf("cache file version = %s, want %s", cf.Version, cacheVersion)
	}
	if _, ok := cf.Servers["priv"]; ok {
		t.Fatal("a private catalogue was persisted to the schema cache")
	}
	entry, ok := cf.Servers["pub"]
	if !ok {
		t.Fatal("the public catalogue is missing from the schema cache")
	}
	if entry.Scope != "public" {
		t.Fatalf("public entry scope = %q, want %q", entry.Scope, "public")
	}
	if !entry.Deadline.Equal(deadline) {
		t.Fatalf("public entry deadline = %v, want the absolute %v", entry.Deadline, deadline)
	}
	if len(entry.Tools) != 1 || entry.Tools[0].Name != "pub-tool" {
		t.Fatalf("public entry tools = %v, want the seeded catalogue", entry.Tools)
	}
}

func TestLoadCacheNeverSeedsPrivateEntry(t *testing.T) {
	dir := t.TempDir()
	r, cfg := cacheScopeRegistry(t, dir)
	fetched := time.Date(2026, 10, 28, 12, 0, 0, 0, time.UTC)
	past := fetched.Add(-time.Hour)

	// A file this daemon's SaveCache would never write: a private
	// entry for pub, and a public entry for priv whose deadline has
	// already passed.
	cf := cacheFile{
		Version: cacheVersion, ConfigHash: r.hash, SavedAt: fetched,
		Servers: map[string]*cachedEntry{
			"pub": {
				Tools:     []mcpclient.Tool{{Name: "sneaky-tool"}},
				FetchedAt: fetched, Deadline: fetched.Add(2 * time.Hour),
				Scope: "private",
			},
			"priv": {
				Tools:     []mcpclient.Tool{{Name: "seeded-tool"}},
				FetchedAt: fetched, Deadline: past,
				Scope: "public",
			},
		},
	}
	b, err := json.Marshal(cf)
	if err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(r.paths.SchemaCachePath(r.hash), b, 0o600); err != nil {
		t.Fatal(err)
	}

	next, err := NewRegistry(cfg, Paths{State: dir, Cache: dir}, nil)
	if err != nil {
		t.Fatal(err)
	}
	pub, _ := next.Pool("pub")
	if _, _, _, at, _, _ := pub.CachedAll(); !at.IsZero() {
		t.Fatal("a private cache entry was seeded from disk")
	}
	privPool, _ := next.Pool("priv")
	tools, _, _, _, deadline, private := privPool.CachedAll()
	if len(tools) != 1 || tools[0].Name != "seeded-tool" {
		t.Fatalf("public entry seeded tools = %v, want the file's catalogue", tools)
	}
	if private {
		t.Fatal("a public entry was seeded as private")
	}
	if !deadline.Equal(past) {
		t.Fatalf("seeded deadline = %v, want the file's past deadline %v (the seed starts expired)", deadline, past)
	}
}

// #1866: a cache file written before the cached types gained a field must
// be invalidated, not served with the new field silently empty (the
// Annotations incident). The version is now derived from the cached types'
// shape, so an old file — including one with a legacy numeric version —
// never matches and loadCache ignores it wholesale.
func TestLoadCacheIgnoresStaleShapeFiles(t *testing.T) {
	dir := t.TempDir()
	r, cfg := cacheScopeRegistry(t, dir)
	fetched := time.Date(2026, 10, 28, 12, 0, 0, 0, time.UTC)

	// The last hand-bumped era, byte for byte: numeric version, and a
	// tool whose annotations a newer build would read from the entry.
	legacy := `{"version":6,"configHash":"` + r.hash + `","savedAt":"2026-10-28T12:00:00Z",` +
		`"servers":{"pub":{"tools":[{"name":"legacy-tool","annotations":{"destructiveHint":true}}],` +
		`"fetchedAt":"2026-10-28T12:00:00Z","scope":"public"}}}`
	if err := os.WriteFile(r.paths.SchemaCachePath(r.hash), []byte(legacy), 0o600); err != nil {
		t.Fatal(err)
	}
	next, err := NewRegistry(cfg, Paths{State: dir, Cache: dir}, nil)
	if err != nil {
		t.Fatal(err)
	}
	pub, _ := next.Pool("pub")
	if _, _, _, at, _, _ := pub.CachedAll(); !at.IsZero() {
		t.Fatal("a stale-shape (legacy numeric version) cache file was seeded")
	}

	// A string version from any other shape is likewise rejected.
	cf := cacheFile{
		Version: "shape-0000000000000000", ConfigHash: r.hash, SavedAt: fetched,
		Servers: map[string]*cachedEntry{
			"pub": {Tools: []mcpclient.Tool{{Name: "other-shape-tool"}}, FetchedAt: fetched, Scope: "public"},
		},
	}
	b, err := json.Marshal(cf)
	if err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(r.paths.SchemaCachePath(r.hash), b, 0o600); err != nil {
		t.Fatal(err)
	}
	next2, err := NewRegistry(cfg, Paths{State: dir, Cache: dir}, nil)
	if err != nil {
		t.Fatal(err)
	}
	pub2, _ := next2.Pool("pub")
	if _, _, _, at, _, _ := pub2.CachedAll(); !at.IsZero() {
		t.Fatal("a foreign-shape cache file was seeded")
	}
	_ = fetched
}
