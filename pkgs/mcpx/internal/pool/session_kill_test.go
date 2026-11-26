package pool_test

import (
	"context"
	"testing"

	"github.com/developing-today/code/pkgs/mcpx/internal/config"
	"github.com/developing-today/code/pkgs/mcpx/internal/pool"
	"github.com/developing-today/code/pkgs/mcpx/internal/testsupport"
)

// Session kill (#1931 slice 2): release detaches the caller without ending
// the instance, kill ends it. The difference only shows while the instance
// is held: ReleaseKey leaves a busy instance alone, KillKey ends it anyway.
// sessions_read observes the difference as the instance leaving the pool's
// Status (live 1 -> 0).
func TestKillKeyEndsAHeldInstanceWhileReleaseKeyDoesNot(t *testing.T) {
	bin := testsupport.FakeMCPBinary(t)
	p := pool.New(resolved(t, bin, &config.Extras{Sharing: config.SharingExclusive, Scope: config.ScopeSession, Max: 2}))
	defer p.Close()

	ctx := context.Background()
	lease, err := p.Acquire(ctx, config.ResolvedScope{Key: "s1"})
	if err != nil {
		t.Fatalf("acquire: %v", err)
	}
	if st := p.Status(); st.Live != 1 {
		t.Fatalf("want 1 live, got %d", st.Live)
	}

	// Release does not kill: the held instance stays live and stays listed.
	if n := p.ReleaseKey("s1"); n != 0 {
		t.Fatalf("ReleaseKey on a held instance must end nothing, ended %d", n)
	}
	if st := p.Status(); st.Live != 1 {
		t.Fatalf("release ended a held instance: %d live, want 1", st.Live)
	}

	// Kill ends it even though it is held, and it leaves the inventory.
	if n := p.KillKey("s1"); n != 1 {
		t.Fatalf("KillKey on a held instance must end 1, ended %d", n)
	}
	if st := p.Status(); st.Live != 0 {
		t.Fatalf("killed instance still listed: %d live, want 0", st.Live)
	}

	// Returning the stale lease after its instance was killed is a no-op,
	// not a panic and not a resurrection.
	lease.Release()
	if st := p.Status(); st.Live != 0 {
		t.Fatalf("lease return resurrected the instance: %d live", st.Live)
	}

	// An unknown key ends nothing.
	if n := p.KillKey("nope"); n != 0 {
		t.Fatalf("KillKey on an unknown key must end 0, ended %d", n)
	}
}
