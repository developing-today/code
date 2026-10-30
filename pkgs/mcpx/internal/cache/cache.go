// Package cache is mcpx's one content-addressed cache mechanism (#1908,
// Approach C). Every kind of cached state — prepared dependencies, import
// graphs, flake resolutions, generated exec workdirs — shares this layout,
// keying, lease and eviction code instead of growing its own pruner.
//
// Layout: <root>/<kind>/<key>, where key is the SHA-256 of the entry's
// canonical inputs (see Key). An entry is either a directory or a single
// file. Entries are built in a temporary sibling and renamed into place,
// so a reader never sees a half-written entry.
//
// An entry in use is pinned by a lease: a lease file inside a directory
// entry (or beside a file entry, named for it), created by Acquire and
// removed by Release. Eviction never removes a leased entry. Leases left
// behind by a crashed process stop pinning once they are older than the
// configured stale age.
//
// Eviction is by total bytes and by age against Limits, with an optional
// per-kind entry count for the small kinds whose historical bounds were
// counts (exec workdirs, generated programs, entry points). The writer
// that stores an entry runs eviction for its kind; there is no separate
// evictor process to race (#1908 open question: any writer evicts, and
// the lease is what makes that safe).
package cache

import (
	"crypto/sha256"
	"encoding/hex"
	"errors"
	"fmt"
	"io/fs"
	"os"
	"path/filepath"
	"sort"
	"strings"
	"time"
)

// Limits bounds one eviction pass. A zero field disables that bound.
type Limits struct {
	// MaxBytes bounds the total size of the entries under consideration.
	MaxBytes int64
	// MaxAge bounds entry age, measured from last use.
	MaxAge time.Duration
	// MaxEntries bounds how many entries survive, newest first. It is
	// the right bound only for kinds whose entries are uniformly small.
	MaxEntries int
	// LeaseStaleAge is how old a lease may be before it is treated as
	// abandoned by a crashed process and stops pinning its entry.
	LeaseStaleAge time.Duration
}

// Entry describes one cached entry.
type Entry struct {
	Kind     string
	Key      string
	Path     string
	Size     int64
	LastUsed time.Time
	Leased   bool
}

const leasePrefix = ".lease-"

// Key returns the cache key for canonical inputs: the SHA-256 of the
// parts, each length-prefixed so part boundaries cannot be smuggled.
func Key(parts ...[]byte) string {
	h := sha256.New()
	for _, p := range parts {
		fmt.Fprintf(h, "%d:", len(p))
		h.Write(p)
	}
	return hex.EncodeToString(h.Sum(nil))
}

// KeyString is Key over string parts.
func KeyString(parts ...string) string {
	bs := make([][]byte, len(parts))
	for i, p := range parts {
		bs[i] = []byte(p)
	}
	return Key(bs...)
}

// Cache is a rooted set of per-kind namespaces.
type Cache struct {
	Root string
}

// Open returns the cache rooted at root (<root>/<kind>/<key>).
func Open(root string) *Cache { return &Cache{Root: root} }

// Dir returns the directory holding one kind's entries.
func (c *Cache) Dir(kind string) string { return filepath.Join(c.Root, kind) }

// Path returns the path of one entry.
func (c *Cache) Path(kind, key string) string {
	return filepath.Join(c.Root, kind, key)
}

// Get returns the entry's path if it exists, refreshing its last-used
// time so age eviction measures use, not creation.
func (c *Cache) Get(kind, key string) (string, bool) {
	p := c.Path(kind, key)
	if _, err := os.Stat(p); err != nil {
		return "", false
	}
	now := time.Now()
	_ = os.Chtimes(p, now, now)
	return p, true
}

// Put stores an entry built by fill and returns its path. fill writes
// the entry's contents into a temporary directory; on success the
// directory is renamed into place atomically. If the entry already
// exists, fill is not called and the existing path is returned with
// hit=true. After a store, Evict runs for the kind with lim.
func (c *Cache) Put(kind, key string, lim Limits, fill func(dir string) error) (path string, hit bool, err error) {
	if p, ok := c.Get(kind, key); ok {
		return p, true, nil
	}
	dir := c.Dir(kind)
	if err := os.MkdirAll(dir, 0o700); err != nil {
		return "", false, err
	}
	tmp, err := os.MkdirTemp(dir, ".tmp-*")
	if err != nil {
		return "", false, err
	}
	defer os.RemoveAll(tmp)
	if err := fill(tmp); err != nil {
		return "", false, err
	}
	dest := c.Path(kind, key)
	if err := os.Rename(tmp, dest); err != nil {
		// Another writer stored the same key first; theirs wins.
		if p, ok := c.Get(kind, key); ok {
			return p, true, nil
		}
		return "", false, err
	}
	_ = c.Evict(kind, lim)
	return dest, false, nil
}

// Lease pins an entry against eviction. Release removes the pin.
type Lease struct {
	file string
}

// Acquire leases the entry at path: inside it when it is a directory,
// beside it (named for it) when it is a file. Works for entries inside
// a Cache and for the legacy locations the runner still uses, so any
// code path can pin what it is running from.
func Acquire(path string) (*Lease, error) {
	dir, pattern := leaseLocation(path)
	f, err := os.CreateTemp(dir, pattern+"*")
	if err != nil {
		return nil, err
	}
	name := f.Name()
	_ = f.Close()
	return &Lease{file: name}, nil
}

// Release removes the lease. It is safe to call twice.
func (l *Lease) Release() {
	if l != nil && l.file != "" {
		_ = os.Remove(l.file)
		l.file = ""
	}
}

// leaseLocation says where an entry's leases live and what pattern they
// are created with: inside a directory entry with a bare lease prefix,
// beside a file entry with the entry's name folded in, so one file's
// lease is never mistaken for its neighbour's.
func leaseLocation(path string) (dir, pattern string) {
	if info, err := os.Stat(path); err == nil && info.IsDir() {
		return path, leasePrefix
	}
	return filepath.Dir(path), filepath.Base(path) + leasePrefix
}

// Leased reports whether path currently holds a live lease. A lease
// older than staleAge (when positive) is abandoned and does not pin.
func Leased(path string, staleAge time.Duration) bool {
	dir, pattern := leaseLocation(path)
	ents, err := os.ReadDir(dir)
	if err != nil {
		return false
	}
	for _, e := range ents {
		if !strings.HasPrefix(e.Name(), pattern) {
			continue
		}
		info, ierr := e.Info()
		if ierr != nil {
			continue
		}
		if staleAge > 0 && time.Since(info.ModTime()) > staleAge {
			continue
		}
		return true
	}
	return false
}

// List returns every entry of a kind, oldest use first.
func (c *Cache) List(kind string, staleAge time.Duration) ([]Entry, error) {
	return Scan(c.Dir(kind), kind, staleAge, nil)
}

// Scan reads the entries directly under dir. match, when non-nil, keeps
// only names it accepts — the bridge for locations whose entries follow
// a naming pattern rather than the cache layout (generated programs and
// entry points pruned by the runner). Entries are returned oldest first.
func Scan(dir, kind string, staleAge time.Duration, match func(name string) bool) ([]Entry, error) {
	ents, err := os.ReadDir(dir)
	if err != nil {
		if errors.Is(err, fs.ErrNotExist) {
			return nil, nil
		}
		return nil, err
	}
	var out []Entry
	for _, e := range ents {
		name := e.Name()
		if strings.HasPrefix(name, leasePrefix) || strings.HasPrefix(name, ".tmp-") {
			continue
		}
		if match != nil && !match(name) {
			continue
		}
		p := filepath.Join(dir, name)
		info, ierr := e.Info()
		if ierr != nil {
			continue
		}
		size := info.Size()
		if e.IsDir() {
			size = dirSize(p)
		}
		out = append(out, Entry{
			Kind:     kind,
			Key:      name,
			Path:     p,
			Size:     size,
			LastUsed: info.ModTime(),
			Leased:   Leased(p, staleAge),
		})
	}
	sort.Slice(out, func(i, j int) bool { return out[i].LastUsed.Before(out[j].LastUsed) })
	return out, nil
}

func dirSize(root string) int64 {
	var total int64
	_ = filepath.WalkDir(root, func(_ string, d fs.DirEntry, err error) error {
		if err != nil || d.IsDir() {
			return nil
		}
		if info, ierr := d.Info(); ierr == nil {
			total += info.Size()
		}
		return nil
	})
	return total
}

// Evict enforces lim over one kind, removing oldest unleased entries
// until every bound holds. It returns the entries removed.
func (c *Cache) Evict(kind string, lim Limits) []Entry {
	entries, err := c.List(kind, lim.LeaseStaleAge)
	if err != nil {
		return nil
	}
	return removeAll(SelectVictims(entries, lim, ""))
}

// Prune enforces lim over an arbitrary directory of entries — the form
// the runner's three historical pruners take, now sharing this code.
// keep is never removed. match, when given, selects which names are
// entries at all. It returns the entries removed.
func Prune(dir string, lim Limits, keep string, match func(name string) bool) []Entry {
	entries, err := Scan(dir, "", lim.LeaseStaleAge, match)
	if err != nil {
		return nil
	}
	return removeAll(SelectVictims(entries, lim, keep))
}

func removeAll(victims []Entry) []Entry {
	var removed []Entry
	for _, v := range victims {
		if err := os.RemoveAll(v.Path); err == nil {
			removed = append(removed, v)
		}
	}
	return removed
}

// SelectVictims picks which entries must go for lim to hold, oldest
// first, never selecting keep or a leased entry. It is pure, so tests
// and callers can preview an eviction before performing it.
func SelectVictims(entries []Entry, lim Limits, keep string) []Entry {
	var total int64
	for _, e := range entries {
		total += e.Size
	}
	now := time.Now()
	var victims []Entry
	remaining := len(entries)
	for _, e := range entries { // oldest first
		overBytes := lim.MaxBytes > 0 && total > lim.MaxBytes
		overCount := lim.MaxEntries > 0 && remaining > lim.MaxEntries
		overAge := lim.MaxAge > 0 && now.Sub(e.LastUsed) > lim.MaxAge
		if !overBytes && !overCount && !overAge {
			continue
		}
		if e.Path == keep || e.Leased {
			continue
		}
		victims = append(victims, e)
		total -= e.Size
		remaining--
	}
	return victims
}

// Remove deletes one entry, refusing while it is leased.
func (c *Cache) Remove(kind, key string, staleAge time.Duration) error {
	p := c.Path(kind, key)
	if Leased(p, staleAge) {
		return fmt.Errorf("cache: %s/%s is leased by a running process", kind, key)
	}
	return os.RemoveAll(p)
}
