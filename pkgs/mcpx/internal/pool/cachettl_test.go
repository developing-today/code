package pool_test

import (
	"context"
	"sync"
	"testing"
	"time"

	"github.com/developing-today/code/pkgs/mcpx/internal/pool"
)

// #1602: the pool catalogue cache honours a modern upstream's ttlMs.
// eramcp declares ERAMCP_TTL_MS / ERAMCP_CACHE_SCOPE on tools/list, and
// its frame log counts the tools/list calls actually made.

func ttlPool(t *testing.T, mode string, extra map[string]string) (*pool.Pool, *eraServer) {
	t.Helper()
	s := newEraServer(t, mode, extra)
	p := pool.New(s.cfg)
	t.Cleanup(func() { p.Close() })
	return p, s
}

func listCalls(t *testing.T, s *eraServer) int {
	t.Helper()
	n := 0
	for _, f := range s.frames(t) {
		if f == "tools/list" {
			n++
		}
	}
	return n
}

func schemas(t *testing.T, p *pool.Pool) {
	t.Helper()
	ctx, cancel := context.WithTimeout(context.Background(), 20*time.Second)
	defer cancel()
	if _, _, err := p.Schemas(ctx); err != nil {
		t.Fatal(err)
	}
}

func TestPoolCacheTTLZeroStale(t *testing.T) {
	p, s := ttlPool(t, "modern", map[string]string{"ERAMCP_TTL_MS": "0"})
	schemas(t, p)
	schemas(t, p)
	if n := listCalls(t, s); n != 2 {
		t.Fatalf("ttlMs=0: tools/list called %d times, want 2 (immediately stale)", n)
	}
}

func TestPoolCacheTTLNegativeStale(t *testing.T) {
	p, s := ttlPool(t, "modern", map[string]string{"ERAMCP_TTL_MS": "-5"})
	schemas(t, p)
	schemas(t, p)
	if n := listCalls(t, s); n != 2 {
		t.Fatalf("ttlMs=-5: tools/list called %d times, want 2 (immediately stale)", n)
	}
}

func TestPoolCacheTTLAbsentStale(t *testing.T) {
	p, s := ttlPool(t, "modern", nil)
	schemas(t, p)
	schemas(t, p)
	if n := listCalls(t, s); n != 2 {
		t.Fatalf("ttlMs absent: tools/list called %d times, want 2 (immediately stale)", n)
	}
}

func TestPoolCacheTTLPositiveFresh(t *testing.T) {
	p, s := ttlPool(t, "modern", map[string]string{"ERAMCP_TTL_MS": "60000"})
	schemas(t, p)
	_, _, _, fetchedAt, deadline, _ := p.CachedAll()
	if want := fetchedAt.Add(time.Minute); !deadline.Equal(want) {
		t.Fatalf("ttlMs=60000: deadline %v, want ordinary hint deadline %v", deadline, want)
	}
	schemas(t, p)
	schemas(t, p)
	if n := listCalls(t, s); n != 1 {
		t.Fatalf("ttlMs=60000: tools/list called %d times, want 1 (served from cache)", n)
	}
}

func TestPoolCacheTTLPositiveExpires(t *testing.T) {
	p, s := ttlPool(t, "modern", map[string]string{"ERAMCP_TTL_MS": "120"})
	schemas(t, p)
	time.Sleep(250 * time.Millisecond)
	schemas(t, p)
	if n := listCalls(t, s); n != 2 {
		t.Fatalf("ttlMs=120 after 250ms: tools/list called %d times, want 2 (expired)", n)
	}
}

func TestPoolCachePrivateScopeStored(t *testing.T) {
	p, _ := ttlPool(t, "modern", map[string]string{
		"ERAMCP_TTL_MS": "60000", "ERAMCP_CACHE_SCOPE": "private",
	})
	schemas(t, p)
	if !p.SchemaPrivate() {
		t.Fatal("cacheScope=private was not stored on the pool cache")
	}
}

func TestPoolCacheConcurrentExpiredCollapse(t *testing.T) {
	p, s := ttlPool(t, "modern", map[string]string{
		"ERAMCP_TTL_MS": "0", "ERAMCP_LIST_DELAY": "300ms",
	})
	schemas(t, p)
	_ = listCalls(t, s) // clear the first fetch from the log

	const n = 8
	var wg sync.WaitGroup
	errs := make([]error, n)
	for i := 0; i < n; i++ {
		wg.Add(1)
		go func(i int) {
			defer wg.Done()
			ctx, cancel := context.WithTimeout(context.Background(), 20*time.Second)
			defer cancel()
			_, _, errs[i] = p.Schemas(ctx)
		}(i)
	}
	wg.Wait()
	for i, err := range errs {
		if err != nil {
			t.Fatalf("concurrent Schemas %d: %v", i, err)
		}
	}
	if got := listCalls(t, s); got != 1 {
		t.Fatalf("concurrent expired reads: tools/list called %d times, want 1 (collapsed refresh)", got)
	}
}

func TestPoolCacheLegacyNeverExpires(t *testing.T) {
	// A legacy upstream's hints (if it sends any at all) are ignored: the
	// catalogue behaves exactly as it did before #1602.
	p, s := ttlPool(t, "legacy-32601", map[string]string{"ERAMCP_TTL_MS": "0"})
	schemas(t, p)
	schemas(t, p)
	if n := listCalls(t, s); n != 1 {
		t.Fatalf("legacy with ttlMs=0: tools/list called %d times, want 1 (no expiry)", n)
	}
}

func TestPoolCacheTTLLargeHintsAreCapped(t *testing.T) {
	for _, ttl := range []string{
		"86400000",            // exactly 24 hours
		"86400001",            // just over the cap
		"1000000000000",       // ~31 years
		"10000000000000",      // duration multiplication wraps negative
		"18446744073710",      // duration multiplication wraps small-positive
		"9223372036854775807", // maximum int64
	} {
		t.Run(ttl, func(t *testing.T) {
			p, s := ttlPool(t, "modern", map[string]string{"ERAMCP_TTL_MS": ttl})
			schemas(t, p)
			_, _, _, fetchedAt, deadline, _ := p.CachedAll()
			wantDeadline := fetchedAt.Add(24 * time.Hour)
			if !deadline.Equal(wantDeadline) {
				t.Fatalf("ttlMs=%s: deadline %v, want bounded deadline %v", ttl, deadline, wantDeadline)
			}
			if deadline.Before(fetchedAt) {
				t.Fatalf("ttlMs=%s: deadline %v precedes fetch time %v", ttl, deadline, fetchedAt)
			}
			schemas(t, p)
			if n := listCalls(t, s); n != 1 {
				t.Fatalf("ttlMs=%s: tools/list called %d times, want 1 (bounded cache remains fresh)", ttl, n)
			}
		})
	}
}

func TestPoolCacheSeededDeadlineIsCapped(t *testing.T) {
	p, _ := ttlPool(t, "modern", nil)
	fetchedAt := time.Now().Add(-time.Hour)
	p.SetSchemas(nil, nil, nil, "", fetchedAt, fetchedAt.Add(48*time.Hour), false)
	_, _, _, gotAt, deadline, _ := p.CachedAll()
	wantDeadline := fetchedAt.Add(24 * time.Hour)
	if !gotAt.Equal(fetchedAt) || !deadline.Equal(wantDeadline) {
		t.Fatalf("seeded cache at %v has deadline %v; want fetch time %v and capped deadline %v", gotAt, deadline, fetchedAt, wantDeadline)
	}
}
