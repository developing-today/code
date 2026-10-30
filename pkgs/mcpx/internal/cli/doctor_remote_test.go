package cli

import (
	"strings"
	"testing"

	"github.com/dezren39/mcpx/internal/config"
	"github.com/dezren39/mcpx/internal/logstore"
)

// The slow-remote doctor check (#1936): the four cases the issue names.

func remote(name string) *config.Resolved {
	return &config.Resolved{Server: &config.Server{Name: name, URL: "https://" + name + ".example/mcp"}}
}

func TestSlowRemoteWithLocalPackageRecommendsLocalHosting(t *testing.T) {
	servers := []*config.Resolved{remote("dropbox")}
	stats := map[string]remoteStat{
		"dropbox": {Calls: 200, Errors: 4, ErrRate: 0.02, P95Ms: 1400},
	}
	pkgs := map[string]string{"dropbox": "npm package @example/dropbox-mcp"}

	got := slowRemoteChecks(servers, stats, pkgs)
	if len(got) != 1 {
		t.Fatalf("expected one warning, got %+v", got)
	}
	c := got[0]
	if c.Status != "warn" {
		t.Errorf("status = %q, want warn", c.Status)
	}
	for _, want := range []string{"dropbox", "200 calls", "1400ms"} {
		if !strings.Contains(c.Detail, want) {
			t.Errorf("detail should name %q: %s", want, c.Detail)
		}
	}
	if !strings.Contains(c.Fix, "registry add dropbox") || !strings.Contains(c.Fix, "@example/dropbox-mcp") {
		t.Errorf("fix should recommend the local package: %s", c.Fix)
	}
}

func TestSlowRemoteWithoutLocalPackageReportsOnly(t *testing.T) {
	servers := []*config.Resolved{remote("outlook")}
	stats := map[string]remoteStat{
		"outlook": {Calls: 90, Errors: 9, ErrRate: 0.10, P95Ms: 120},
	}

	got := slowRemoteChecks(servers, stats, map[string]string{})
	if len(got) != 1 {
		t.Fatalf("a high error rate at volume must be reported, got %+v", got)
	}
	if strings.Contains(got[0].Fix, "registry add") {
		t.Errorf("no local package is offered, so none may be recommended: %s", got[0].Fix)
	}
	if !strings.Contains(got[0].Detail, "10.0%") {
		t.Errorf("detail should carry the measured error rate: %s", got[0].Detail)
	}
}

func TestLocalCommandServerIsNeverFlagged(t *testing.T) {
	servers := []*config.Resolved{{Server: &config.Server{Name: "files", Command: "mcp-files"}}}
	stats := map[string]remoteStat{
		"files": {Calls: 5000, Errors: 500, ErrRate: 0.10, P95Ms: 9000},
	}
	if got := slowRemoteChecks(servers, stats, map[string]string{"files": "npm package x"}); len(got) != 0 {
		t.Fatalf("a local server has no round trip to remove and must never be flagged: %+v", got)
	}
}

func TestLowVolumeRemoteIsSilent(t *testing.T) {
	servers := []*config.Resolved{remote("drive")}
	stats := map[string]remoteStat{
		"drive": {Calls: slowRemoteFloorCalls - 1, Errors: 10, ErrRate: 0.25, P95Ms: 8000},
	}
	if got := slowRemoteChecks(servers, stats, nil); len(got) != 0 {
		t.Fatalf("below the volume floor the figures prove nothing and must be silent: %+v", got)
	}
}

func TestAggregateRemoteStatsFoldsTools(t *testing.T) {
	rows := []logstore.CallStat{
		{Server: "s", Tool: "a", Calls: 30, Errors: 3, P95: 100},
		{Server: "s", Tool: "b", Calls: 30, Errors: 0, P95: 900},
		{Server: "t", Tool: "a", Calls: 5, Errors: 0, P95: 10},
	}
	got := aggregateRemoteStats(rows)
	st := got["s"]
	if st.Calls != 60 || st.Errors != 3 || st.P95Ms != 900 {
		t.Errorf("folded wrong: %+v", st)
	}
	if st.ErrRate != 0.05 {
		t.Errorf("error rate = %v, want 0.05", st.ErrRate)
	}
}
