// lane-scratch: code-c /tmp/lane-code-c-20261114-1635 task 16927
package pluginsession

import (
	"encoding/json"
	"testing"
	"time"

	"github.com/developing-today/code/pkgs/mcpx/internal/events"
)

// The manifest-hook oracles (#10046): a hook fires to the owning
// plugin on a matching event; a non-matching event does not fire
// it; session end drops the subscriptions; two sessions with
// identical declarations receive only their own deliveries.

func newResolver() (*events.Bus, *Registry, *Resolver) {
	bus := events.New(64)
	reg := &Registry{}
	return bus, reg, NewResolver(reg, bus)
}

// deliveries subscribes to the delivery stream addressed to one
// session -- the plugin's own view of its hooks firing.
func deliveries(t *testing.T, bus *events.Bus, sessionID string) *events.Subscription {
	t.Helper()
	sub, _ := bus.Subscribe(events.Filter{Kinds: []string{string(DeliveryKind)}, Session: sessionID}, 0)
	t.Cleanup(sub.Close)
	return sub
}

func recvDelivery(t *testing.T, sub *events.Subscription) (events.Event, Delivery) {
	t.Helper()
	select {
	case e := <-sub.C:
		var d Delivery
		if err := json.Unmarshal(e.Data, &d); err != nil {
			t.Fatalf("delivery data does not decode: %v", err)
		}
		return e, d
	case <-time.After(2 * time.Second):
		t.Fatal("timed out waiting for a hook delivery")
		return events.Event{}, Delivery{}
	}
}

func expectNoDelivery(t *testing.T, sub *events.Subscription, what string) {
	t.Helper()
	select {
	case e := <-sub.C:
		t.Fatalf("%s: unexpected delivery: %+v", what, e)
	case <-time.After(250 * time.Millisecond):
	}
}

func TestManifestHookFiresToOwningPluginOnMatchingEvent(t *testing.T) {
	bus, reg, r := newResolver()
	sess := r.Open("greeter", nil, []string{"elicit"})
	if len(sess.Owned) != 1 || sess.Owned[0] != (OwnedItem{Kind: "hook", Name: "elicit"}) {
		t.Fatalf("live subscription must be a session-owned hook item: %+v", sess.Owned)
	}
	if got, _ := reg.Get(sess.ID); len(got.Owned) != 1 {
		t.Fatalf("owned hook must be visible through the registry: %+v", got)
	}
	sub := deliveries(t, bus, sess.ID)
	bus.Publish(events.Event{Kind: events.ElicitOpened})
	e, d := recvDelivery(t, sub)
	if e.Session != sess.ID || d.Plugin != "greeter" || d.Hook != "elicit" || d.Event.Kind != events.ElicitOpened {
		t.Fatalf("delivery must name the owning session, plugin, hook and event: event=%+v delivery=%+v", e, d)
	}
}

func TestManifestHookDoesNotFireOnNonMatchingEvent(t *testing.T) {
	bus, _, r := newResolver()
	sess := r.Open("greeter", nil, []string{"elicit"})
	sub := deliveries(t, bus, sess.ID)
	bus.Publish(events.Event{Kind: events.ServerStarted})
	bus.Publish(events.Event{Kind: events.CallFinished})
	expectNoDelivery(t, sub, "non-matching events")
}

func TestSessionCloseDropsSubscriptions(t *testing.T) {
	bus, _, r := newResolver()
	base := bus.Subscribers()
	sess := r.Open("greeter", nil, []string{"elicit"})
	if got := bus.Subscribers(); got != base+1 {
		t.Fatalf("open must add one bus subscription: base %d, got %d", base, got)
	}
	sub := deliveries(t, bus, sess.ID)
	if _, ok := r.Close(sess.ID); !ok {
		t.Fatal("close of a live session must report true")
	}
	if got := bus.Subscribers(); got != base+1 { // +1 is this test's own collector
		t.Fatalf("close must drop the hook subscription: got %d subscribers", got)
	}
	bus.Publish(events.Event{Kind: events.ElicitOpened})
	expectNoDelivery(t, sub, "post-close event")
}

func TestTwoSessionsIdenticalDeclarationsGetOnlyTheirOwn(t *testing.T) {
	bus, _, r := newResolver()
	a := r.Open("greeter", nil, []string{"elicit"})
	b := r.Open("greeter", nil, []string{"elicit"})
	subA := deliveries(t, bus, a.ID)
	subB := deliveries(t, bus, b.ID)
	bus.Publish(events.Event{Kind: events.ElicitAnswered})
	eA, _ := recvDelivery(t, subA)
	eB, _ := recvDelivery(t, subB)
	if eA.Session != a.ID || eB.Session != b.ID {
		t.Fatalf("each delivery must be addressed to its own session: A=%s B=%s", eA.Session, eB.Session)
	}
	// Closing one session leaves the other's hooks live.
	if _, ok := r.Close(a.ID); !ok {
		t.Fatal("close A")
	}
	bus.Publish(events.Event{Kind: events.ElicitAnswered})
	_, dB := recvDelivery(t, subB)
	if dB.Hook != "elicit" {
		t.Fatalf("surviving session's delivery is wrong: %+v", dB)
	}
	expectNoDelivery(t, subA, "closed session")
}

func TestNonHookableDeclarationsStayRecordedOnly(t *testing.T) {
	bus, _, r := newResolver()
	base := bus.Subscribers()
	// "onSuccess" is a blocking phase name from the hook vocabulary,
	// not a bus kind prefix: recorded, never subscribed.
	sess := r.Open("greeter", nil, []string{"onSuccess"})
	if len(sess.Hooks) != 1 || len(sess.Owned) != 0 {
		t.Fatalf("phase-name declaration must stay recorded-only: %+v", sess)
	}
	if got := bus.Subscribers(); got != base {
		t.Fatalf("phase-name declaration must not subscribe: %d subscribers", got)
	}
	// A wildcard declaration subscribes and fires.
	wild := r.Open("watcher", nil, []string{"*"})
	if len(wild.Owned) != 1 {
		t.Fatalf("wildcard declaration must resolve live: %+v", wild)
	}
	sub := deliveries(t, bus, wild.ID)
	bus.Publish(events.Event{Kind: events.ServerStopped})
	_, d := recvDelivery(t, sub)
	if d.Hook != "*" || d.Event.Kind != events.ServerStopped {
		t.Fatalf("wildcard delivery is wrong: %+v", d)
	}
}
