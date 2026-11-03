package pool_test

import (
	"context"
	"encoding/json"
	"path/filepath"
	"testing"
	"time"

	"github.com/developing-today/code/pkgs/mcpx/internal/config"
	"github.com/developing-today/code/pkgs/mcpx/internal/pool"
	"github.com/developing-today/code/pkgs/mcpx/internal/testsupport"
)

// An adopted child keeps the executable identity it was started with. The
// command is relative ("./server") and resolves only under the scope root,
// so the identity exists solely as the instance's effective-spawn-cwd
// stamp. If the handoff dropped it, the successor's first exec check would
// read a zero stamp, treat the unchanged executable as newly changed, and
// restart the adopted child for no reason.
func TestHandoffCarriesExecIdentityForRelativeCommand(t *testing.T) {
	setExecWatch(t, execTick, 10*time.Second)
	fake := testsupport.FakeMCPBinary(t)
	root := t.TempDir()
	copyFake(t, fake, filepath.Join(root, "server"), nil)

	ex := &config.Extras{Sharing: config.SharingExclusive, Scope: config.ScopeSession, Max: 2}
	from := pool.New(resolved(t, "./server", ex))
	defer from.Close()
	scope := config.ResolvedScope{Key: "ws1", Root: root}
	lease, err := from.Acquire(context.Background(), scope)
	if err != nil {
		t.Fatal(err)
	}
	lease.Release()

	ph, err := from.Detach()
	if err != nil {
		t.Fatalf("detach: %v", err)
	}
	if len(ph.Instances) != 1 {
		t.Fatalf("want the one child handed on, got %d", len(ph.Instances))
	}
	h := ph.Instances[0]
	wantPath, err := filepath.EvalSymlinks(filepath.Join(root, "server"))
	if err != nil {
		t.Fatal(err)
	}
	if h.ExecStamp.Path != wantPath {
		t.Fatalf("handoff exec stamp path = %q, want %q", h.ExecStamp.Path, wantPath)
	}
	if h.ExecIdentity == "" {
		t.Fatal("handoff carries no exec identity for the adopted child")
	}

	// The handoff metadata crosses a process boundary as JSON; the
	// identity must survive the round trip. (The stdio pipes themselves
	// travel beside the JSON as file descriptors, so adoption below uses
	// the original handoff, not the unmarshalled copy.)
	wire, err := json.Marshal(ph)
	if err != nil {
		t.Fatal(err)
	}
	var ph2 pool.PoolHandoff
	if err := json.Unmarshal(wire, &ph2); err != nil {
		t.Fatal(err)
	}
	if ph2.Instances[0].ExecIdentity != h.ExecIdentity || ph2.Instances[0].ExecStamp != h.ExecStamp {
		t.Fatalf("exec identity lost on the wire: %+v", ph2.Instances[0])
	}

	to := pool.New(resolved(t, "./server", ex))
	defer to.Close()
	ads, err := pool.Adopt(ph.Instances)
	if err != nil {
		t.Fatalf("adopt: %v", err)
	}
	from.Commit()
	to.Attach(ads, ph.Seq)

	pids := childPIDs(to)
	if len(pids) != 1 {
		t.Fatalf("want the adopted child serving, got pids %v", pids)
	}
	// Several exec-watch ticks pass with the executable untouched: the
	// adopted child must still be the one serving.
	time.Sleep(300 * time.Millisecond)
	res, err := to.Call(context.Background(), scope, "state", map[string]any{})
	if err != nil {
		t.Fatalf("state after takeover: %v", err)
	}
	var st struct {
		PID int `json:"pid"`
	}
	if err := json.Unmarshal([]byte(textOf(t, res)), &st); err != nil {
		t.Fatalf("decode state: %v", err)
	}
	if st.PID != pids[0] {
		t.Fatalf("adopted child pid %d was replaced by pid %d with its executable unchanged", pids[0], st.PID)
	}
}
