package daemon

import (
	"path/filepath"
	"testing"

	"github.com/developing-today/code/pkgs/mcpx/internal/registry"
	"github.com/developing-today/code/pkgs/mcpx/internal/wantlog"
)

func TestRegistrySearchMissIsRecordedByTheDaemon(t *testing.T) {
	state := filepath.Join(t.TempDir(), "state")
	s := &Server{paths: Paths{State: state}}
	s.recordRegistrySearch("  Calendar   MCP ", registry.Results{})

	entries, err := wantlog.New(wantlog.PathFor(state)).List()
	if err != nil {
		t.Fatal(err)
	}
	if len(entries) != 1 {
		t.Fatalf("recorded %d entries, want one miss", len(entries))
	}
	if entries[0].Kind != wantlog.KindSearchMiss || entries[0].Query != "calendar mcp" {
		t.Errorf("entry = %+v, want a normalised search-miss", entries[0])
	}
}

func TestRegistrySearchHitIsNotRecordedByTheDaemon(t *testing.T) {
	state := filepath.Join(t.TempDir(), "state")
	s := &Server{paths: Paths{State: state}}
	s.recordRegistrySearch("calendar", registry.Results{Servers: []registry.Server{{Name: "calendar"}}})

	entries, err := wantlog.New(wantlog.PathFor(state)).List()
	if err != nil {
		t.Fatal(err)
	}
	if len(entries) != 0 {
		t.Errorf("recorded %+v after a hit, want nothing", entries)
	}
}

func TestRegistryMissUsesDefaultStateDirectory(t *testing.T) {
	state := t.TempDir()
	t.Setenv("XDG_STATE_HOME", state)
	s := &Server{}
	s.recordRegistrySearch("not-a-server", registry.Results{})

	entries, err := wantlog.New(wantlog.PathFor(filepath.Join(state, "mcpx"))).List()
	if err != nil {
		t.Fatal(err)
	}
	if len(entries) != 1 || entries[0].Query != "not-a-server" {
		t.Errorf("entries = %+v, want one miss in MCPX_STATE_DIR", entries)
	}
}
