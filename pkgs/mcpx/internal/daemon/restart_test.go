package daemon

import (
	"encoding/json"
	"errors"
	"net/http"
	"net/http/httptest"
	"os"
	"syscall"
	"testing"
	"time"
)

// stubSignalSelf replaces the daemon's self-signal seam for one test and
// returns the channel that receives the signals the handlers asked for.
// The host process is never signalled: before #5262 the handlers signalled
// os.Getpid() directly, and the test binary died whenever a delayed
// SIGTERM landed while no signal.Notify interception was active.
func stubSignalSelf(t *testing.T) chan os.Signal {
	t.Helper()
	old := signalSelf
	ch := make(chan os.Signal, 1)
	signalSelf = func(sig os.Signal) error {
		ch <- sig
		return nil
	}
	t.Cleanup(func() { signalSelf = old })
	return ch
}

func TestHandleDaemonRestart(t *testing.T) {
	oldExec := execSelf
	execSelf = func(string, []string, []string) error { return errors.New("exec is disabled in tests") }
	t.Cleanup(func() { execSelf = oldExec })
	term := stubSignalSelf(t)

	s := &Server{}
	req := httptest.NewRequest(http.MethodPost, "/v1/daemon/restart", nil)
	w := httptest.NewRecorder()

	s.handleDaemonRestart(w, req)
	if w.Code != http.StatusOK {
		t.Fatalf("expected 200, got %d", w.Code)
	}

	var res map[string]any
	if err := json.Unmarshal(w.Body.Bytes(), &res); err != nil {
		t.Fatalf("failed to decode response: %v", err)
	}
	if res["status"] != "restarting" {
		t.Fatalf("expected status=restarting, got: %v", res["status"])
	}

	select {
	case sig := <-term:
		if sig != syscall.SIGTERM {
			t.Fatalf("expected the restart fallback to request SIGTERM, got %v", sig)
		}
	case <-time.After(5 * time.Second):
		t.Fatal("the restart handler never finished")
	}
}

func TestHandleShutdownSignalsSelfThroughSeam(t *testing.T) {
	term := stubSignalSelf(t)

	s := &Server{}
	req := httptest.NewRequest(http.MethodPost, "/v1/shutdown", nil)
	w := httptest.NewRecorder()

	s.handleShutdown(w, req)
	if w.Code != http.StatusOK {
		t.Fatalf("expected 200, got %d", w.Code)
	}

	var res map[string]any
	if err := json.Unmarshal(w.Body.Bytes(), &res); err != nil {
		t.Fatalf("failed to decode response: %v", err)
	}
	if res["status"] != "stopping" {
		t.Fatalf("expected status=stopping, got: %v", res["status"])
	}

	select {
	case sig := <-term:
		if sig != syscall.SIGTERM {
			t.Fatalf("expected the shutdown handler to request SIGTERM, got %v", sig)
		}
	case <-time.After(5 * time.Second):
		t.Fatal("the shutdown handler never signalled through the seam")
	}
	// Survival is the regression assertion (#5262): this test registers no
	// signal.Notify interception, so a real SIGTERM to this process would
	// have terminated the test binary before it could report.
}
