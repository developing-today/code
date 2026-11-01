package cache_test

import (
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"github.com/developing-today/code/pkgs/mcpx/internal/cache"
)

func writeEntry(t *testing.T, c *cache.Cache, kind, key string, size int, age time.Duration) string {
	t.Helper()
	p, _, err := c.Put(kind, key, cache.Limits{}, func(dir string) error {
		return os.WriteFile(filepath.Join(dir, "payload"), make([]byte, size), 0o600)
	})
	if err != nil {
		t.Fatalf("Put %s/%s: %v", kind, key, err)
	}
	old := time.Now().Add(-age)
	if err := os.Chtimes(p, old, old); err != nil {
		t.Fatal(err)
	}
	return p
}

func TestKeyIsCanonicalAndBoundarySafe(t *testing.T) {
	if cache.KeyString("ab", "c") == cache.KeyString("a", "bc") {
		t.Fatal("part boundaries must change the key")
	}
	if got := cache.KeyString("x"); len(got) != 64 {
		t.Fatalf("key length = %d, want 64 hex chars", len(got))
	}
}

func TestPutGetRoundTripAndHit(t *testing.T) {
	c := cache.Open(t.TempDir())
	fills := 0
	fill := func(dir string) error {
		fills++
		return os.WriteFile(filepath.Join(dir, "f"), []byte("data"), 0o600)
	}
	key := cache.KeyString("inputs")
	if _, hit, err := c.Put("deps", key, cache.Limits{}, fill); err != nil || hit {
		t.Fatalf("first Put: hit=%v err=%v", hit, err)
	}
	p, hit, err := c.Put("deps", key, cache.Limits{}, fill)
	if err != nil || !hit || fills != 1 {
		t.Fatalf("second Put: hit=%v fills=%d err=%v", hit, fills, err)
	}
	if got, ok := c.Get("deps", key); !ok || got != p {
		t.Fatalf("Get = %q,%v want %q,true", got, ok, p)
	}
	if _, ok := c.Get("deps", cache.KeyString("other")); ok {
		t.Fatal("Get of missing key reported a hit")
	}
}

func TestEvictByBytesKeepsNewest(t *testing.T) {
	c := cache.Open(t.TempDir())
	writeEntry(t, c, "deps", cache.KeyString("a"), 100, 3*time.Hour)
	writeEntry(t, c, "deps", cache.KeyString("b"), 100, 2*time.Hour)
	newest := writeEntry(t, c, "deps", cache.KeyString("c"), 100, time.Hour)
	removed := c.Evict("deps", cache.Limits{MaxBytes: 150})
	if len(removed) != 2 {
		t.Fatalf("removed %d entries, want 2", len(removed))
	}
	if _, err := os.Stat(newest); err != nil {
		t.Fatalf("newest entry was evicted: %v", err)
	}
}

func TestEvictByAge(t *testing.T) {
	c := cache.Open(t.TempDir())
	old := writeEntry(t, c, "graphs", cache.KeyString("old"), 10, 48*time.Hour)
	fresh := writeEntry(t, c, "graphs", cache.KeyString("new"), 10, time.Hour)
	c.Evict("graphs", cache.Limits{MaxAge: 24 * time.Hour})
	if _, err := os.Stat(old); !os.IsNotExist(err) {
		t.Fatal("entry older than MaxAge survived")
	}
	if _, err := os.Stat(fresh); err != nil {
		t.Fatal("fresh entry was evicted by age")
	}
}

func TestLeasedEntrySurvivesEviction(t *testing.T) {
	c := cache.Open(t.TempDir())
	leased := writeEntry(t, c, "deps", cache.KeyString("leased"), 100, 3*time.Hour)
	writeEntry(t, c, "deps", cache.KeyString("free"), 100, 2*time.Hour)
	lease, err := cache.Acquire(leased)
	if err != nil {
		t.Fatal(err)
	}
	removed := c.Evict("deps", cache.Limits{MaxBytes: 1})
	for _, r := range removed {
		if r.Path == leased {
			t.Fatal("leased entry was evicted")
		}
	}
	if _, err := os.Stat(leased); err != nil {
		t.Fatalf("leased entry gone: %v", err)
	}
	lease.Release()
	removed = c.Evict("deps", cache.Limits{MaxBytes: 1})
	if len(removed) != 1 || removed[0].Path != leased {
		t.Fatalf("after release, removed = %v, want the formerly leased entry", removed)
	}
}

func TestStaleLeaseDoesNotPin(t *testing.T) {
	c := cache.Open(t.TempDir())
	p := writeEntry(t, c, "deps", cache.KeyString("x"), 10, time.Hour)
	lease, err := cache.Acquire(p)
	if err != nil {
		t.Fatal(err)
	}
	defer lease.Release()
	// Age the lease file itself past the stale bound.
	ents, _ := os.ReadDir(p)
	for _, e := range ents {
		if strings.HasPrefix(e.Name(), ".lease-") {
			old := time.Now().Add(-2 * time.Hour)
			_ = os.Chtimes(filepath.Join(p, e.Name()), old, old)
		}
	}
	if cache.Leased(p, time.Hour) {
		t.Fatal("stale lease still pins")
	}
	if !cache.Leased(p, 0) {
		t.Fatal("with no stale bound, any lease should pin")
	}
}

func TestPruneCountWithPatternAndKeep(t *testing.T) {
	dir := t.TempDir()
	var paths []string
	for i, name := range []string{"script-a.ts", "script-b.ts", "script-c.ts"} {
		p := filepath.Join(dir, name)
		if err := os.WriteFile(p, []byte("x"), 0o600); err != nil {
			t.Fatal(err)
		}
		old := time.Now().Add(-time.Duration(3-i) * time.Hour)
		_ = os.Chtimes(p, old, old)
		paths = append(paths, p)
	}
	other := filepath.Join(dir, "notes.txt")
	_ = os.WriteFile(other, []byte("keep me"), 0o600)
	removed := cache.Prune(dir, cache.Limits{MaxEntries: 2}, paths[2],
		func(name string) bool { return strings.HasPrefix(name, "script-") })
	if len(removed) != 1 || removed[0].Path != paths[0] {
		t.Fatalf("removed = %v, want only the oldest script", removed)
	}
	if _, err := os.Stat(other); err != nil {
		t.Fatal("non-matching file was pruned")
	}
	if _, err := os.Stat(paths[2]); err != nil {
		t.Fatal("keep entry was pruned")
	}
}

func TestRemoveRefusesLeased(t *testing.T) {
	c := cache.Open(t.TempDir())
	key := cache.KeyString("guarded")
	p := writeEntry(t, c, "deps", key, 10, 0)
	lease, err := cache.Acquire(p)
	if err != nil {
		t.Fatal(err)
	}
	defer lease.Release()
	if err := c.Remove("deps", key, 0); err == nil {
		t.Fatal("Remove of a leased entry succeeded")
	}
}

func TestListReportsSizesAndLeaseState(t *testing.T) {
	c := cache.Open(t.TempDir())
	writeEntry(t, c, "deps", cache.KeyString("one"), 42, time.Hour)
	entries, err := c.List("deps", 0)
	if err != nil {
		t.Fatal(err)
	}
	if len(entries) != 1 || entries[0].Size != 42 || entries[0].Leased {
		t.Fatalf("List = %+v", entries)
	}
	entries, err = c.List("missing-kind", 0)
	if err != nil || len(entries) != 0 {
		t.Fatalf("List of missing kind = %v, %v", entries, err)
	}
}

func TestTraversalNamesRejected(t *testing.T) {
	root := t.TempDir()
	c := cache.Open(filepath.Join(root, "cache"))
	outside := filepath.Join(root, "outside")
	if err := os.MkdirAll(outside, 0o700); err != nil {
		t.Fatal(err)
	}
	sentinel := filepath.Join(outside, "keep.txt")
	if err := os.WriteFile(sentinel, []byte("keep"), 0o600); err != nil {
		t.Fatal(err)
	}
	writeEntry(t, c, "deps", cache.KeyString("real"), 10, 0)

	bad := [][2]string{
		{"..", "outside"},      // kind escapes the root
		{"deps", "../outside"}, // key escapes the kind dir
		{"deps", "../../etc"},  // key escapes the root
		{"a/b", "x"},           // separator in kind
		{"deps", "a/b"},        // separator in key
		{"deps", ".."},         // dot component key
		{".", "x"},             // dot component kind
		{"", "x"},              // empty kind
		{"deps", ""},           // empty key
	}
	for _, tc := range bad {
		if err := c.Remove(tc[0], tc[1], 0); err == nil {
			t.Fatalf("Remove(%q, %q) succeeded, want rejection", tc[0], tc[1])
		}
		if got := c.Path(tc[0], tc[1]); got != "" {
			t.Fatalf("Path(%q, %q) = %q, want empty (rejected)", tc[0], tc[1], got)
		}
		if _, _, err := c.Put(tc[0], tc[1], cache.Limits{}, func(string) error { return nil }); err == nil {
			t.Fatalf("Put(%q, %q) succeeded, want rejection", tc[0], tc[1])
		}
		if _, ok := c.Get(tc[0], tc[1]); ok {
			t.Fatalf("Get(%q, %q) reported a hit", tc[0], tc[1])
		}
	}
	if _, err := os.Stat(sentinel); err != nil {
		t.Fatalf("traversal attempt removed a file outside the cache root: %v", err)
	}
	if _, err := c.List("..", 0); err == nil {
		t.Fatal("List of a traversal kind succeeded, want rejection")
	}
	if got := c.Dir(".."); got != "" {
		t.Fatalf("Dir(%q) = %q, want empty (rejected)", "..", got)
	}
	// The legitimate entry is untouched.
	if _, ok := c.Get("deps", cache.KeyString("real")); !ok {
		t.Fatal("legitimate entry lost during traversal attempts")
	}
}

func TestPutNeverEvictsItsOwnEntry(t *testing.T) {
	c := cache.Open(t.TempDir())
	writeEntry(t, c, "deps", cache.KeyString("old"), 10, time.Hour)
	// The acceptance repro: a 2-byte payload with MaxBytes 1. The new
	// entry is larger than the bound by itself; it must be kept (the
	// bound is exceeded by that entry alone) instead of being evicted
	// by its own Put while Put reports success.
	key := cache.KeyString("oversized")
	p, hit, err := c.Put("deps", key, cache.Limits{MaxBytes: 1}, func(dir string) error {
		return os.WriteFile(filepath.Join(dir, "payload"), []byte("ab"), 0o600)
	})
	if err != nil || hit {
		t.Fatalf("Put oversized: hit=%v err=%v", hit, err)
	}
	if _, err := os.Stat(p); err != nil {
		t.Fatalf("Put evicted the entry it just stored: %v", err)
	}
	if got, ok := c.Get("deps", key); !ok || got != p {
		t.Fatalf("Get after oversized Put = %q,%v want %q,true", got, ok, p)
	}
	// The older entry, not the new one, is what eviction removes.
	if _, ok := c.Get("deps", cache.KeyString("old")); ok {
		t.Fatal("older entry survived while the new entry was protected")
	}
}

func TestPutKeepsLeasedOlderEntry(t *testing.T) {
	c := cache.Open(t.TempDir())
	leasedPath := writeEntry(t, c, "deps", cache.KeyString("leased-elder"), 100, time.Hour)
	lease, err := cache.Acquire(leasedPath)
	if err != nil {
		t.Fatal(err)
	}
	defer lease.Release()
	// The #2560 repro shape through Put itself: the older entry (100
	// bytes) holds a live lease, and the second Put (100 bytes) brings
	// the total above MaxBytes 150. The leased elder is not a victim
	// and the just-stored entry is protected by its own Put, so Put
	// must report success with both entries still available.
	key := cache.KeyString("newcomer")
	p, hit, err := c.Put("deps", key, cache.Limits{MaxBytes: 150}, func(dir string) error {
		return os.WriteFile(filepath.Join(dir, "payload"), make([]byte, 100), 0o600)
	})
	if err != nil || hit {
		t.Fatalf("Put newcomer: hit=%v err=%v", hit, err)
	}
	if got, ok := c.Get("deps", cache.KeyString("leased-elder")); !ok || got != leasedPath {
		t.Fatalf("leased older entry lost during Put: Get = %q,%v want %q,true", got, ok, leasedPath)
	}
	if got, ok := c.Get("deps", key); !ok || got != p {
		t.Fatalf("Get after Put = %q,%v want %q,true", got, ok, p)
	}
	if _, err := os.Stat(leasedPath); err != nil {
		t.Fatalf("leased older entry removed by Put eviction: %v", err)
	}
}
