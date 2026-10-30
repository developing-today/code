package cli

import (
	"context"
	"errors"
	"testing"

	"github.com/dezren39/mcpx/internal/mcpserver"
)

// PM-13 (#1942): a failed first build must not be cached for the
// daemon's life -- the next request retries the build.
func TestLazyMCPFailedBuildIsNotCached(t *testing.T) {
	calls := 0
	l := &lazyMCP{
		inputs: func() string { return "fp1" },
		build: func(ctx context.Context) (*mcpserver.Server, error) {
			calls++
			if calls == 1 {
				return nil, errors.New("transient: document fetch failed")
			}
			return &mcpserver.Server{}, nil
		},
	}
	if _, err := l.get(context.Background()); err == nil {
		t.Fatal("first build: want the transient error")
	}
	srv, err := l.get(context.Background())
	if err != nil || srv == nil {
		t.Fatalf("second build: got (%v, %v), want a retried, successful build", srv, err)
	}
	if calls != 2 {
		t.Errorf("build calls = %d, want 2 (failure retried, not cached)", calls)
	}
	// And the successful build IS cached while inputs are unchanged.
	if _, err := l.get(context.Background()); err != nil {
		t.Fatal(err)
	}
	if calls != 2 {
		t.Errorf("build calls = %d after a cached hit, want 2", calls)
	}
}

// PM-13 (#1942): when the declared inputs change, the surface is
// rebuilt instead of serving the first build forever.
func TestLazyMCPRebuildsWhenInputsChange(t *testing.T) {
	fp := "fp1"
	calls := 0
	l := &lazyMCP{
		inputs: func() string { return fp },
		build: func(ctx context.Context) (*mcpserver.Server, error) {
			calls++
			return &mcpserver.Server{}, nil
		},
	}
	first, err := l.get(context.Background())
	if err != nil {
		t.Fatal(err)
	}
	fp = "fp2" // an adapter file changed on disk
	second, err := l.get(context.Background())
	if err != nil {
		t.Fatal(err)
	}
	if calls != 2 {
		t.Errorf("build calls = %d, want 2 after an input change", calls)
	}
	if first == second {
		t.Error("rebuild returned the same server instance")
	}
}

// PM-13 follow-up (#1942): replacing the surface retires the old server
// (its sessions are ended), rather than abandoning it with live streams.
func TestLazyMCPRetiresReplacedSurface(t *testing.T) {
	fp := "fp1"
	var retired []*mcpserver.Server
	l := &lazyMCP{
		inputs: func() string { return fp },
		build: func(ctx context.Context) (*mcpserver.Server, error) {
			return &mcpserver.Server{}, nil
		},
		retire: func(s *mcpserver.Server) { retired = append(retired, s) },
	}
	first, err := l.get(context.Background())
	if err != nil {
		t.Fatal(err)
	}
	if len(retired) != 0 {
		t.Fatalf("first build retired %d servers, want 0", len(retired))
	}
	fp = "fp2"
	second, err := l.get(context.Background())
	if err != nil {
		t.Fatal(err)
	}
	if len(retired) != 1 || retired[0] != first {
		t.Fatalf("retired = %v, want exactly the replaced first surface", retired)
	}
	// A cached hit retires nothing.
	if _, err := l.get(context.Background()); err != nil {
		t.Fatal(err)
	}
	if len(retired) != 1 {
		t.Fatalf("cached hit retired more surfaces: %v", retired)
	}
	_ = second
}
