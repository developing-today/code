package eventstore_test

import (
	"encoding/json"
	"errors"
	"os"
	"path/filepath"
	"strings"
	"sync"
	"testing"
	"time"

	"github.com/developing-today/code/pkgs/mcpx/internal/eventstore"
	"github.com/developing-today/code/pkgs/mcpx/internal/logstore"
)

func openStore(t *testing.T) *eventstore.Store {
	t.Helper()
	st, err := eventstore.Open(":memory:")
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { st.Close() })
	return st
}

func f64(v float64) *float64 { return &v }

const callSchema = `{
  "type": "object",
  "required": ["server", "tool", "ok"],
  "properties": {
    "server": {"type": "string"},
    "tool": {"type": "string"},
    "ok": {"type": "boolean"}
  },
  "additionalProperties": true
}`

func TestWriteRejectsUndeclaredKind(t *testing.T) {
	st := openStore(t)
	_, err := st.Write(eventstore.Event{
		TsUnixMs: 1, Kind: "mcp.call", Source: "mcpx",
		Data: json.RawMessage(`{"server":"fff","tool":"search","ok":true}`),
	})
	if !errors.Is(err, eventstore.ErrUndeclaredKind) {
		t.Fatalf("err = %v, want ErrUndeclaredKind", err)
	}
	var n int
	if err := st.DB().QueryRow(`SELECT count(*) FROM event`).Scan(&n); err != nil {
		t.Fatal(err)
	}
	if n != 0 {
		t.Errorf("store holds %d events after a rejected write, want 0", n)
	}
}

func TestKindSchemaIsEnforced(t *testing.T) {
	st := openStore(t)
	if _, err := st.RegisterKind("mcp.call", "ms", callSchema); err != nil {
		t.Fatal(err)
	}
	// Idempotent re-registration with the same shape.
	if _, err := st.RegisterKind("mcp.call", "ms", callSchema); err != nil {
		t.Fatalf("identical re-registration failed: %v", err)
	}
	// A different shape for the same name is a conflict, not an update.
	if _, err := st.RegisterKind("mcp.call", "ms", `{"type":"object"}`); !errors.Is(err, eventstore.ErrKindConflict) {
		t.Fatalf("conflicting re-registration err = %v, want ErrKindConflict", err)
	}
	// Data missing a required field is rejected with the named error.
	_, err := st.Write(eventstore.Event{
		TsUnixMs: 1, Kind: "mcp.call", Source: "mcpx",
		Data: json.RawMessage(`{"server":"fff"}`),
	})
	if !errors.Is(err, eventstore.ErrInvalidData) {
		t.Fatalf("schema-violating write err = %v, want ErrInvalidData", err)
	}
	// Conforming data is accepted.
	if _, err := st.Write(eventstore.Event{
		TsUnixMs: 2, Kind: "mcp.call", Source: "mcpx", Value: f64(10),
		Data: json.RawMessage(`{"server":"fff","tool":"search","ok":true}`),
	}); err != nil {
		t.Fatalf("conforming write failed: %v", err)
	}
}

func TestInterningDedupesIdentifierSets(t *testing.T) {
	st := openStore(t)
	set := [][]string{{"session", "s1"}, {"parent", "p1"}, {"worktree", "/w"}}
	first, err := st.InternIDs(set)
	if err != nil {
		t.Fatal(err)
	}
	again, err := st.InternIDs(set)
	if err != nil {
		t.Fatal(err)
	}
	reordered, err := st.InternIDs([][]string{{"worktree", "/w"}, {"session", "s1"}, {"parent", "p1"}})
	if err != nil {
		t.Fatal(err)
	}
	if again != first || reordered != first {
		t.Errorf("same set interned to %d, %d, %d — want one id", first, again, reordered)
	}
	// A variadic entry (aliases for one key) is a different set.
	aliased, err := st.InternIDs([][]string{{"session", "s1", "s1-alt"}, {"parent", "p1"}, {"worktree", "/w"}})
	if err != nil {
		t.Fatal(err)
	}
	if aliased == first {
		t.Errorf("aliased set interned to the plain set's id %d", first)
	}
	var sets, members int
	if err := st.DB().QueryRow(`SELECT count(*) FROM idset`).Scan(&sets); err != nil {
		t.Fatal(err)
	}
	if err := st.DB().QueryRow(`SELECT count(*) FROM idset_member WHERE idset_id = ?`, first).Scan(&members); err != nil {
		t.Fatal(err)
	}
	if sets != 2 || members != 3 {
		t.Errorf("idset rows = %d (want 2), members of first set = %d (want 3)", sets, members)
	}
}

func TestWriterInternsTracePairs(t *testing.T) {
	st := openStore(t)
	if _, err := st.RegisterKind("session.start", "", ""); err != nil {
		t.Fatal(err)
	}
	ids := [][]string{{"session", "s1"}, {"parent", "p1"}, {"worktree", "/w"}}
	for i := range 2 {
		if _, err := st.Write(eventstore.Event{
			TsUnixMs: int64(i + 1), Kind: "session.start", Source: "mcpx", IDs: ids,
		}); err != nil {
			t.Fatal(err)
		}
	}
	var distinct int
	if err := st.DB().QueryRow(`SELECT count(DISTINCT idset_id) FROM event`).Scan(&distinct); err != nil {
		t.Fatal(err)
	}
	if distinct != 1 {
		t.Errorf("two writes with identical trace pairs carry %d distinct idsets, want 1", distinct)
	}
}

// line builds one JSONL log record, spelled out in wire form as in
// logstore's own tests.
func line(ts time.Time, level, msg string, kv map[string]any) string {
	obj := map[string]any{"ts": ts.Format(time.RFC3339Nano), "level": level, "msg": msg}
	for k, v := range kv {
		obj[k] = v
	}
	b, err := json.Marshal(obj)
	if err != nil {
		panic(err)
	}
	return string(b)
}

func TestCallStatsAgreeWithLogstore(t *testing.T) {
	base := time.Date(2026, 9, 27, 10, 0, 0, 0, time.UTC)
	type call struct {
		server, tool string
		dur          float64
		ok           bool
	}
	calls := []call{
		{"fff", "search", 10, true},
		{"codedb", "defs", 20, false},
		{"fff", "search", 5, true},
		{"fff", "search", 30, false},
		{"codedb", "defs", 40, true},
		{"codedb", "defs", 15, true},
	}

	// The hand-written dimension, over logstore's records.
	dir := t.TempDir()
	var lines []string
	for i, c := range calls {
		lines = append(lines, line(base.Add(time.Duration(i)*time.Second), "debug", "mcp.call",
			map[string]any{"event": "mcp.call", "session": "s1", "server": c.server,
				"tool": c.tool, "durationMs": c.dur, "ok": c.ok}))
	}
	if err := os.WriteFile(filepath.Join(dir, "mcpx-2026-09-27.jsonl"),
		[]byte(strings.Join(lines, "\n")+"\n"), 0o600); err != nil {
		t.Fatal(err)
	}
	ls, err := logstore.Open(dir)
	if err != nil {
		t.Fatal(err)
	}
	defer ls.Close()
	if _, err := ls.Ingest(); err != nil {
		t.Fatal(err)
	}
	want, err := ls.Calls(logstore.Query{})
	if err != nil {
		t.Fatal(err)
	}

	// The same calls, as events.
	st := openStore(t)
	if _, err := st.RegisterKind("mcp.call", "ms", callSchema); err != nil {
		t.Fatal(err)
	}
	for i, c := range calls {
		data, _ := json.Marshal(map[string]any{"server": c.server, "tool": c.tool, "ok": c.ok})
		if _, err := st.Write(eventstore.Event{
			TsUnixMs: base.Add(time.Duration(i) * time.Second).UnixMilli(),
			Kind:     "mcp.call", Source: "mcpx", Value: f64(c.dur), Data: data,
			IDs: [][]string{{"session", "s1"}},
		}); err != nil {
			t.Fatal(err)
		}
	}
	got, err := st.CallStats()
	if err != nil {
		t.Fatal(err)
	}

	if len(got) != len(want) {
		t.Fatalf("eventstore reported %d rows, logstore %d", len(got), len(want))
	}
	for i := range want {
		g, w := got[i], want[i]
		if g.Server != w.Server || g.Tool != w.Tool || g.Calls != w.Calls ||
			g.Errors != w.Errors || g.ErrRate != w.ErrRate || g.P50 != w.P50 ||
			g.P95 != w.P95 || g.P99 != w.P99 || g.Max != w.Max || g.TotalMs != w.TotalMs {
			t.Errorf("row %d disagrees:\n eventstore %+v\n logstore   %+v", i, g, w)
		}
	}
}

// Regression tests for the three Codex findings on PR #2449.

func TestInterningFingerprintIsUnambiguous(t *testing.T) {
	st := openStore(t)
	// Under a separator-joined fingerprint these two distinct sets
	// encode to identical bytes and would collide onto one idset.
	a, err := st.InternIDs([][]string{{"session", "a\x00b"}})
	if err != nil {
		t.Fatal(err)
	}
	b, err := st.InternIDs([][]string{{"session\x00a", "b"}})
	if err != nil {
		t.Fatal(err)
	}
	if a == b {
		t.Fatalf("distinct identifier sets interned to the same idset %d", a)
	}
	var key, value string
	if err := st.DB().QueryRow(`SELECT key, value FROM idset_member WHERE idset_id = ?`, b).Scan(&key, &value); err != nil {
		t.Fatal(err)
	}
	if key != "session\x00a" || value != "b" {
		t.Errorf("idset %d holds member (%q, %q), want (%q, %q)", b, key, value, "session\x00a", "b")
	}
}

func TestInterningIsAtomicOnMemberFailure(t *testing.T) {
	st := openStore(t)
	if _, err := st.DB().Exec(`CREATE TRIGGER fail_boom BEFORE INSERT ON idset_member WHEN NEW.value = 'boom' BEGIN SELECT RAISE(ABORT, 'boom'); END`); err != nil {
		t.Fatal(err)
	}
	if _, err := st.InternIDs([][]string{{"session", "ok"}, {"session", "boom"}}); err == nil {
		t.Fatal("InternIDs succeeded despite a failing member insert")
	}
	var sets, members int
	if err := st.DB().QueryRow(`SELECT count(*) FROM idset`).Scan(&sets); err != nil {
		t.Fatal(err)
	}
	if err := st.DB().QueryRow(`SELECT count(*) FROM idset_member`).Scan(&members); err != nil {
		t.Fatal(err)
	}
	if sets != 0 || members != 0 {
		t.Fatalf("failed InternIDs left %d idsets and %d members, want 0 and 0", sets, members)
	}
	// The retry after the failure must produce the complete set.
	if _, err := st.DB().Exec(`DROP TRIGGER fail_boom`); err != nil {
		t.Fatal(err)
	}
	id, err := st.InternIDs([][]string{{"session", "ok"}, {"session", "boom"}})
	if err != nil {
		t.Fatal(err)
	}
	if err := st.DB().QueryRow(`SELECT count(*) FROM idset_member WHERE idset_id = ?`, id).Scan(&members); err != nil {
		t.Fatal(err)
	}
	if members != 2 {
		t.Errorf("retried idset holds %d members, want 2", members)
	}
}

func TestRegisterKindConcurrentAcrossStores(t *testing.T) {
	path := filepath.Join(t.TempDir(), "events.db")
	st1, err := eventstore.Open(path)
	if err != nil {
		t.Fatal(err)
	}
	defer st1.Close()
	st2, err := eventstore.Open(path)
	if err != nil {
		t.Fatal(err)
	}
	defer st2.Close()
	stores := []*eventstore.Store{st1, st2}

	// Identical definitions registered concurrently from two Store
	// instances over one database: all succeed with the same id.
	const n = 8
	ids := make([]int64, n)
	errs := make([]error, n)
	var wg sync.WaitGroup
	for i := 0; i < n; i++ {
		wg.Add(1)
		go func(i int) {
			defer wg.Done()
			ids[i], errs[i] = stores[i%2].RegisterKind("mcp.call", "ms", callSchema)
		}(i)
	}
	wg.Wait()
	for i := 0; i < n; i++ {
		if errs[i] != nil {
			t.Fatalf("concurrent identical RegisterKind %d: %v", i, errs[i])
		}
		if ids[i] != ids[0] {
			t.Fatalf("concurrent identical RegisterKind returned ids %d and %d", ids[0], ids[i])
		}
	}

	// Differing definitions raced: exactly one wins, the other gets
	// ErrKindConflict — never a raw UNIQUE constraint error.
	errs = make([]error, 2)
	wg.Add(2)
	go func() { defer wg.Done(); _, errs[0] = st1.RegisterKind("mcp.raced", "ms", callSchema) }()
	go func() { defer wg.Done(); _, errs[1] = st2.RegisterKind("mcp.raced", "count", callSchema) }()
	wg.Wait()
	var ok, conflict int
	for _, err := range errs {
		switch {
		case err == nil:
			ok++
		case errors.Is(err, eventstore.ErrKindConflict):
			conflict++
		default:
			t.Fatalf("raced RegisterKind returned unexpected error: %v", err)
		}
	}
	if ok != 1 || conflict != 1 {
		t.Fatalf("raced RegisterKind: %d ok, %d conflict, want 1 and 1 (errs %v)", ok, conflict, errs)
	}
}
