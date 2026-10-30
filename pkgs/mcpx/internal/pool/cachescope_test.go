package pool_test

import (
	"context"
	"path/filepath"
	"testing"
	"time"

	"github.com/dezren39/mcpx/internal/config"
	"github.com/dezren39/mcpx/internal/mcpclient"
	"github.com/dezren39/mcpx/internal/pool"
)

// #1603: the disk boundary for cacheScope. A seed carries the absolute
// deadline and scope it was saved with; a past deadline starts expired,
// and a failed re-read of an expired cache serves stale instead of
// erroring — while a failed first fetch keeps erroring.

func badPool(t *testing.T) *pool.Pool {
	t.Helper()
	cfg := &config.Config{MCPServers: map[string]*config.Server{
		"bad": {Name: "bad", Command: filepath.Join(t.TempDir(), "never-run")},
	}}
	r, err := cfg.Resolve("bad")
	if err != nil {
		t.Fatal(err)
	}
	p := pool.New(r)
	t.Cleanup(func() { p.Close() })
	return p
}

func TestPoolCacheSeededPastDeadlineRefetches(t *testing.T) {
	p, s := ttlPool(t, "modern", map[string]string{"ERAMCP_TTL_MS": "60000"})
	now := time.Now()
	p.SetSchemas(
		[]mcpclient.Tool{{Name: "seeded-marker"}}, nil, nil, "",
		now, now.Add(-time.Minute), false,
	)
	ctx, cancel := context.WithTimeout(context.Background(), 20*time.Second)
	defer cancel()
	tools, _, err := p.Schemas(ctx)
	if err != nil {
		t.Fatal(err)
	}
	if n := listCalls(t, s); n != 1 {
		t.Fatalf("seeded past its deadline: tools/list called %d times, want 1 (seed starts expired)", n)
	}
	for _, tool := range tools {
		if tool.Name == "seeded-marker" {
			t.Fatal("the expired seed was served as fresh instead of being re-read")
		}
	}
}

func TestPoolCacheSeededPrivateScopeStored(t *testing.T) {
	p, _ := ttlPool(t, "modern", nil)
	now := time.Now()
	p.SetSchemas(nil, nil, nil, "", now, now.Add(time.Hour), true)
	if !p.SchemaPrivate() {
		t.Fatal("a seed's private scope was not carried onto the pool cache")
	}
	_, _, _, _, deadline, private := p.CachedAll()
	if !private || !deadline.Equal(now.Add(time.Hour)) {
		t.Fatalf("CachedAll = deadline %v, private %v; want the seed's deadline and scope", deadline, private)
	}
}

func TestPoolCacheStaleServedOnRefetchError(t *testing.T) {
	p := badPool(t)
	at := time.Now().Add(-2 * time.Hour)
	p.SetSchemas(
		[]mcpclient.Tool{{Name: "stale-tool"}}, nil, nil, "",
		at, time.Now().Add(-time.Hour), false,
	)
	ctx, cancel := context.WithTimeout(context.Background(), 20*time.Second)
	defer cancel()
	for i := 0; i < 2; i++ {
		tools, _, err := p.Schemas(ctx)
		if err != nil {
			t.Fatalf("call %d: expired cache + failing upstream must serve stale, got error %v", i, err)
		}
		if len(tools) != 1 || tools[0].Name != "stale-tool" {
			t.Fatalf("call %d: served %v, want the stale catalogue", i, tools)
		}
	}
	if p.Status().LastError == "" {
		t.Fatal("the failed refresh was not recorded in the pool status")
	}
	_, _, gotAt := p.CachedSchemas()
	if !gotAt.Equal(at) {
		t.Fatalf("cache timestamp moved to %v; a stale serve must leave it at %v so the next read retries", gotAt, at)
	}
}

func TestPoolCacheFirstFetchFailureStillErrors(t *testing.T) {
	p := badPool(t)
	ctx, cancel := context.WithTimeout(context.Background(), 20*time.Second)
	defer cancel()
	if _, _, err := p.Schemas(ctx); err == nil {
		t.Fatal("a failed first fetch must return its error")
	}
	_, _, at := p.CachedSchemas()
	if !at.IsZero() {
		t.Fatalf("a failed first fetch recorded timestamp %v; it must stay unset", at)
	}
}
