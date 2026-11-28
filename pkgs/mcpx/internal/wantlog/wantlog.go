// Package wantlog records what mcpx was asked for and could not find.
//
// An explicit capability request ("add a calendar mcp") and a registry
// search that returned nothing are both data points about missing
// capability, and both were previously lost: the trace feedback store
// rates past interactions, and the empty-search path printed a hint and
// recorded nothing (#1936). This package is that record.
//
// Records are local-only by construction: one append-only JSONL file under
// the mcpx state directory, no network, no transmission. A corrupt or
// unreadable line is skipped rather than failing the list, because a
// feedback record must never be the thing that breaks a query.
package wantlog

import (
	"bufio"
	"encoding/json"
	"fmt"
	"io"
	"os"
	"path/filepath"
	"sort"
	"strings"
	"sync"
	"time"

	"github.com/developing-today/code/pkgs/mcpx/internal/defaults"
)

// Kinds of want records.
const (
	// KindRequest is an explicit capability request recorded via the CLI.
	KindRequest = "request"
	// KindSearchMiss is a registry search that returned no servers,
	// recorded automatically with no flags required.
	KindSearchMiss = "search-miss"
)

// FileName is the append-only log under the state directory.
const FileName = "wantlog.jsonl"

// Entry is one recorded want: an explicit request or an empty-search miss.
type Entry struct {
	Kind  string    `json:"kind"`
	Query string    `json:"query"`
	At    time.Time `json:"at"`
}

// Summary groups entries by kind and normalised query, with the count and
// the first- and last-seen timestamps the acceptance for this record needs.
type Summary struct {
	Kind      string    `json:"kind"`
	Query     string    `json:"query"`
	Count     int       `json:"count"`
	FirstSeen time.Time `json:"firstSeen"`
	LastSeen  time.Time `json:"lastSeen"`
}

// Normalize lowers, trims and collapses whitespace so that "Calendar",
// " calendar " and "calendar   mcp" group to one query. An empty result
// means there is nothing worth recording.
func Normalize(s string) string {
	return strings.Join(strings.Fields(strings.ToLower(s)), " ")
}

// ResolveStateDir finds the mcpx state directory the same way the daemon
// and the supervised client do (MCPX_STATE_DIR, then XDG, then the home
// default), so a caller with no resolved paths still lands in the same
// place. This package cannot import either of them: the daemon imports too
// much to be a leaf, and duplicating ten lines beats a cycle.
func ResolveStateDir() string {
	if v := os.Getenv("MCPX_STATE_DIR"); v != "" {
		return v
	}
	if v := os.Getenv("XDG_STATE_HOME"); v != "" {
		return filepath.Join(v, "mcpx")
	}
	home, _ := os.UserHomeDir()
	return filepath.Join(home, ".local", "state", "mcpx")
}

// PathFor returns the log file for a state directory, resolving the default
// when the caller has none (a zero-value App in tests, for example).
func PathFor(stateDir string) string {
	if stateDir == "" {
		stateDir = ResolveStateDir()
	}
	return filepath.Join(stateDir, FileName)
}

// Store appends to and reads one want log file.
type Store struct {
	mu   sync.Mutex
	path string
}

// New opens the store at path, resolving the default when path is empty.
// The file itself is created on the first append, so constructing a store
// never touches disk.
func New(path string) *Store {
	if path == "" {
		path = PathFor("")
	}
	return &Store{path: path}
}

// Path returns the file this store reads and appends.
func (s *Store) Path() string { return s.path }

// Append records one want with the current time.
func (s *Store) Append(kind, query string) (Entry, error) {
	return s.AppendAt(kind, query, time.Now().UTC())
}

// AppendAt records one want at a fixed time, which is what tests use to
// pin first- and last-seen without sleeping.
func (s *Store) AppendAt(kind, query string, at time.Time) (Entry, error) {
	nq := Normalize(query)
	if nq == "" {
		return Entry{}, fmt.Errorf("wantlog: nothing to record (empty query)")
	}
	if kind != KindRequest && kind != KindSearchMiss {
		return Entry{}, fmt.Errorf("wantlog: unknown kind %q (want %q or %q)", kind, KindRequest, KindSearchMiss)
	}
	if at.IsZero() {
		at = time.Now().UTC()
	}
	e := Entry{Kind: kind, Query: nq, At: at.UTC()}

	s.mu.Lock()
	defer s.mu.Unlock()
	if err := os.MkdirAll(filepath.Dir(s.path), defaults.DirMode); err != nil {
		return Entry{}, err
	}
	f, err := os.OpenFile(s.path, os.O_CREATE|os.O_WRONLY|os.O_APPEND, defaults.PrivateMode)
	if err != nil {
		return Entry{}, err
	}
	defer func() { _ = f.Close() }()
	line, err := json.Marshal(e)
	if err != nil {
		return Entry{}, err
	}
	if _, err := f.Write(append(line, '\n')); err != nil {
		return Entry{}, err
	}
	return e, nil
}

// List reads every record in file order. A missing file is no records, not
// an error, and a corrupt or unreadable line is skipped: a feedback record
// must never be the thing that breaks a query.
func (s *Store) List() ([]Entry, error) {
	s.mu.Lock()
	defer s.mu.Unlock()

	f, err := os.Open(s.path)
	if err != nil {
		if os.IsNotExist(err) {
			return nil, nil
		}
		return nil, err
	}
	defer func() { _ = f.Close() }()

	var out []Entry
	reader := bufio.NewReader(f)
	for {
		line, readErr := reader.ReadString('\n')
		line = strings.TrimSpace(line)
		if line != "" {
			var e Entry
			if err := json.Unmarshal([]byte(line), &e); err == nil {
				e.Query = Normalize(e.Query)
				if (e.Kind == KindRequest || e.Kind == KindSearchMiss) && e.Query != "" && !e.At.IsZero() {
					out = append(out, e)
				}
			}
		}
		if readErr != nil {
			if readErr == io.EOF {
				break
			}
			return nil, readErr
		}
	}
	return out, nil
}

// Summarize groups entries by kind and query. The order is deterministic:
// highest count first, then most recently seen, then kind and query.
func Summarize(entries []Entry) []Summary {
	byKey := map[string]*Summary{}
	for _, e := range entries {
		q := Normalize(e.Query)
		if e.Kind == "" || q == "" {
			continue
		}
		key := e.Kind + "\x00" + q
		s, ok := byKey[key]
		if !ok {
			s = &Summary{Kind: e.Kind, Query: q, FirstSeen: e.At, LastSeen: e.At}
			byKey[key] = s
		}
		s.Count++
		if e.At.Before(s.FirstSeen) {
			s.FirstSeen = e.At
		}
		if e.At.After(s.LastSeen) {
			s.LastSeen = e.At
		}
	}
	out := make([]Summary, 0, len(byKey))
	for _, s := range byKey {
		out = append(out, *s)
	}
	sort.Slice(out, func(i, j int) bool {
		if out[i].Count != out[j].Count {
			return out[i].Count > out[j].Count
		}
		if !out[i].LastSeen.Equal(out[j].LastSeen) {
			return out[i].LastSeen.After(out[j].LastSeen)
		}
		if out[i].Kind != out[j].Kind {
			return out[i].Kind < out[j].Kind
		}
		return out[i].Query < out[j].Query
	})
	return out
}

// Summaries lists the grouped records, optionally filtered to one kind.
// An empty kind lists everything; a limit <= 0 lists all groups. The
// result is never nil, so --json prints [] rather than null.
func (s *Store) Summaries(kind string, limit int) ([]Summary, error) {
	entries, err := s.List()
	if err != nil {
		return nil, err
	}
	if kind != "" {
		kept := entries[:0:0]
		for _, e := range entries {
			if e.Kind == kind {
				kept = append(kept, e)
			}
		}
		entries = kept
	}
	out := Summarize(entries)
	if out == nil {
		out = []Summary{}
	}
	if limit > 0 && len(out) > limit {
		out = out[:limit]
	}
	return out, nil
}

// RecordSearchMiss appends a search-miss for query when resultCount is
// zero, and does nothing otherwise. Callers invoke it unconditionally
// after a search so no new flag is required; it is best-effort by
// contract, and a caller that must not fail ignores the error.
func RecordSearchMiss(stateDir, query string, resultCount int) error {
	if resultCount != 0 {
		return nil
	}
	if Normalize(query) == "" {
		return nil
	}
	_, err := New(PathFor(stateDir)).Append(KindSearchMiss, query)
	return err
}
