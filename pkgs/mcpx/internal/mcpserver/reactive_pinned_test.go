package mcpserver_test

import (
	"context"
	"encoding/json"
	"testing"
	"time"

	"github.com/developing-today/code/pkgs/mcpx/internal/catalog"
	"github.com/developing-today/code/pkgs/mcpx/internal/mcpserver"
)

// pinnedSurfaceServer builds a reactive server the way cli/serve.go does:
// PinnedTools and the working set are both seeded from the same pinned
// list, so every pinned tool starts out in the working set as
// OriginPinned (#3182).
func pinnedSurfaceServer(t *testing.T, pinned []string) *mcpserver.Server {
	t.Helper()
	s := mcpserver.New(newBackend(), "mcpx", "test")
	var extras []mcpserver.Extra
	for _, name := range []string{"alpha", "beta", "gamma"} {
		n := name
		extras = append(extras, mcpserver.Extra{
			Tool: mcpserver.Tool{Name: n, Description: "tool " + n},
			Call: func(ctx context.Context, raw json.RawMessage) (string, error) { return "ok", nil },
		})
	}
	s = s.WithExtras(extras)
	s.Reactive = true
	s.PinnedTools = pinned
	s.ReactiveWorkingSet = catalog.NewWorkingSet(24, 2*time.Hour, pinned)
	return s
}

func surfaceNames(t *testing.T, s *mcpserver.Server) []string {
	t.Helper()
	tools, err := s.SurfaceForTest(context.Background())
	if err != nil {
		t.Fatalf("surface failed: %v", err)
	}
	names := make([]string, 0, len(tools))
	for _, tool := range tools {
		names = append(names, tool.Name)
	}
	return names
}

func assertSurface(t *testing.T, got, want []string) {
	t.Helper()
	if len(got) != len(want) {
		t.Fatalf("surface = %v, want %v", got, want)
	}
	for i := range want {
		if got[i] != want[i] {
			t.Fatalf("surface = %v, want %v", got, want)
		}
	}
	counts := map[string]int{}
	for _, n := range got {
		counts[n]++
		if counts[n] > 1 {
			t.Fatalf("tool %q appears %d times in surface %v", n, counts[n], got)
		}
	}
}

// Pinned-only: each pinned tool appears exactly once even though the
// working set also carries it as OriginPinned.
func TestReactiveSurfacePinnedOnly(t *testing.T) {
	s := pinnedSurfaceServer(t, []string{"alpha"})
	assertSurface(t, surfaceNames(t, s),
		[]string{"request_tools", "batch_call", "alpha"})
}

// Pinned + activated: an activated tool appears once; activating a tool
// that is also pinned does not duplicate it either.
func TestReactiveSurfacePinnedPlusActivated(t *testing.T) {
	s := pinnedSurfaceServer(t, []string{"alpha"})
	s.ReactiveWorkingSet.Add("", "beta", catalog.OriginSticky)
	s.ReactiveWorkingSet.Add("", "alpha", catalog.OriginSticky)
	assertSurface(t, surfaceNames(t, s),
		[]string{"request_tools", "batch_call", "alpha", "beta"})
}

// No pinned tools: the surface is the meta tools plus the working set.
func TestReactiveSurfaceNoPinned(t *testing.T) {
	s := pinnedSurfaceServer(t, nil)
	s.ReactiveWorkingSet.Add("", "beta", catalog.OriginSticky)
	assertSurface(t, surfaceNames(t, s),
		[]string{"request_tools", "batch_call", "beta"})
}
