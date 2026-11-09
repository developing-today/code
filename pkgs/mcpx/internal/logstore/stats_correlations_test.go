package logstore_test

import (
	"testing"
	"time"

	"github.com/developing-today/code/pkgs/mcpx/internal/logstore"
)

// corrFixture is the #1695 probe fixture, extended with a third session on
// a second server: s1 (server fff) calls alpha x3 and beta x2, s2 (server
// fff) calls alpha, beta and gamma once each, s3 (server ggg) calls alpha
// and beta once each. Record-pair counting would report alpha/beta = 8;
// the distinct-session count is 3.
func corrFixture(t *testing.T, dir string) string {
	t.Helper()
	call := func(ts time.Time, server, tool, session string) string {
		return line(ts, "info", "mcp.call", map[string]any{
			"event": "mcp.call", "server": server, "tool": tool,
			"session": session, "ok": true})
	}
	var lines []string
	at := base
	for i := 0; i < 3; i++ {
		lines = append(lines, call(at, "fff", "alpha", "s1"))
		at = at.Add(time.Second)
	}
	for i := 0; i < 2; i++ {
		lines = append(lines, call(at, "fff", "beta", "s1"))
		at = at.Add(time.Second)
	}
	for _, tool := range []string{"alpha", "beta", "gamma"} {
		lines = append(lines, call(base.Add(time.Hour), "fff", tool, "s2"))
	}
	for _, tool := range []string{"alpha", "beta"} {
		lines = append(lines, call(base.Add(2*time.Hour), "ggg", tool, "s3"))
	}
	return writeLog(t, dir, "mcpx-2026-09-27.jsonl", lines...)
}

func corrCounts(t *testing.T, rows []logstore.ToolCorrelation) map[string]int {
	t.Helper()
	out := map[string]int{}
	for _, r := range rows {
		out[r.ToolA+"/"+r.ToolB] = r.Count
	}
	return out
}

func TestCorrelationsCountsDistinctSessionsNotRecordPairs(t *testing.T) {
	dir := t.TempDir()
	corrFixture(t, dir)
	st := open(t, dir)
	ingest(t, st)

	rows, err := st.Correlations(logstore.Query{}, 20)
	if err != nil {
		t.Fatal(err)
	}
	got := corrCounts(t, rows)
	want := map[string]int{"alpha/beta": 3, "alpha/gamma": 1, "beta/gamma": 1}
	if len(got) != len(want) {
		t.Fatalf("correlations = %v, want %v", got, want)
	}
	for pair, n := range want {
		if got[pair] != n {
			t.Errorf("%s count = %d, want %d distinct sessions", pair, got[pair], n)
		}
	}
}

func TestCorrelationsFilteredQueryAppliesToBothSides(t *testing.T) {
	dir := t.TempDir()
	corrFixture(t, dir)
	st := open(t, dir)
	ingest(t, st)

	// Server filter: only the two fff sessions count; before the fix this
	// returned an ambiguous-column SQL error.
	rows, err := st.Correlations(logstore.Query{Server: "fff"}, 20)
	if err != nil {
		t.Fatal(err)
	}
	got := corrCounts(t, rows)
	if got["alpha/beta"] != 2 || got["alpha/gamma"] != 1 || got["beta/gamma"] != 1 {
		t.Errorf("server-filtered correlations = %v, want alpha/beta=2 alpha/gamma=1 beta/gamma=1", got)
	}

	// Time filter: only s2 (at +1h) is inside the window.
	rows, err = st.Correlations(logstore.Query{
		Since: base.Add(30 * time.Minute), Until: base.Add(90 * time.Minute)}, 20)
	if err != nil {
		t.Fatal(err)
	}
	got = corrCounts(t, rows)
	if len(got) != 3 || got["alpha/beta"] != 1 {
		t.Errorf("time-filtered correlations = %v, want three pairs each counting 1", got)
	}

	// Level filter above every fixture record's level: no rows, no error.
	rows, err = st.Correlations(logstore.Query{Level: "error"}, 20)
	if err != nil {
		t.Fatal(err)
	}
	if len(rows) != 0 {
		t.Errorf("level-filtered correlations = %v, want none", rows)
	}
}
