package wantlog

import (
	"os"
	"path/filepath"
	"testing"
	"time"
)

func TestNormalizeCollapsesVariants(t *testing.T) {
	for _, tc := range []struct{ in, want string }{
		{"Calendar", "calendar"},
		{"  calendar  ", "calendar"},
		{"calendar   MCP", "calendar mcp"},
		{"Add\tA\nCalendar MCP", "add a calendar mcp"},
		{"", ""},
		{"   ", ""},
	} {
		if got := Normalize(tc.in); got != tc.want {
			t.Errorf("Normalize(%q) = %q, want %q", tc.in, got, tc.want)
		}
	}
}

func TestAppendListRoundTrip(t *testing.T) {
	st := New(filepath.Join(t.TempDir(), "state", FileName))
	if _, err := st.Append(KindRequest, "Calendar MCP"); err != nil {
		t.Fatal(err)
	}
	if _, err := st.Append(KindSearchMiss, "calendar mcp"); err != nil {
		t.Fatal(err)
	}
	got, err := st.List()
	if err != nil {
		t.Fatal(err)
	}
	if len(got) != 2 {
		t.Fatalf("List = %d entries, want 2", len(got))
	}
	if got[0].Kind != KindRequest || got[0].Query != "calendar mcp" {
		t.Errorf("first entry = %+v, want normalised request for calendar mcp", got[0])
	}
	if got[1].Kind != KindSearchMiss {
		t.Errorf("second entry kind = %q, want %q", got[1].Kind, KindSearchMiss)
	}
}

func TestAppendRejectsEmptyAndUnknownKind(t *testing.T) {
	st := New(filepath.Join(t.TempDir(), FileName))
	if _, err := st.Append(KindRequest, "   "); err == nil {
		t.Error("Append with an empty query should fail")
	}
	if _, err := st.Append("wish", "calendar"); err == nil {
		t.Error("Append with an unknown kind should fail")
	}
	if got, err := st.List(); err != nil || len(got) != 0 {
		t.Errorf("List after rejected appends = %v, %v; want no entries", got, err)
	}
}

func TestListSkipsCorruptLines(t *testing.T) {
	path := filepath.Join(t.TempDir(), FileName)
	st := New(path)
	if _, err := st.Append(KindRequest, "good entry"); err != nil {
		t.Fatal(err)
	}
	f, err := os.OpenFile(path, os.O_WRONLY|os.O_APPEND, 0o600)
	if err != nil {
		t.Fatal(err)
	}
	for _, bad := range []string{"\n", "not json\n", "{\"kind\": \"request\"}\n", "{}\n"} {
		if _, err := f.WriteString(bad); err != nil {
			t.Fatal(err)
		}
	}
	_ = f.Close()
	if _, err := st.Append(KindRequest, "second good entry"); err != nil {
		t.Fatal(err)
	}

	got, err := st.List()
	if err != nil {
		t.Fatalf("List must skip corrupt lines rather than fail: %v", err)
	}
	if len(got) != 2 {
		t.Fatalf("List = %d entries, want the 2 good ones", len(got))
	}
	if got[0].Query != "good entry" || got[1].Query != "second good entry" {
		t.Errorf("List kept %+v, want only the good entries in order", got)
	}
}

func TestListMissingFileIsEmpty(t *testing.T) {
	got, err := New(filepath.Join(t.TempDir(), "nothing-here", FileName)).List()
	if err != nil {
		t.Fatal(err)
	}
	if len(got) != 0 {
		t.Errorf("List on a missing file = %v, want no entries", got)
	}
}

func TestSummarizeCountsAndFirstLastSeen(t *testing.T) {
	st := New(filepath.Join(t.TempDir(), FileName))
	base := time.Date(2026, 11, 1, 12, 0, 0, 0, time.UTC)
	must := func(kind, q string, at time.Time) {
		t.Helper()
		if _, err := st.AppendAt(kind, q, at); err != nil {
			t.Fatal(err)
		}
	}
	must(KindRequest, "Calendar", base)
	must(KindRequest, "calendar ", base.Add(time.Hour))
	must(KindRequest, "  CALENDAR ", base.Add(2*time.Hour))
	must(KindSearchMiss, "calendar", base.Add(3*time.Hour))
	must(KindSearchMiss, "drive", base)

	got := Summarize(mustList(t, st))
	if len(got) != 3 {
		t.Fatalf("Summarize = %d groups, want 3", len(got))
	}
	// Highest count first.
	if got[0].Kind != KindRequest || got[0].Query != "calendar" || got[0].Count != 3 {
		t.Errorf("first group = %+v, want the 3x request/calendar group", got[0])
	}
	if !got[0].FirstSeen.Equal(base) || !got[0].LastSeen.Equal(base.Add(2*time.Hour)) {
		t.Errorf("request/calendar first/last = %v/%v, want %v/%v",
			got[0].FirstSeen, got[0].LastSeen, base, base.Add(2*time.Hour))
	}
	// The two singletons order by most recent: calendar miss, then drive miss.
	if got[1].Query != "calendar" || got[2].Query != "drive" {
		t.Errorf("singleton order = %q, %q; want calendar then drive", got[1].Query, got[2].Query)
	}
}

func TestSummariesFilterAndLimit(t *testing.T) {
	st := New(filepath.Join(t.TempDir(), FileName))
	for _, tc := range [][2]string{
		{KindRequest, "alpha"}, {KindRequest, "beta"}, {KindSearchMiss, "gamma"},
	} {
		if _, err := st.Append(tc[0], tc[1]); err != nil {
			t.Fatal(err)
		}
	}
	req, err := st.Summaries(KindRequest, 0)
	if err != nil {
		t.Fatal(err)
	}
	if len(req) != 2 {
		t.Errorf("Summaries(request) = %d groups, want 2", len(req))
	}
	one, err := st.Summaries("", 1)
	if err != nil {
		t.Fatal(err)
	}
	if len(one) != 1 {
		t.Errorf("Summaries(all, limit 1) = %d groups, want 1", len(one))
	}
	empty, err := New(filepath.Join(t.TempDir(), FileName)).Summaries("", 0)
	if err != nil {
		t.Fatal(err)
	}
	if empty == nil || len(empty) != 0 {
		t.Errorf("Summaries on a missing file = %#v, want an empty non-nil slice", empty)
	}
}

func TestRecordSearchMissOnlyOnEmpty(t *testing.T) {
	dir := t.TempDir()
	if err := RecordSearchMiss(dir, "calendar", 3); err != nil {
		t.Fatal(err)
	}
	if got, err := New(PathFor(dir)).List(); err != nil || len(got) != 0 {
		t.Fatalf("non-empty search recorded %+v, %v; want nothing", got, err)
	}
	if err := RecordSearchMiss(dir, "   ", 0); err != nil {
		t.Fatal(err)
	}
	if got, err := New(PathFor(dir)).List(); err != nil || len(got) != 0 {
		t.Fatalf("blank query recorded %+v, %v; want nothing", got, err)
	}
	if err := RecordSearchMiss(dir, "  Calendar MCP ", 0); err != nil {
		t.Fatal(err)
	}
	got, err := New(PathFor(dir)).List()
	if err != nil {
		t.Fatal(err)
	}
	if len(got) != 1 || got[0].Kind != KindSearchMiss || got[0].Query != "calendar mcp" {
		t.Fatalf("miss recorded %+v, want one normalised search-miss", got)
	}
}

func TestPathForResolvesDefault(t *testing.T) {
	t.Setenv("MCPX_STATE_DIR", filepath.Join(t.TempDir(), "custom-state"))
	if got := PathFor(""); got != filepath.Join(os.Getenv("MCPX_STATE_DIR"), FileName) {
		t.Errorf("PathFor(\"\") = %q, want the MCPX_STATE_DIR file", got)
	}
	if got := PathFor("/s"); got != filepath.Join("/s", FileName) {
		t.Errorf("PathFor(\"/s\") = %q, want /s/%s", got, FileName)
	}
}

func mustList(t *testing.T, st *Store) []Entry {
	t.Helper()
	got, err := st.List()
	if err != nil {
		t.Fatal(err)
	}
	return got
}
