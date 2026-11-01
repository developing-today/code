package api_test

import (
	"testing"

	"github.com/developing-today/code/pkgs/mcpx/internal/api"
)

// Every operation in the table declares a permission, and the declared
// tier is the one its flags derive — the pin that makes a future policy
// change a deliberate diff (#1968).
func TestEveryOperationDeclaresItsPermission(t *testing.T) {
	ops := api.Ops()
	if len(ops) == 0 {
		t.Fatal("no operations; the table changed shape and this test checks nothing")
	}
	for _, op := range ops {
		if op.Permission == "" {
			t.Errorf("operation %s declares no permission", op.Name)
			continue
		}
		if want := api.DerivePermission(op); op.Permission != want {
			t.Errorf("operation %s declares permission %q, its flags (admin=%v mutating=%v destructive=%v) derive %q",
				op.Name, op.Permission, op.Admin, op.Mutating, op.Destructive, want)
		}
	}
}

// The permissive stage, pinned: every principal the surfaces can
// construct resolves to the owner and is allowed every tier. If a future
// change makes any of these answers no, it must edit this test to say so.
func TestPermissiveStageResolvesToOwnerAndAllowsEverything(t *testing.T) {
	principals := []api.Principal{
		api.UnixPeerPrincipal(1000),
		api.UnixPrincipal(),
		api.TCPPrincipal("127.0.0.1:51000"),
		api.InProcessPrincipal(),
		api.AnonymousPrincipal(),
	}
	tiers := []api.Permission{api.PermissionRead, api.PermissionRun, api.PermissionAdmin}
	for _, p := range principals {
		if !p.Owner {
			t.Errorf("principal %+v does not resolve to the owner; the permissive stage says every principal does", p)
		}
		for _, tier := range tiers {
			if !p.Allows(tier) {
				t.Errorf("principal %+v refuses tier %q; the permissive stage allows everything", p, tier)
			}
		}
	}
}

// Authorize answers yes and records that it did — the record is what
// lets a future policy ask what it would have refused.
func TestAuthorizeRecordsItsDecision(t *testing.T) {
	op, ok := api.ByName("health")
	if !ok {
		t.Fatal("health operation missing from the table")
	}
	before := len(api.Decisions())
	if !api.Authorize(api.UnixPeerPrincipal(1000), op) {
		t.Fatal("Authorize refused in the permissive stage")
	}
	got := api.Decisions()
	if len(got) != before+1 {
		t.Fatalf("decisions grew by %d, want 1", len(got)-before)
	}
	d := got[len(got)-1]
	if d.Op != "health" || d.Permission != api.PermissionRead || !d.Allowed {
		t.Errorf("recorded decision = %+v, want health/read/allowed", d)
	}
	if d.Principal.Surface != "unix" || d.Principal.ID != "uid=1000" {
		t.Errorf("recorded principal = %+v, want the unix peer uid=1000", d.Principal)
	}
}

// ByRoute names the same operation the mux pattern would: it is the
// lookup the daemon's authorisation funnel records decisions through.
func TestByRouteFindsDeclaredOperations(t *testing.T) {
	op, ok := api.ByRoute("GET /v1/health")
	if !ok || op.Name != "health" {
		t.Errorf("ByRoute(GET /v1/health) = %q, %v; want health, true", op.Name, ok)
	}
	if _, ok := api.ByRoute("GET /v1/no-such-route"); ok {
		t.Error("ByRoute found an operation for a route no operation declares")
	}
}
