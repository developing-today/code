// lane-scratch: code-e lane-e-20261110-scratch task 14746
package main

import (
	"testing"

	"github.com/charmbracelet/ssh"
)

// fakeSession is a distinct ssh.Session identity per instance. The
// embedded nil interface satisfies ssh.Session; the registry and the
// cleanup middleware use a session only as an identity key and pass it
// through, never calling its methods.
type fakeSession struct {
	ssh.Session
	id int
}

func registrySize(a *app) int {
	a.mu.RLock()
	defer a.mu.RUnlock()
	return len(a.progs)
}

// TestSessionCleanupDeregistersOnHandlerReturn covers the #4515 waiter
// lifecycle: the program stays registered while the session's handler
// chain runs, and the cleanup middleware removes it when the chain
// returns (after p.Run() completes) — no p.Wait() goroutine involved.
func TestSessionCleanupDeregistersOnHandlerReturn(t *testing.T) {
	a := &app{}
	p := &fakeProg{}
	s := &fakeSession{id: 1}
	a.registerSession(s, p)

	handler := a.sessionCleanupMiddleware(func(sess ssh.Session) {
		if n := registrySize(a); n != 1 {
			t.Fatalf("registry has %d programs while session is live, want 1", n)
		}
	})
	handler(s)

	if n := registrySize(a); n != 0 {
		t.Fatalf("registry has %d programs after session handler returned, want 0", n)
	}
}

// TestSessionCleanupLeavesOtherSessions covers the Codex finding on
// PR #4521: sessions sharing one SSH connection are independent —
// ending one session must deregister only its own program while a
// still-live session on the same connection keeps its registration.
func TestSessionCleanupLeavesOtherSessions(t *testing.T) {
	a := &app{}
	p1, p2 := &fakeProg{}, &fakeProg{}
	s1, s2 := &fakeSession{id: 1}, &fakeSession{id: 2}
	a.registerSession(s1, p1)
	a.registerSession(s2, p2)

	a.deregisterSession(s1)

	if n := registrySize(a); n != 1 {
		t.Fatalf("registry has %d programs after one session ended, want 1", n)
	}
	a.mu.RLock()
	remaining := a.progs[0]
	a.mu.RUnlock()
	if remaining != sender(p2) {
		t.Fatal("the surviving registration is not the live session's program")
	}

	// Deregistering an unknown or already-removed session is a no-op.
	a.deregisterSession(s1)
	a.deregisterSession(&fakeSession{id: 99})
	if n := registrySize(a); n != 1 {
		t.Fatalf("registry has %d programs after no-op deregistrations, want 1", n)
	}
}

// TestRemoveDropsSessionBinding ensures the queue-prune path (remove)
// cannot leave a stale session binding behind for a later
// deregisterSession to act on.
func TestRemoveDropsSessionBinding(t *testing.T) {
	a := &app{}
	p := &fakeProg{}
	s := &fakeSession{id: 1}
	a.registerSession(s, p)

	a.remove(p)

	a.mu.RLock()
	bindings := len(a.sessions)
	a.mu.RUnlock()
	if bindings != 0 {
		t.Fatalf("%d session bindings remain after remove, want 0", bindings)
	}
	a.deregisterSession(s)
	if n := registrySize(a); n != 0 {
		t.Fatalf("registry has %d programs, want 0", n)
	}
}
