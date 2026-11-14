package cli

import (
	"context"
	"encoding/json"
	"path/filepath"
	"strings"
	"testing"

	"github.com/developing-today/code/pkgs/mcpx/internal/daemon"
	"github.com/developing-today/code/pkgs/mcpx/internal/registry"
	"github.com/developing-today/code/pkgs/mcpx/internal/wantlog"
)

// wantTestApp returns an App whose state directory is a scratch dir, so
// recording never touches the real machine state.
func wantTestApp(t *testing.T) *App {
	t.Helper()
	t.Setenv("MCPX_STATE_DIR", t.TempDir())
	return &App{Paths: daemon.PathsAt(filepath.Join(t.TempDir(), "state"), "")}
}

func TestWantRequestRecordsAndListsBack(t *testing.T) {
	a := wantTestApp(t)
	ctx := context.Background()

	if err := a.CmdWant(ctx, []string{"request", "Calendar", "MCP"}); err != nil {
		t.Fatalf("want request: %v", err)
	}
	if err := a.CmdWant(ctx, []string{"request", "calendar mcp"}); err != nil {
		t.Fatalf("want request: %v", err)
	}

	store := wantlog.New(wantlog.PathFor(a.wantStateDir()))
	groups, err := store.Summaries("", 0)
	if err != nil {
		t.Fatal(err)
	}
	if len(groups) != 1 {
		t.Fatalf("groups = %+v, want one normalised group", groups)
	}
	g := groups[0]
	if g.Kind != wantlog.KindRequest || g.Query != "calendar mcp" || g.Count != 2 {
		t.Errorf("group = %+v, want 2x request/calendar mcp", g)
	}
	if g.FirstSeen.IsZero() || g.LastSeen.IsZero() || g.LastSeen.Before(g.FirstSeen) {
		t.Errorf("group first/last = %v/%v, want a valid window", g.FirstSeen, g.LastSeen)
	}
}

func TestWantUnknownSubcommandIsARequest(t *testing.T) {
	a := wantTestApp(t)
	// `mcpx want calendar mcp` must record "calendar mcp", not fail on a
	// verb named "calendar".
	if err := a.CmdWant(context.Background(), []string{"calendar", "mcp"}); err != nil {
		t.Fatalf("want calendar mcp: %v", err)
	}
	groups, err := wantlog.New(wantlog.PathFor(a.wantStateDir())).Summaries("", 0)
	if err != nil {
		t.Fatal(err)
	}
	if len(groups) != 1 || groups[0].Query != "calendar mcp" {
		t.Errorf("groups = %+v, want one request for calendar mcp", groups)
	}
}

func TestWantListKindFilterAndLimit(t *testing.T) {
	a := wantTestApp(t)
	ctx := context.Background()
	if err := a.CmdWant(ctx, []string{"request", "alpha"}); err != nil {
		t.Fatal(err)
	}
	if err := wantlog.RecordSearchMiss(a.wantStateDir(), "gamma", 0); err != nil {
		t.Fatal(err)
	}
	// Unknown kind is a usage error, not an empty list.
	if err := a.CmdWant(ctx, []string{"list", "--kind", "wish"}); err == nil {
		t.Error("want list --kind wish should fail")
	}
	// Trailing flags reach the list verb (Go's parser stops at "list").
	if err := a.CmdWant(ctx, []string{"list", "--kind", "search-miss"}); err != nil {
		t.Fatalf("want list --kind search-miss: %v", err)
	}
	aJSON := wantTestApp(t)
	aJSON.JSON = true
	if err := aJSON.CmdWant(ctx, []string{"request", "alpha"}); err != nil {
		t.Fatal(err)
	}
	out, err := captureStdout(t, func() error {
		return aJSON.CmdWant(ctx, []string{"list", "--limit", "1"})
	})
	if err != nil {
		t.Fatalf("want list --json: %v", err)
	}
	var decoded []wantlog.Summary
	if err := json.Unmarshal([]byte(out), &decoded); err != nil {
		t.Fatalf("list --json is not a JSON array: %v (%q)", err, out)
	}
	if len(decoded) != 1 {
		t.Errorf("list --limit 1 decoded %d groups, want 1", len(decoded))
	}
}

func TestWantListEmptyPrintsHint(t *testing.T) {
	a := wantTestApp(t)
	out, err := captureStdout(t, func() error {
		return a.CmdWant(context.Background(), []string{"list"})
	})
	if err != nil {
		t.Fatal(err)
	}
	if !strings.Contains(out, "No capability requests") {
		t.Errorf("empty list printed %q, want the no-records hint", out)
	}
}

func TestRegistryEmptySearchRecordsMiss(t *testing.T) {
	a := wantTestApp(t)
	// This is the exact result hook called by registrySearch after a
	// successful query. Supplying an empty result avoids external network
	// access in tests while still exercising the recording decision.
	a.recordRegistrySearch("  Calendar   MCP ", registry.Results{})

	groups, err := wantlog.New(wantlog.PathFor(a.wantStateDir())).Summaries(wantlog.KindSearchMiss, 0)
	if err != nil {
		t.Fatal(err)
	}
	if len(groups) != 1 || groups[0].Query != "calendar mcp" || groups[0].Count != 1 {
		t.Errorf("miss groups = %+v, want one search-miss for the query", groups)
	}
}

func TestRegistryNonEmptySearchRecordsNothing(t *testing.T) {
	a := wantTestApp(t)
	a.recordRegistrySearch("calendar", registry.Results{Servers: []registry.Server{{Name: "calendar"}}})
	groups, err := wantlog.New(wantlog.PathFor(a.wantStateDir())).Summaries("", 0)
	if err != nil {
		t.Fatal(err)
	}
	if len(groups) != 0 {
		t.Errorf("groups after a hit = %+v, want nothing recorded", groups)
	}
}
