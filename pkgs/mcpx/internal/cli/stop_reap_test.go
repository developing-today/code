package cli

import (
	"context"
	"errors"
	"net"
	"net/http"
	"os"
	"path/filepath"
	"sync/atomic"
	"syscall"
	"testing"
	"time"

	"github.com/developing-today/code/pkgs/mcpx/internal/mcpclient"
)

// `mcpx stop --all` reaps the stopped config's supervisors (#9963):
// supervisors outlive daemon restarts by design, so only the
// explicit stop terminates them — child killed, socket removed —
// while another config's supervisor in the same state directory is
// left exactly in place.
func TestStopAllReapsStoppedConfigsSupervisors(t *testing.T) {
	a, defaultDir := customStateApp(t)
	if err := os.MkdirAll(defaultDir, 0o755); err != nil {
		t.Fatal(err)
	}
	socket := filepath.Join(defaultDir, "daemon.sock")
	ln, err := net.Listen("unix", socket)
	if err != nil {
		t.Fatal(err)
	}
	var shutdowns atomic.Int32
	mux := http.NewServeMux()
	mux.HandleFunc("/v1/health", func(w http.ResponseWriter, r *http.Request) {
		_, _ = w.Write([]byte(`{"status":"ok"}`))
	})
	var srv *http.Server
	mux.HandleFunc("/v1/shutdown", func(w http.ResponseWriter, r *http.Request) {
		shutdowns.Add(1)
		_, _ = w.Write([]byte(`{}`))
		// Close the whole server, not just the listener: the client's
		// pooled health connection must break too, or waitUntilStopped
		// keeps pinging a live connection to a dead daemon.
		go func() {
			time.Sleep(50 * time.Millisecond)
			_ = srv.Close()
		}()
	})
	srv = &http.Server{Handler: mux}
	go func() { _ = srv.Serve(ln) }()
	defer func() { _ = srv.Close() }()

	cfgPath := filepath.Join(t.TempDir(), "mcpx.json")
	if err := os.WriteFile(cfgPath, []byte(`{"mcpServers": {"era": {"command": "cat"}}}`), 0o644); err != nil {
		t.Fatal(err)
	}
	writeDaemonRecord(t, defaultDir, socket, cfgPath)

	eraPID := plantSupervisor(t, defaultDir, "era")
	otherPID := plantSupervisor(t, defaultDir, "other")
	otherSock := mcpclient.SupervisorSocketPath(defaultDir, "other")

	if err := a.stopAll(context.Background()); err != nil {
		t.Fatalf("stopAll: %v", err)
	}
	if got := shutdowns.Load(); got != 1 {
		t.Fatalf("fake daemon received %d shutdowns, want 1", got)
	}

	// The stopped config's supervisor is reaped: child dead, socket gone.
	waitPIDGone(t, eraPID)
	if _, err := os.Stat(mcpclient.SupervisorSocketPath(defaultDir, "era")); !os.IsNotExist(err) {
		t.Fatalf("stopped config's supervisor socket still present: %v", err)
	}
	// The other config's supervisor is untouched: still listening,
	// child alive — terminating it now is the test's cleanup.
	if err := syscall.Kill(otherPID, 0); err != nil {
		t.Fatalf("other supervisor's child %d should still be alive: %v", otherPID, err)
	}
	if err := mcpclient.TerminateSupervisorSocket(otherSock); err != nil {
		t.Fatalf("other supervisor should still answer terminate: %v", err)
	}
	waitPIDGone(t, otherPID)
}

// plantSupervisor runs a supervisor for name in-process in stateDir
// and returns its child's pid, learned by attaching once.
func plantSupervisor(t *testing.T, stateDir, name string) int {
	t.Helper()
	sock := mcpclient.SupervisorSocketPath(stateDir, name)
	go func() {
		_ = mcpclient.RunSupervisor(mcpclient.SupervisorOptions{
			SocketPath: sock, Command: "cat", InheritEnv: true,
		})
	}()
	opts := mcpclient.SupervisedOptions{Name: name, StateDir: stateDir, Command: "cat", InheritEnv: true}
	deadline := time.Now().Add(15 * time.Second)
	for {
		tr, err := mcpclient.AttachSupervised(opts)
		if err == nil {
			pid := tr.PID()
			_ = tr.Close()
			if pid <= 0 {
				t.Fatalf("supervisor for %s reported child pid %d", name, pid)
			}
			return pid
		}
		if time.Now().After(deadline) {
			t.Fatalf("attach to planted supervisor %s: %v", name, err)
		}
		time.Sleep(20 * time.Millisecond)
	}
}

func waitPIDGone(t *testing.T, pid int) {
	t.Helper()
	deadline := time.Now().Add(10 * time.Second)
	for {
		err := syscall.Kill(pid, 0)
		if errors.Is(err, syscall.ESRCH) {
			return
		}
		if err != nil {
			t.Fatalf("check pid %d: %v", pid, err)
		}
		if time.Now().After(deadline) {
			t.Fatalf("pid %d still alive", pid)
		}
		time.Sleep(20 * time.Millisecond)
	}
}
