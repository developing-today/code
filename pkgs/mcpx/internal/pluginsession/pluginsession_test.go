package pluginsession

import (
	"strings"
	"sync"
	"testing"
)

// The packet's named tests for the plugin-session foundation
// (#1873): session issuance + identity, two sessions' registrations
// isolated, teardown drops a dead session's registrations
// atomically, and declared capabilities recorded and reportable
// (decision 0003: declare-only).

func TestOpenIssuesDistinguishableIDs(t *testing.T) {
	var r Registry
	a := r.Open("alpha", nil, nil)
	b := r.Open("beta", nil, nil)
	if a.ID == "" || b.ID == "" {
		t.Fatalf("sessions must carry ids: %+v %+v", a, b)
	}
	if a.ID == b.ID {
		t.Fatalf("two sessions share an id: %s", a.ID)
	}
	for _, s := range []Session{a, b} {
		if !strings.HasPrefix(s.ID, IDPrefix) {
			t.Fatalf("session id %q lacks the %q prefix that distinguishes it from a leasing session id", s.ID, IDPrefix)
		}
	}
	if a.Plugin != "alpha" || b.Plugin != "beta" {
		t.Fatalf("session records the wrong plugin: %+v %+v", a, b)
	}
}

func TestOpenRecordsDeclaredCapabilitiesAndHooks(t *testing.T) {
	var r Registry
	s := r.Open("alpha",
		[]string{"net.out", "fs.read", "net.out", ""},
		[]string{"onSuccess", "onSuccess"})
	if got := s.Capabilities; len(got) != 2 || got[0] != "net.out" || got[1] != "fs.read" {
		t.Fatalf("declared capabilities must be recorded once each, blanks dropped: %v", got)
	}
	if got := s.Hooks; len(got) != 1 || got[0] != "onSuccess" {
		t.Fatalf("declared hooks must be recorded once each: %v", got)
	}
	// Reportable: the same declarations come back through Get/List.
	back, ok := r.Get(s.ID)
	if !ok {
		t.Fatalf("opened session %s not found", s.ID)
	}
	if len(back.Capabilities) != 2 || len(back.Hooks) != 1 {
		t.Fatalf("Get does not report the declarations: %+v", back)
	}
	if listed := r.List(); len(listed) != 1 || listed[0].ID != s.ID {
		t.Fatalf("List does not report the session: %+v", listed)
	}
}

func TestSessionsAreIsolated(t *testing.T) {
	var r Registry
	a := r.Open("alpha", []string{"cap.a"}, nil)
	b := r.Open("beta", []string{"cap.b"}, nil)
	if !r.Own(a.ID, OwnedItem{Kind: "hook", Name: "alpha-hook"}) {
		t.Fatal("Own against a live session refused")
	}
	if !r.Own(a.ID, OwnedItem{Kind: "hook", Name: "alpha-hook"}) {
		t.Fatal("re-recording the same item must be a no-op success")
	}
	if !r.Own(b.ID, OwnedItem{Kind: "var", Name: "beta-var"}) {
		t.Fatal("Own against a live session refused")
	}
	ga, _ := r.Get(a.ID)
	gb, _ := r.Get(b.ID)
	if len(ga.Owned) != 1 || ga.Owned[0].Name != "alpha-hook" {
		t.Fatalf("session a owns the wrong things: %+v", ga.Owned)
	}
	if len(gb.Owned) != 1 || gb.Owned[0].Name != "beta-var" {
		t.Fatalf("session b owns the wrong things: %+v", gb.Owned)
	}
	if len(ga.Capabilities) != 1 || ga.Capabilities[0] != "cap.a" {
		t.Fatalf("session a's declarations leaked or vanished: %+v", ga)
	}
	// Closing b leaves a untouched.
	if _, ok := r.Close(b.ID); !ok {
		t.Fatal("Close of a live session refused")
	}
	ga, ok := r.Get(a.ID)
	if !ok || len(ga.Owned) != 1 {
		t.Fatalf("closing b disturbed a: %+v ok=%v", ga, ok)
	}
}

func TestCloseDropsEverythingAtomically(t *testing.T) {
	var r Registry
	s := r.Open("alpha", []string{"cap.a"}, []string{"onError"})
	r.Own(s.ID, OwnedItem{Kind: "hook", Name: "h1"})
	r.Own(s.ID, OwnedItem{Kind: "var", Name: "v1"})

	dropped, ok := r.Close(s.ID)
	if !ok {
		t.Fatal("Close of a live session refused")
	}
	// The dropped snapshot is the full record of what the session
	// owned -- declarations and registrations together.
	if len(dropped.Capabilities) != 1 || len(dropped.Hooks) != 1 || len(dropped.Owned) != 2 {
		t.Fatalf("dropped snapshot is not the whole session: %+v", dropped)
	}
	if _, ok := r.Get(s.ID); ok {
		t.Fatal("closed session still visible through Get")
	}
	if listed := r.List(); len(listed) != 0 {
		t.Fatalf("closed session still listed: %+v", listed)
	}
	if r.Own(s.ID, OwnedItem{Kind: "var", Name: "late"}) {
		t.Fatal("a registration against a dead session was accepted; teardown could never reach it")
	}
	if _, ok := r.Close(s.ID); ok {
		t.Fatal("closing a dead session claims it dropped something")
	}
}

func TestSnapshotsAreCopies(t *testing.T) {
	var r Registry
	s := r.Open("alpha", []string{"cap.a"}, nil)
	s.Capabilities[0] = "mutated"
	back, _ := r.Get(s.ID)
	if back.Capabilities[0] != "cap.a" {
		t.Fatalf("mutating a returned snapshot changed the registry: %+v", back)
	}
}

func TestConcurrentOpenOwnClose(t *testing.T) {
	var r Registry
	var wg sync.WaitGroup
	for i := 0; i < 32; i++ {
		wg.Add(1)
		go func() {
			defer wg.Done()
			s := r.Open("p", []string{"c"}, nil)
			r.Own(s.ID, OwnedItem{Kind: "hook", Name: "h"})
			r.List()
			if _, ok := r.Close(s.ID); !ok {
				t.Error("Close of a just-opened session refused")
			}
		}()
	}
	wg.Wait()
	if listed := r.List(); len(listed) != 0 {
		t.Fatalf("sessions leaked under concurrency: %+v", listed)
	}
}

func TestListEmptyIsEmptyNotNil(t *testing.T) {
	var r Registry
	if got := r.List(); got == nil || len(got) != 0 {
		t.Fatalf("empty registry must list an empty (non-nil) slice, got %+v", got)
	}
}
