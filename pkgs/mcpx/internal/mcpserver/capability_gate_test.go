package mcpserver_test

import (
	"context"
	"strings"
	"testing"

	"github.com/developing-today/code/pkgs/mcpx/internal/mcpserver"
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
	// does not carry resources.subscribe (nothing can push).
	// completions is absent too, but that absence cannot gate
	// completion/complete: the revision has no capability to declare
	// it with, while the method itself is older than the capability
	// (see FeatCompletions) — the gate applies only where the
	// capability exists to be advertised (#2716).
	c := s.ConnForTest("gate-old")
	gateInit(t, s, c, "2024-11-05")

	cases := []struct {
		method string
		params any
		want   string
	}{
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

// TestCapabilityGateAdmitsCompletionBeforeTheCapabilityExisted pins
// the other half of the completions rule: at a revision that does
// not define the completions capability (2024-11-05), the gate must
// not refuse completion/complete — there was nothing the peer could
// have been advertised. The request reaches the completion handler,
// whose own -32602 for a missing ref is the answer. (The conformance
// suite's completion-max-100 case at 2024-11-05 is the same rule
// from the spec side; the #2436 gate's original refusal here was
// the desync #2716 records.)
func TestCapabilityGateAdmitsCompletionBeforeTheCapabilityExisted(t *testing.T) {
	s := mcpserver.New(newBackend(), "mcpx", "test")
	c := s.ConnForTest("gate-old-completion")
	gateInit(t, s, c, "2024-11-05")

	resp := s.HandleOn(context.Background(), c, mcpserver.Request(2, "completion/complete",
		map[string]any{}))
	if resp == nil || resp.Error == nil {
		t.Fatalf("completion/complete: resp = %+v, want the handler's own answer", resp)
	}
	if resp.Error.Code != -32602 || strings.Contains(resp.Error.Message, "not advertised") {
		t.Errorf("completion/complete: error = %d %q, want -32602 from the completion handler, not the gate's -32601",
			resp.Error.Code, resp.Error.Message)
	}
}

func TestCapabilityGateAdmitsAdvertisedMethods(t *testing.T) {
	s := mcpserver.New(newBackend(), "mcpx", "test")
	// capabilities() advertises resources.subscribe only when a
	// notifier exists AND the connection can push: configure the
	// notifier before initializing, as the subscription tests do, so
	// the snapshot actually carries subscribe:true.
	s.Notify = &fakeNotifier{got: make(chan mcpserver.ListenFilter, 1), fire: make(chan [2]any, 4)}
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

// The #7844 widening of the gate: the skills extension methods
// (skills/list, skills/get, resources/directory/read) are gated on
// the extensions.skills advertisement, which exists only in
// 2026-07-28 and only when skills are served. The era disposition
// (docs/spec/revision-conflicts.md item C24): a 2026-07-28 peer
// whose advertisement carries no skills extension gets -32601; a
// pre-2026 peer keeps liberal acceptance, because its schema has
// no extensions advertisement to gate on.

func TestCapabilityGateRefusesUnadvertisedSkillsExtension(t *testing.T) {
	// ExtrasOnly serves no skills, so capabilities() for a
	// 2026-07-28 peer declares extensions without the skills entry:
	// the extension methods are unadvertised for that peer.
	s := mcpserver.New(newBackend(), "mcpx", "test")
	s.ExtrasOnly = true
	c := s.ConnForTest("gate-skills-modern")

	for _, method := range []string{"skills/list", "skills/get", "resources/directory/read"} {
		resp := s.HandleOn(context.Background(), c, mcpserver.Request(2, method,
			modernParams(map[string]any{"uri": "skill://x"})))
		if resp == nil || resp.Error == nil {
			t.Fatalf("%s: not refused, resp = %+v", method, resp)
		}
		if resp.Error.Code != -32601 || !strings.Contains(resp.Error.Message, "capability extensions.skills not advertised") {
			t.Errorf("%s: error = %d %q, want -32601 naming the extensions.skills capability",
				method, resp.Error.Code, resp.Error.Message)
		}
	}
}

func TestCapabilityGateAdmitsAdvertisedSkillsExtension(t *testing.T) {
	// A default server serves mcpx's own skills, so a 2026-07-28
	// peer is advertised the skills extension and skills/list is
	// answered by the skills handler, not refused by the gate.
	s := mcpserver.New(newBackend(), "mcpx", "test")
	c := s.ConnForTest("gate-skills-advertised")

	resp := s.HandleOn(context.Background(), c, mcpserver.Request(2, "skills/list", modernParams(nil)))
	if resp != nil && resp.Error != nil {
		t.Fatalf("skills/list refused although the skills extension is advertised: %v", resp.Error)
	}
}

func TestCapabilityGateKeepsLegacySkillsAcceptance(t *testing.T) {
	// A pre-2026 peer's schema has no extensions advertisement, so
	// the gate does not apply to the extension methods at all
	// (gateEraApplies): even on a server serving no skills, the
	// answer is the skills handler's own, never the gate's -32601.
	s := mcpserver.New(newBackend(), "mcpx", "test")
	s.ExtrasOnly = true
	c := s.ConnForTest("gate-skills-legacy")
	gateInit(t, s, c, "2025-03-26")

	resp := s.HandleOn(context.Background(), c, mcpserver.Request(2, "skills/list", map[string]any{}))
	if resp != nil && resp.Error != nil && resp.Error.Code == -32601 &&
		strings.Contains(resp.Error.Message, "not advertised") {
		t.Fatalf("skills/list gated for a legacy peer: %v", resp.Error)
	}
}
