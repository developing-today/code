package mcpclient_test

import (
	"context"
	"fmt"
	"net"
	"os"
	"path/filepath"
	"strings"
	"syscall"
	"testing"
	"time"

	"github.com/developing-today/code/pkgs/mcpx/internal/mcpclient"
)

// Phase 3 of #1384 (issue #9963): restart through the supervisor,
// terminate (the reap mechanism), the adopt-or-reap scan, and the
// protocol-refusal classification status relies on.

func phase3Opts(dir, name string, child ...string) mcpclient.SupervisedOptions {
	return mcpclient.SupervisedOptions{
		Name:       name,
		StateDir:   dir,
		Command:    child[0],
		Args:       child[1:],
		InheritEnv: true,
	}
}

// waitAttachFree polls until the supervisor's attach slot is free
// (a Close clears it asynchronously) and leaves the transport
// detached again.
func waitAttachFree(t *testing.T, opts mcpclient.SupervisedOptions) {
	t.Helper()
	deadline := time.Now().Add(10 * time.Second)
	for {
		tr, err := mcpclient.AttachSupervised(opts)
		if err == nil {
			_ = tr.Close()
			time.Sleep(50 * time.Millisecond)
			return
		}
		if time.Now().After(deadline) {
			t.Fatalf("attach slot never freed: %v", err)
		}
		time.Sleep(10 * time.Millisecond)
	}
}

func pidAlive(pid int) bool {
	return pid > 0 && syscall.Kill(pid, 0) == nil
}

func waitPIDDead(t *testing.T, pid int) {
	t.Helper()
	deadline := time.Now().Add(10 * time.Second)
	for pidAlive(pid) {
		if time.Now().After(deadline) {
			t.Fatalf("pid %d still alive", pid)
		}
		time.Sleep(20 * time.Millisecond)
	}
}

// A restart dial has the supervisor replace its child: the new
// transport speaks to a new PID, and the supervisor (and its socket)
// survives — the daemon never respawns either.
func TestSupervisedRestartThroughSupervisor(t *testing.T) {
	dir := t.TempDir()
	opts := phase3Opts(dir, "echo", "cat")
	sock := mcpclient.SupervisorSocketPath(dir, "echo")

	tr1, err := mcpclient.NewSupervised(opts)
	if err != nil {
		t.Fatalf("attach 1: %v", err)
	}
	pid1 := tr1.PID()
	defer killChild(t, tr1)
	if err := tr1.Close(); err != nil {
		t.Fatalf("detach: %v", err)
	}
	waitAttachFree(t, opts)

	var tr2 *mcpclient.SupervisedTransport
	deadline := time.Now().Add(10 * time.Second)
	for {
		tr2, err = mcpclient.RestartSupervised(opts)
		if err == nil {
			break
		}
		if !strings.Contains(err.Error(), "already attached") || time.Now().After(deadline) {
			t.Fatalf("restart: %v", err)
		}
		time.Sleep(10 * time.Millisecond)
	}
	defer func() { _ = tr2.Close() }()
	defer killChild(t, tr2)
	if tr2.PID() == pid1 {
		t.Fatalf("restart kept the old child pid %d", pid1)
	}
	waitPIDDead(t, pid1)

	ctx := context.Background()
	frame := []byte(`{"jsonrpc":"2.0","id":1,"method":"ping"}`)
	if err := tr2.Send(ctx, frame); err != nil {
		t.Fatalf("send to restarted child: %v", err)
	}
	got, err := tr2.Recv()
	if err != nil || string(got) != string(frame) {
		t.Fatalf("recv from restarted child: %q, %v", got, err)
	}
	if _, err := os.Stat(sock); err != nil {
		t.Fatalf("supervisor socket did not survive the restart: %v", err)
	}
}

// Terminate is the reap: the supervisor kills its child and exits,
// removing its socket.
func TestSupervisedTerminateReapsChild(t *testing.T) {
	dir := t.TempDir()
	opts := phase3Opts(dir, "echo", "cat")
	sock := mcpclient.SupervisorSocketPath(dir, "echo")

	tr, err := mcpclient.NewSupervised(opts)
	if err != nil {
		t.Fatalf("attach: %v", err)
	}
	pid := tr.PID()
	defer killChild(t, tr)
	if err := tr.Close(); err != nil {
		t.Fatalf("detach: %v", err)
	}
	waitAttachFree(t, opts)

	if err := mcpclient.TerminateSupervisorSocket(sock); err != nil {
		t.Fatalf("terminate: %v", err)
	}
	waitPIDDead(t, pid)
	deadline := time.Now().Add(10 * time.Second)
	for {
		if _, err := os.Stat(sock); os.IsNotExist(err) {
			break
		}
		if time.Now().After(deadline) {
			t.Fatalf("supervisor socket %s not removed after terminate", sock)
		}
		time.Sleep(20 * time.Millisecond)
	}
}

// The adopt-or-reap scan, both branches plus the stale file: a
// configured supervisor is left in place and remains adoptable, an
// unconfigured one is reaped (child killed, reap logged), and a
// socket file with nobody listening is removed.
func TestScanSupervisorSocketsAdoptAndReap(t *testing.T) {
	dir := t.TempDir()
	keptOpts := phase3Opts(dir, "kept", "cat")
	orphanOpts := phase3Opts(dir, "orphan", "cat")

	keptTr, err := mcpclient.NewSupervised(keptOpts)
	if err != nil {
		t.Fatalf("start kept: %v", err)
	}
	defer killChild(t, keptTr)
	orphanTr, err := mcpclient.NewSupervised(orphanOpts)
	if err != nil {
		t.Fatalf("start orphan: %v", err)
	}
	orphanPID := orphanTr.PID()
	defer killChild(t, orphanTr)
	_ = keptTr.Close()
	_ = orphanTr.Close()
	waitAttachFree(t, keptOpts)
	waitAttachFree(t, orphanOpts)

	staleSock := mcpclient.SupervisorSocketPath(dir, "gone")
	if err := os.WriteFile(staleSock, []byte("not a socket"), 0o600); err != nil {
		t.Fatal(err)
	}

	var logs []string
	logf := func(format string, args ...any) {
		logs = append(logs, fmt.Sprintf(format, args...))
	}
	var outcomes []mcpclient.SupervisorScanOutcome
	deadline := time.Now().Add(15 * time.Second)
	for {
		outcomes = mcpclient.ScanSupervisorSockets(dir, []string{"kept"}, logf)
		byAction := map[string]string{}
		for _, o := range outcomes {
			byAction[o.Socket] = o.Action
		}
		if byAction[mcpclient.SupervisorSocketPath(dir, "orphan")] == "reaped" {
			break
		}
		if time.Now().After(deadline) {
			t.Fatalf("orphan never reaped; outcomes = %+v, logs = %v", outcomes, logs)
		}
		time.Sleep(50 * time.Millisecond)
	}
	byAction := map[string]string{}
	for _, o := range outcomes {
		byAction[o.Socket] = o.Action
	}
	if got := byAction[mcpclient.SupervisorSocketPath(dir, "kept")]; got != "configured" {
		t.Fatalf("kept supervisor action = %q, want configured (outcomes %+v)", got, outcomes)
	}
	if got := byAction[staleSock]; got != "stale-removed" {
		t.Fatalf("stale socket action = %q, want stale-removed (outcomes %+v)", got, outcomes)
	}
	if _, err := os.Stat(staleSock); !os.IsNotExist(err) {
		t.Fatalf("stale socket still present: %v", err)
	}
	waitPIDDead(t, orphanPID)
	joined := strings.Join(logs, "\n")
	if !strings.Contains(joined, "reaped orphaned supervisor") {
		t.Fatalf("reap not logged; logs = %v", logs)
	}

	// The adopt branch: the configured supervisor is still there to
	// be adopted by attach, with its original child.
	adopted, err := mcpclient.AttachSupervised(keptOpts)
	if err != nil {
		t.Fatalf("adopt kept supervisor after scan: %v", err)
	}
	if adopted.PID() != keptTr.PID() {
		t.Fatalf("adopted child pid = %d, want the kept child %d", adopted.PID(), keptTr.PID())
	}
	_ = adopted.Close()
	waitAttachFree(t, keptOpts)
	if err := mcpclient.TerminateSupervisorSocket(mcpclient.SupervisorSocketPath(dir, "kept")); err != nil {
		t.Fatalf("cleanup terminate kept: %v", err)
	}
}

// A supervisor that answers with a different protocol version
// refuses, and the refusal is classified so status can name it
// (#9963) instead of the failure looking like a dead socket.
func TestSupervisorProtocolRefusalIsClassified(t *testing.T) {
	dir := t.TempDir()
	sock := mcpclient.SupervisorSocketPath(dir, "future")
	if err := os.MkdirAll(filepath.Dir(sock), 0o700); err != nil {
		t.Fatal(err)
	}
	ln, err := net.Listen("unix", sock)
	if err != nil {
		t.Fatal(err)
	}
	defer func() { _ = ln.Close() }()
	go func() {
		conn, err := ln.Accept()
		if err != nil {
			return
		}
		defer func() { _ = conn.Close() }()
		buf := make([]byte, 4096)
		_ = conn.SetReadDeadline(time.Now().Add(5 * time.Second))
		if _, err := conn.Read(buf); err != nil {
			return
		}
		_, _ = conn.Write([]byte("{\"version\":999,\"pid\":4242}\n"))
	}()

	_, err = mcpclient.AttachSupervised(phase3Opts(dir, "future", "cat"))
	if err == nil {
		t.Fatal("attach to a version-999 supervisor succeeded")
	}
	if !mcpclient.IsSupervisorRefusal(err) {
		t.Fatalf("error is not a supervisor refusal: %v", err)
	}
	if !mcpclient.IsProtocolRefusal(err) {
		t.Fatalf("error is not classified as a protocol refusal: %v", err)
	}
	if !strings.Contains(err.Error(), "protocol version mismatch") {
		t.Fatalf("error does not name the version mismatch: %v", err)
	}
}
