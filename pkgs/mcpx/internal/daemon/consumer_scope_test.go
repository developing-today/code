package daemon

import (
	"testing"

	"github.com/developing-today/code/pkgs/mcpx/internal/pool"
)

func TestResolvedInstanceScopePreservesRootForNewLane(t *testing.T) {
	got := resolvedInstanceScope(pool.InstanceStatus{Key: "worktree:/checkout", Root: "/checkout"})
	if got.Key != "worktree:/checkout" || got.Root != "/checkout" {
		t.Fatalf("selected instance scope = %+v, want key and root from instance", got)
	}
}
