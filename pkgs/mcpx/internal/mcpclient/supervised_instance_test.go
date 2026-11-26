package mcpclient_test

import (
	"os"
	"testing"
	"time"

	"github.com/developing-today/code/pkgs/mcpx/internal/mcpclient"
)

// Per-instance supervisor socket keying (#9963, the fix for the
// defect that forced #11343's revert): the primary instance keeps
// the historical name-only socket, and every scoped instance gets a
// distinct socket of its own.

func TestSupervisorSocketPathForInstanceKeying(t *testing.T) {
	dir := t.TempDir()
	legacy := mcpclient.SupervisorSocketPath(dir, "era")
	for _, primary := range []string{"", "global"} {
		if got := mcpclient.SupervisorSocketPathForInstance(dir, "era", primary); got != legacy {
			t.Fatalf("instance %q socket = %s, want the legacy name-only socket %s", primary, got, legacy)
		}
	}
	a := mcpclient.SupervisorSocketPathForInstance(dir, "era", "cwd:/a")
	b := mcpclient.SupervisorSocketPathForInstance(dir, "era", "cwd:/b")
	if a == legacy || b == legacy {
		t.Fatalf("scoped instances must not use the primary socket: a=%s b=%s legacy=%s", a, b, legacy)
	}
	if a == b {
		t.Fatalf("distinct instances share socket %s", a)
	}
	if again := mcpclient.SupervisorSocketPathForInstance(dir, "era", "cwd:/a"); again != a {
		t.Fatalf("instance socket not stable: %s then %s", a, again)
	}
	other := mcpclient.SupervisorSocketPathForInstance(dir, "other", "cwd:/a")
	if other == a {
		t.Fatalf("distinct servers share socket %s", a)
	}
	if sockDir := mcpclient.SupervisorSocketDir(dir); a[:len(sockDir)] != sockDir {
		t.Fatalf("instance socket %s not in socket dir %s", a, sockDir)
	}
}

// Two instances of one server run at once, under distinct
// supervisors with distinct children — the second spawn is not
// refused by the first instance's supervisor — and a scoped instance
// survives a frontend restart: detach and re-attach reaches the same
// child.
func TestTwoInstancesSameServerDistinctSupervisors(t *testing.T) {
	dir := t.TempDir()
	optsA := phase3Opts(dir, "echo", "cat")
	optsA.Instance = "cwd:/a"
	optsB := phase3Opts(dir, "echo", "cat")
	optsB.Instance = "cwd:/b"

	trA, err := mcpclient.NewSupervised(optsA)
	if err != nil {
		t.Fatalf("spawn instance A: %v", err)
	}
	defer killChild(t, trA)
	trB, err := mcpclient.NewSupervised(optsB)
	if err != nil {
		t.Fatalf("spawn instance B (the #11343 refusal case): %v", err)
	}
	defer killChild(t, trB)
	if trA.PID() == trB.PID() {
		t.Fatalf("instances share child pid %d", trA.PID())
	}
	for _, opts := range []mcpclient.SupervisedOptions{optsA, optsB} {
		sock := mcpclient.SupervisorSocketPathForInstance(dir, opts.Name, opts.Instance)
		if _, err := os.Stat(sock); err != nil {
			t.Fatalf("no supervisor socket at %s: %v", sock, err)
		}
	}

	// Frontend restart for instance A: detach, re-attach, same child.
	pidA := trA.PID()
	if err := trA.Close(); err != nil {
		t.Fatalf("detach A: %v", err)
	}
	waitAttachFree(t, optsA)
	reattached, err := mcpclient.AttachSupervised(optsA)
	if err != nil {
		t.Fatalf("re-attach A after frontend restart: %v", err)
	}
	if reattached.PID() != pidA {
		t.Fatalf("re-attached child pid = %d, want the surviving child %d", reattached.PID(), pidA)
	}
	// Instance B is untouched by A's restart.
	if !pidAlive(trB.PID()) {
		t.Fatalf("instance B child %d died during A's restart", trB.PID())
	}
	_ = reattached.Close()
	_ = trB.Close()
	waitAttachFree(t, optsB)
	waitAttachFree(t, optsA)
	for _, opts := range []mcpclient.SupervisedOptions{optsA, optsB} {
		sock := mcpclient.SupervisorSocketPathForInstance(dir, opts.Name, opts.Instance)
		if err := mcpclient.TerminateSupervisorSocket(sock); err != nil {
			t.Fatalf("cleanup terminate %s: %v", sock, err)
		}
	}
}

// The adopt-or-reap scan classifies by server, not by exact socket:
// a configured server's scoped instance socket is "configured" (left
// in place for adoption), and ReapSupervisors — the `mcpx stop`
// mechanism — terminates every instance of a named server and no
// other server's.
func TestScanAndReapAcrossInstances(t *testing.T) {
	dir := t.TempDir()
	keptOpts := phase3Opts(dir, "kept", "cat")
	keptOpts.Instance = "cwd:/x"
	otherOpts := phase3Opts(dir, "other", "cat")

	keptTr, err := mcpclient.NewSupervised(keptOpts)
	if err != nil {
		t.Fatalf("start kept: %v", err)
	}
	defer killChild(t, keptTr)
	otherTr, err := mcpclient.NewSupervised(otherOpts)
	if err != nil {
		t.Fatalf("start other: %v", err)
	}
	defer killChild(t, otherTr)
	keptPID, otherPID := keptTr.PID(), otherTr.PID()
	_ = keptTr.Close()
	_ = otherTr.Close()
	waitAttachFree(t, keptOpts)
	waitAttachFree(t, otherOpts)

	keptSock := mcpclient.SupervisorSocketPathForInstance(dir, "kept", "cwd:/x")
	outcomes := mcpclient.ScanSupervisorSockets(dir, []string{"kept"}, nil)
	bySocket := map[string]string{}
	for _, o := range outcomes {
		bySocket[o.Socket] = o.Action
	}
	if got := bySocket[keptSock]; got != "configured" {
		t.Fatalf("scoped socket of a configured server = %q, want configured (outcomes %+v)", got, outcomes)
	}
	if got := bySocket[mcpclient.SupervisorSocketPath(dir, "other")]; got != "reaped" {
		t.Fatalf("unconfigured server in scan = %q, want reaped (outcomes %+v)", got, outcomes)
	}
	waitPIDDead(t, otherPID)

	// Reap of a different name leaves the kept supervisor alone.
	if got := mcpclient.ReapSupervisors(dir, []string{"other"}, nil); len(got) != 0 {
		t.Fatalf("reap of another server reported outcomes: %+v", got)
	}
	if !pidAlive(keptPID) {
		t.Fatalf("kept child %d died in another server's reap", keptPID)
	}

	// Reap of the server's own name terminates its scoped instance.
	got := mcpclient.ReapSupervisors(dir, []string{"kept"}, nil)
	if len(got) != 1 || got[0].Socket != keptSock || got[0].Action != "reaped" {
		t.Fatalf("reap outcomes = %+v, want the scoped socket reaped", got)
	}
	waitPIDDead(t, keptPID)
	deadline := time.Now().Add(10 * time.Second)
	for {
		if _, err := os.Stat(keptSock); os.IsNotExist(err) {
			break
		}
		if time.Now().After(deadline) {
			t.Fatalf("scoped socket %s not removed after reap", keptSock)
		}
		time.Sleep(20 * time.Millisecond)
	}
}
