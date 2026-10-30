package daemon

import (
	"context"
	"encoding/json"
	"errors"
	"testing"
	"time"

	"github.com/dezren39/mcpx/internal/config"
	"github.com/dezren39/mcpx/internal/mcpclient"
	"github.com/dezren39/mcpx/internal/pool"
)

// The write policy gate (#1979), in the destructive_test.go fixture
// style: a pool over a locally declared server, its schemas seeded
// into the cache with SetSchemas so no process starts, and a registry
// whose consumer policy is the one under test. The registry here has
// no broker unless a test installs one, which is itself one of the
// cases: ask with nobody to answer must refuse.

func writePolicyPool(t *testing.T, writePolicy string, annotations string) *pool.Pool {
	t.Helper()
	p := pool.New(&config.Resolved{
		Server:      &config.Server{Command: "demo"},
		Namespace:   "demo",
		WritePolicy: writePolicy,
	})
	p.SetSchemas(
		[]mcpclient.Tool{{Name: "wipe", Annotations: json.RawMessage(annotations)}},
		nil, nil, "", time.Now(), time.Time{}, false)
	return p
}

func writePolicyRegistry(mode string) *Registry {
	return &Registry{consumer: ConsumerPolicy{WritePolicy: mode}}
}

// (a) Deny refuses a write tool -- an unannotated tool, which the
// specification's defaults read as a possibly destructive write.
func TestWritePolicyDenyRefusesAWriteTool(t *testing.T) {
	r := writePolicyRegistry("deny")
	p := writePolicyPool(t, "", ``)
	asked, err := r.applyWritePolicy(context.Background(), p, "wipe", config.CallContext{}, map[string]any{"id": 1})
	if asked {
		t.Error("a denied call must not report it asked")
	}
	var denied ErrWriteDenied
	if !errors.As(err, &denied) {
		t.Fatalf("applyWritePolicy = %v, want ErrWriteDenied", err)
	}
	if got := failureStatus(err); got != 403 {
		t.Errorf("failureStatus(ErrWriteDenied) = %d, want 403", got)
	}
}

// (b) Allow passes a write tool without a broker: no error, no ask,
// and no schema-dependent refusal on the way.
func TestWritePolicyAllowPassesAWriteToolWithoutBroker(t *testing.T) {
	r := writePolicyRegistry("allow")
	p := writePolicyPool(t, "", ``)
	asked, err := r.applyWritePolicy(context.Background(), p, "wipe", config.CallContext{}, map[string]any{"id": 1})
	if err != nil {
		t.Fatalf("applyWritePolicy = %v, want nil", err)
	}
	if asked {
		t.Error("allow must not ask")
	}
}

// (c) Ask with no broker refuses with ErrNotConfirmed: a caller that
// cannot answer gets a clear refusal naming the escape, not a call
// that proceeds unasked or waits out a deadline.
func TestWritePolicyAskWithNoBrokerRefuses(t *testing.T) {
	r := writePolicyRegistry("ask")
	p := writePolicyPool(t, "", ``)
	asked, err := r.applyWritePolicy(context.Background(), p, "wipe", config.CallContext{}, map[string]any{"id": 1})
	if asked {
		t.Error("an unanswered ask must not report it asked")
	}
	var notConfirmed ErrNotConfirmed
	if !errors.As(err, &notConfirmed) {
		t.Fatalf("applyWritePolicy = %v, want ErrNotConfirmed", err)
	}
}

// (d) A per-server writePolicy beats the daemon mode, in both
// directions: deny on the server refuses under a daemon-wide allow,
// and allow on the server passes under a daemon-wide deny.
func TestWritePolicyPerServerOverrideBeatsDaemonMode(t *testing.T) {
	r := writePolicyRegistry("allow")
	p := writePolicyPool(t, "deny", ``)
	if _, err := r.applyWritePolicy(context.Background(), p, "wipe", config.CallContext{}, nil); err == nil {
		t.Error("server deny under daemon allow: want a refusal, got nil")
	} else {
		var denied ErrWriteDenied
		if !errors.As(err, &denied) {
			t.Errorf("server deny under daemon allow: got %v, want ErrWriteDenied", err)
		}
	}

	r = writePolicyRegistry("deny")
	p = writePolicyPool(t, "allow", ``)
	asked, err := r.applyWritePolicy(context.Background(), p, "wipe", config.CallContext{}, nil)
	if err != nil || asked {
		t.Errorf("server allow under daemon deny: asked=%v err=%v, want a pass", asked, err)
	}
}

// A read-only tool is not a write, so even deny lets it through.
func TestWritePolicyDenyPassesAReadOnlyTool(t *testing.T) {
	r := writePolicyRegistry("deny")
	p := writePolicyPool(t, "", `{"readOnlyHint":true}`)
	asked, err := r.applyWritePolicy(context.Background(), p, "wipe", config.CallContext{}, nil)
	if err != nil || asked {
		t.Errorf("read-only under deny: asked=%v err=%v, want a pass", asked, err)
	}
}
