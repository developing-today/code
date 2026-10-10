package e2e_test

import (
	"bytes"
	"encoding/json"
	"fmt"
	"net"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"testing"
	"time"
)

const twoServers = `{
  "mcpServers": {
    "a": { "command": "FAKE", "mcpx": { "sharing": "shared", "scope": "global", "description": "unchanged" } },
    "b": { "command": "FAKE", "args": ["--gen", "1"], "mcpx": { "sharing": "shared", "scope": "global", "description": "changed" } }
  }
}`

// The changed server's args move it to a different pool; the unchanged one
// is byte-for-byte the same.
const twoServersChanged = `{
  "mcpServers": {
    "a": { "command": "FAKE", "mcpx": { "sharing": "shared", "scope": "global", "description": "unchanged" } },
    "b": { "command": "FAKE", "args": ["--gen", "2"], "mcpx": { "sharing": "shared", "scope": "global", "description": "changed" } }
  }
}`

type daemonDoc struct {
	PID     int `json:"pid"`
	Servers []struct {
		Namespace string `json:"namespace"`
		Instances []struct {
			PID int `json:"pid"`
		} `json:"instances"`
	} `json:"servers"`
}

func (d daemonDoc) childOf(namespace string) int {
	for _, s := range d.Servers {
		if s.Namespace == namespace && len(s.Instances) > 0 {
			return s.Instances[0].PID
		}
	}
	return 0
}

func (e *env) daemonState(t *testing.T) daemonDoc {
	t.Helper()
	var d daemonDoc
	if err := json.Unmarshal([]byte(jsonOf(t, e.run("--json", "status"))), &d); err != nil {
		t.Fatal(err)
	}
	return d
}

// successor is a `mcpx daemon --takeover` process the test started, with the
// systemd notification socket it reports to.
type successor struct {
	cmd    *exec.Cmd
	notify *net.UnixConn
	done   chan struct{}
}

// listenNotify opens a systemd notification socket and returns its path.
func listenNotify(t *testing.T) (string, *net.UnixConn) {
	t.Helper()
	dir, err := os.MkdirTemp("", "ntf")
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { os.RemoveAll(dir) })
	sock := filepath.Join(dir, "notify.sock")
	ln, err := net.ListenUnixgram("unixgram", &net.UnixAddr{Name: sock, Net: "unixgram"})
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { ln.Close() })
	return sock, ln
}

func startSuccessor(t *testing.T, e *env) *successor {
	t.Helper()
	sock, ln := listenNotify(t)

	var out bytes.Buffer
	cmd := exec.Command(e.mcpx, "daemon", "--takeover")
	cmd.Dir = e.dir
	cmd.Env = append(append([]string{}, e.envVars...), "NOTIFY_SOCKET="+sock)
	cmd.Stdout, cmd.Stderr = &out, &out
	if err := cmd.Start(); err != nil {
		t.Fatal(err)
	}
	s := &successor{cmd: cmd, notify: ln, done: make(chan struct{})}
	go func() { _ = cmd.Wait(); close(s.done) }()
	t.Cleanup(func() {
		select {
		case <-s.done:
			return
		default:
		}
		_ = cmd.Process.Signal(os.Interrupt)
		select {
		case <-s.done:
		case <-time.After(15 * time.Second):
			_ = cmd.Process.Kill()
			<-s.done
		}
		if t.Failed() {
			t.Logf("successor output:\n%s", out.String())
		}
	})
	return s
}

// awaitSuccessor waits until the daemon the socket reaches is the successor.
func (e *env) awaitSuccessor(t *testing.T, s *successor) daemonDoc {
	t.Helper()
	deadline := time.Now().Add(30 * time.Second)
	for {
		select {
		case <-s.done:
			t.Fatal("the successor exited before it served")
		default:
		}
		if out, err := e.try("--json", "status"); err == nil {
			var d daemonDoc
			if json.Unmarshal([]byte(jsonOf(t, out)), &d) == nil && d.PID == s.cmd.Process.Pid {
				return d
			}
		}
		if time.Now().After(deadline) {
			t.Fatal("the successor never took over")
		}
		time.Sleep(50 * time.Millisecond)
	}
}

// mainPIDReported reads the systemd notifications and checks for the
// successor's MAINPID with READY in the same datagram.
func (s *successor) mainPIDReported(t *testing.T) {
	t.Helper()
	awaitMainPID(t, s.notify, s.cmd.Process.Pid)
}

func awaitMainPID(t *testing.T, ln *net.UnixConn, pid int) {
	t.Helper()
	want := fmt.Sprintf("MAINPID=%d\nREADY=1", pid)
	if err := ln.SetReadDeadline(time.Now().Add(15 * time.Second)); err != nil {
		t.Fatal(err)
	}
	buf := make([]byte, 4096)
	for {
		n, _, err := ln.ReadFrom(buf)
		if err != nil {
			t.Fatalf("no %q on NOTIFY_SOCKET: %v", want, err)
		}
		if string(buf[:n]) == want {
			return
		}
	}
}

// exited reports whether a process is gone or a zombie; processExists alone
// counts a zombie as alive.
func exited(pid int) bool {
	b, err := os.ReadFile(fmt.Sprintf("/proc/%d/stat", pid))
	if err != nil {
		return true
	}
	i := strings.LastIndexByte(string(b), ')')
	return i+2 < len(b) && b[i+2] == 'Z'
}

func awaitExit(t *testing.T, pid int, what string) {
	t.Helper()
	deadline := time.Now().Add(20 * time.Second)
	for !exited(pid) {
		if time.Now().After(deadline) {
			t.Fatalf("%s (pid %d) is still running", what, pid)
		}
		time.Sleep(50 * time.Millisecond)
	}
}

func TestTakeoverKeepsTheChildAndTheListener(t *testing.T) {
	e := newEnv(t, oneServer)
	e.envVars = append(e.envVars, "MCPX_DAEMON_WATCH_CONFIG=false")
	e.run("call", "demo.echo", `{"message":"before"}`)
	before := e.daemonState(t)
	child := before.childOf("demo")
	if child == 0 {
		t.Fatalf("no live child before the takeover: %+v", before)
	}

	s := startSuccessor(t, e)
	after := e.awaitSuccessor(t, s)
	if got := after.childOf("demo"); got != child {
		t.Fatalf("the child was replaced by the takeover: pid %d -> %d", child, got)
	}
	if out := e.run("call", "demo.echo", `{"message":"after"}`); !strings.Contains(out, "after") {
		t.Fatalf("the taken-over daemon did not answer through the child:\n%s", out)
	}
	s.mainPIDReported(t)
	awaitExit(t, before.PID, "the previous daemon")
	if !processExists(child) {
		t.Fatalf("the child %d died with the previous daemon", child)
	}
}

func TestTakeoverRestartsAChangedServerAndKeepsTheRest(t *testing.T) {
	e := newEnv(t, twoServers)
	e.envVars = append(e.envVars, "MCPX_DAEMON_WATCH_CONFIG=false")
	e.run("call", "a.echo", `{"message":"a"}`)
	e.run("call", "b.echo", `{"message":"b"}`)
	before := e.daemonState(t)
	keep, change := before.childOf("a"), before.childOf("b")
	if keep == 0 || change == 0 {
		t.Fatalf("both children must be live before the takeover: %+v", before)
	}

	if err := os.WriteFile(filepath.Join(e.dir, ".mcpx.json"),
		[]byte(strings.ReplaceAll(twoServersChanged, "FAKE", e.fake)), 0o644); err != nil {
		t.Fatal(err)
	}
	s := startSuccessor(t, e)
	after := e.awaitSuccessor(t, s)
	if got := after.childOf("a"); got != keep {
		t.Fatalf("the unchanged server's child was replaced: pid %d -> %d", keep, got)
	}
	awaitExit(t, change, "the changed server's old child")
	e.run("call", "b.echo", `{"message":"b again"}`)
	if got := e.daemonState(t).childOf("b"); got == 0 || got == change {
		t.Fatalf("the changed server did not start a new child: old %d, now %d", change, got)
	}
	s.mainPIDReported(t)
}

func TestTakeoverWithNoDaemonToTakeOverFails(t *testing.T) {
	e := newEnv(t, oneServer)
	out, err := e.try("daemon", "--takeover")
	if err == nil {
		t.Fatalf("a takeover with no daemon to take over succeeded:\n%s", out)
	}
	if !strings.Contains(out, "no daemon to take over") {
		t.Fatalf("the failure does not say why:\n%s", out)
	}
}
