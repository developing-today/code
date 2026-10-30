package mcpserver_test

import (
	"context"
	"strings"
	"testing"

	"github.com/dezren39/mcpx/internal/mcpserver"
)

// The advertised-capability gate (#2436): a method whose gating
// capability was not advertised to the peer is -32601 at dispatch.
// Legacy peers are judged against the capability map snapshotted at
// initialize, never a dispatch-time re-evaluation.

func gateInit(t *testing.T, s *mcpserver.Server, c *mcpserver.Conn, version string) {
	t.Helper()
	resp := s.HandleOn(context.Background(), c, mcpserver.Request(1, "initialize",
		map[string]any{"protocolVersion": version, "capabilities": map[string]any{}}))
	if resp != nil && resp.Error != nil {
		t.Fatalf("initialize %s: %v", version, resp.Error)
	}
}

func TestCapabilityGateRefusesUnadvertisedMethods(t *testing.T) {
	s := mcpserver.New(newBackend(), "mcpx", "test")
	// A connection that cannot push: at 2024-11-05 the advertisement
	// carries neither completions (the revision does not define it)
	// nor resources.subscribe (nothing can push).
	c := s.ConnForTest("gate-old")
	gateInit(t, s, c, "2024-11-05")

	cases := []struct {
		method string
		params any
		want   string
	}{
		{"completion/complete", map[string]any{}, "capability completions not advertised"},
		{"resources/subscribe", map[string]any{"uri": "mcpx://x"}, "capability resources.subscribe not advertised"},
		{"resources/unsubscribe", map[string]any{"uri": "mcpx://x"}, "capability resources.subscribe not advertised"},
	}
	for _, tc := range cases {
		resp := s.HandleOn(context.Background(), c, mcpserver.Request(2, tc.method, tc.params))
		if resp == nil || resp.Error == nil {
			t.Fatalf("%s: not refused, resp = %+v", tc.method, resp)
		}
		if resp.Error.Code != -32601 || !strings.Contains(resp.Error.Message, tc.want) {
			t.Errorf("%s: error = %d %q, want -32601 naming %q",
				tc.method, resp.Error.Code, resp.Error.Message, tc.want)
		}
	}
}

func TestCapabilityGateAdmitsAdvertisedMethods(t *testing.T) {
	s := mcpserver.New(newBackend(), "mcpx", "test")
	rec := &recorder{}
	// A connection that can push, at a revision that defines
	// completions: both gated capabilities are advertised.
	c := s.ConnWithSend("gate-new", rec.send)
	gateInit(t, s, c, "2025-03-26")

	// resources/subscribe succeeds outright.
	resp := s.HandleOn(context.Background(), c, mcpserver.Request(2, "resources/subscribe",
		map[string]any{"uri": "mcpx://x"}))
	if resp != nil && resp.Error != nil {
		t.Fatalf("resources/subscribe refused although advertised: %v", resp.Error)
	}

	// completion/complete passes the gate and is answered by the
	// completion handler itself — its own -32602 for a missing ref,
	// not the gate's -32601.
	resp = s.HandleOn(context.Background(), c, mcpserver.Request(3, "completion/complete",
		map[string]any{}))
	if resp == nil || resp.Error == nil {
		t.Fatalf("completion/complete: resp = %+v, want the handler's own answer", resp)
	}
	if resp.Error.Code == -32601 || strings.Contains(resp.Error.Message, "not advertised") {
		t.Errorf("completion/complete was gated although advertised: %d %q",
			resp.Error.Code, resp.Error.Message)
	}
}

func TestCapabilityGateReadsTheInitializeSnapshot(t *testing.T) {
	s := mcpserver.New(newBackend(), "mcpx", "test")
	// No push on this connection, at a revision that DOES define
	// completions: the snapshot advertises completions (presence is
	// version-determined) but resources.subscribe stays false, and the
	// gate must honour that snapshot per method.
	c := s.ConnForTest("gate-snap")
	gateInit(t, s, c, "2025-03-26")

	resp := s.HandleOn(context.Background(), c, mcpserver.Request(2, "resources/subscribe",
		map[string]any{"uri": "mcpx://x"}))
	if resp == nil || resp.Error == nil || resp.Error.Code != -32601 {
		t.Fatalf("resources/subscribe against a subscribe:false snapshot: resp = %+v, want -32601", resp)
	}
	resp = s.HandleOn(context.Background(), c, mcpserver.Request(3, "completion/complete",
		map[string]any{}))
	if resp != nil && resp.Error != nil && resp.Error.Code == -32601 {
		t.Errorf("completion/complete gated although the snapshot advertises completions: %v", resp.Error)
	}
}
