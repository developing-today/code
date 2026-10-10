package daemon

import (
	"bytes"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"net"
	"net/http"
	"os"
	"os/exec"
	"path/filepath"
	"sync"
)

const upgradeRoute = "/v1/upgrade"

// An upgrade starts the new binary as this daemon's child. The child inherits
// the environment, including NOTIFY_SOCKET, so it is still in the unit's
// cgroup and may take over MAINPID when this process exits. The request is
// hijacked like a takeover is, because the handover drains this daemon's HTTP
// server, and a handler still waiting on it would hold that drain open.

type upgradeRun struct {
	conn net.Conn
	once sync.Once
	done chan struct{}
}

// settle answers the upgrade once. The handover calls it before the old
// daemon waits for the successor to hang up, so the answer is on the wire
// before this process can exit.
func (u *upgradeRun) settle(err error) {
	u.once.Do(func() {
		defer close(u.done)
		code, body := http.StatusOK, map[string]any{"status": "upgraded"}
		if err != nil {
			code, body = http.StatusInternalServerError, map[string]any{"error": err.Error()}
		}
		b, _ := json.Marshal(body)
		resp := &http.Response{
			StatusCode:    code,
			ProtoMajor:    1,
			ProtoMinor:    1,
			Header:        http.Header{"Content-Type": {"application/json"}},
			ContentLength: int64(len(b)),
			Body:          io.NopCloser(bytes.NewReader(b)),
			Close:         true,
		}
		_ = resp.Write(u.conn)
	})
}

func (s *Server) settleUpgrade(err error) {
	if u := s.upgrading.Load(); u != nil {
		u.settle(err)
	}
}

func (s *Server) handleUpgrade(w http.ResponseWriter, r *http.Request) {
	var req struct {
		Binary string `json:"binary"`
	}
	body, err := io.ReadAll(r.Body)
	if err != nil || json.Unmarshal(body, &req) != nil || req.Binary == "" {
		http.Error(w, "an upgrade needs the path of the new binary", http.StatusBadRequest)
		return
	}
	self, err := os.Executable()
	if err != nil {
		http.Error(w, err.Error(), http.StatusInternalServerError)
		return
	}
	if samePath(self, req.Binary) {
		writeJSON(w, http.StatusOK, map[string]any{"status": "current"})
		return
	}
	if !s.upgradeMu.TryLock() {
		http.Error(w, "an upgrade is already under way", http.StatusConflict)
		return
	}
	defer s.upgradeMu.Unlock()
	conn, _, err := http.NewResponseController(w).Hijack()
	if err != nil {
		http.Error(w, err.Error(), http.StatusInternalServerError)
		return
	}
	defer conn.Close()

	u := &upgradeRun{conn: conn, done: make(chan struct{})}
	// Registered before the successor starts, since it can dial back before
	// Start returns.
	s.upgrading.Store(u)
	defer s.upgrading.Store(nil)
	s.logger.Printf("upgrade requested: starting %s as a child", req.Binary)
	cmd, err := s.startSuccessor(req.Binary)
	if err != nil {
		u.settle(err)
		return
	}
	exited := make(chan error, 1)
	go func() { exited <- cmd.Wait() }()
	select {
	case <-u.done:
	case err := <-exited:
		// A handover that was under way holds takeoverMu until it has put the
		// children back, so the failure is reported after that.
		s.takeoverMu.Lock()
		s.takeoverMu.Unlock()
		msg := "the successor exited before it took over"
		if err != nil {
			msg += ": " + err.Error()
		}
		u.settle(errors.New(msg))
	}
}

// startSuccessor runs the new binary as this daemon's child. The environment
// is inherited because the successor needs NOTIFY_SOCKET.
func (s *Server) startSuccessor(binary string) (*exec.Cmd, error) {
	args := []string{"daemon", "--takeover"}
	if s.ConfigArg != "" {
		args = append(args, "--config", s.ConfigArg)
	}
	cmd := exec.Command(binary, args...)
	cmd.Stdout, cmd.Stderr = os.Stdout, os.Stderr
	if err := cmd.Start(); err != nil {
		return nil, fmt.Errorf("start %s: %w", binary, err)
	}
	return cmd, nil
}

func samePath(a, b string) bool {
	ra, errA := filepath.EvalSymlinks(a)
	rb, errB := filepath.EvalSymlinks(b)
	if errA != nil || errB != nil {
		return filepath.Clean(a) == filepath.Clean(b)
	}
	return ra == rb
}
