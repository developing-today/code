package daemon

import (
	"path/filepath"
	"testing"
	"time"

	"github.com/developing-today/code/pkgs/mcpx/internal/config"
	"github.com/developing-today/code/pkgs/mcpx/internal/events"
	"github.com/developing-today/code/pkgs/mcpx/internal/mcpclient"
)

// #1604 (Phase 3 of #1601): a modern list_changed notification empties the
// pool catalogue cache. The wiring is InstallHooks' OnListChanged ->
// Pool.Invalidate; before this test nothing covered it, which is what the
// conformance gap caching-notification-invalidates recorded. Invalidate
// forgets the cache timestamp and deadline, so the next Schemas read
// re-lists instead of serving the stale catalogue.
func TestListChangedInvalidatesPoolCache(t *testing.T) {
	dir := t.TempDir()
	cfg := &config.Config{MCPServers: map[string]*config.Server{
		"srv": {Name: "srv", Command: filepath.Join(dir, "never-run")},
	}}
	r, err := NewRegistry(cfg, Paths{State: dir, Cache: dir}, nil)
	if err != nil {
		t.Fatal(err)
	}
	r.InstallHooks(events.New(0), nil, nil)
	p, ok := r.Pool("srv")
	if !ok {
		t.Fatal("pool for srv missing")
	}

	for _, kind := range []string{"tools", "resources", "prompts"} {
		fetched := time.Now()
		deadline := fetched.Add(2 * time.Hour)
		p.SetSchemas(
			[]mcpclient.Tool{{Name: "before"}},
			[]mcpclient.Resource{{URI: "file:///before"}},
			nil, "", fetched, deadline, false,
		)
		if _, _, _, at, dl, _ := p.CachedAll(); !at.Equal(fetched) || !dl.Equal(deadline) {
			t.Fatalf("%s: seeded cache at=%v deadline=%v, want %v / %v", kind, at, dl, fetched, deadline)
		}

		r.hooks.OnListChanged("srv", kind)

		if _, _, _, at, dl, _ := p.CachedAll(); !at.IsZero() || !dl.IsZero() {
			t.Fatalf("%s: list_changed left the catalogue cache fresh (at=%v deadline=%v); the next read would serve the stale catalogue", kind, at, dl)
		}
	}
}
