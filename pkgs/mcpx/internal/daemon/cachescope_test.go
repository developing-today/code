package daemon

import (
	"encoding/json"
	"os"
	"path/filepath"
	"testing"
	"time"

	"github.com/dezren39/mcpx/internal/config"
	"github.com/dezren39/mcpx/internal/mcpclient"
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
		t.Fatalf("cache file version = %d, want %d", cf.Version, cacheVersion)
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
