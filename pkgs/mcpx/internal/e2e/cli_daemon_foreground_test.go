package e2e_test

import (
	"bytes"
	"encoding/json"
	"fmt"
	"os"
	"os/exec"
	"strconv"
	"strings"
	"testing"
	"time"
)

// foregroundDaemon starts `mcpx daemon` in the foreground for e and waits
// until `status` reports the daemon it started as the one serving. The
// daemon is interrupted when the test ends if it is still running.
func foregroundDaemon(t *testing.T, e *env) (*exec.Cmd, chan struct{}, *bytes.Buffer) {
	t.Helper()
	var out bytes.Buffer
	cmd := exec.Command(e.mcpx, "daemon")
	cmd.Dir = e.dir
	cmd.Env = append([]string{}, e.envVars...)
	cmd.Stdout, cmd.Stderr = &out, &out
	if err := cmd.Start(); err != nil {
		t.Fatal(err)
	}
	done := make(chan struct{})
	go func() { _ = cmd.Wait(); close(done) }()
	t.Cleanup(func() {
		select {
		case <-done:
			return
		default:
		}
		_ = cmd.Process.Signal(os.Interrupt)
		select {
		case <-done:
		case <-time.After(15 * time.Second):
			_ = cmd.Process.Kill()
			<-done
		}
		if t.Failed() {
			t.Logf("daemon output:\n%s", out.String())
		}
	})
	deadline := time.Now().Add(30 * time.Second)
	for {
		select {
		case <-done:
			t.Fatalf("the foreground daemon exited before it served:\n%s", out.String())
		default:
		}
		// tryUnrecorded: status must not autostart a daemon of its own,
		// and it does not, but the poll is not the exercise under test.
		if st, err := e.tryUnrecorded("--json", "status"); err == nil {
			var doc daemonDoc
			if json.Unmarshal([]byte(jsonOf(t, st)), &doc) == nil && doc.PID == cmd.Process.Pid {
				return cmd, done, &out
			}
		}
		if time.Now().After(deadline) {
			t.Fatalf("the foreground daemon never served:\n%s", out.String())
		}
		time.Sleep(100 * time.Millisecond)
	}
}

// TestForegroundDaemonServesUntilInterrupted exercises `mcpx daemon`
// (#8461): run in the foreground it becomes the daemon the CLI talks to,
// serves the configured server, and exits cleanly on an interrupt.
func TestForegroundDaemonServesUntilInterrupted(t *testing.T) {
	e := newEnv(t, oneServer)
	cmd, done, out := foregroundDaemon(t, e)
	if ls := e.run("ls"); !strings.Contains(ls, "demo") {
		t.Fatalf("the foreground daemon should serve demo:\n%s", ls)
	}
	_ = cmd.Process.Signal(os.Interrupt)
	select {
	case <-done:
	case <-time.After(15 * time.Second):
		t.Fatalf("the foreground daemon did not exit on an interrupt:\n%s", out.String())
	}
	if code := cmd.ProcessState.ExitCode(); code != 0 {
		t.Fatalf("an interrupted foreground daemon should exit 0, exited %d:\n%s", code, out.String())
	}
	// The command ran and served: record it, since it was started outside
	// the harness's runner (it never returns while it serves).
	e.recordCLI([]string{"daemon"})
}

// TestStdioProxyCarriesHTTPToTheDaemon exercises `mcpx stdio-proxy`
// (#8461): an HTTP request written to its stdin reaches the daemon on this
// host, and the daemon's answer comes back on stdout.
func TestStdioProxyCarriesHTTPToTheDaemon(t *testing.T) {
	e := newEnv(t, oneServer)
	foregroundDaemon(t, e)
	out := e.runStdin("GET /v1/health HTTP/1.0\r\nHost: mcpx\r\n\r\n", "stdio-proxy")
	if !strings.HasPrefix(out, "HTTP/1.0 200") && !strings.HasPrefix(out, "HTTP/1.1 200") {
		t.Fatalf("the proxied health check should answer 200:\n%s", out)
	}
}

// parentOf reads a process's parent PID from /proc, or 0 when it is gone.
func parentOf(pid int) int {
	raw, err := os.ReadFile(fmt.Sprintf("/proc/%d/stat", pid))
	if err != nil {
		return 0
	}
	// The command name is parenthesised and may hold spaces; the fields
	// after its closing paren are state then ppid.
	s := string(raw)
	fields := strings.Fields(s[strings.LastIndexByte(s, ')')+1:])
	if len(fields) < 2 {
		return 0
	}
	ppid, _ := strconv.Atoi(fields[1])
	return ppid
}

// TestSuperviseKeepsTheChildAcrossADaemonStop exercises `mcpx supervise`
// (#8461), the re-exec target of the supervised transport: a server with
// mcpx.supervised runs under `mcpx supervise --socket <path> -- <command>`,
// and its child outlives the daemon process, so the next daemon answers
// through the same child. The daemon here is killed, not stopped:
// since the #9963 restore an explicit `mcpx stop` reaps the config's
// supervisors (that is the no-orphan guarantee), so what this test
// pins is survival across the daemon's death and restart — the
// boundary the supervisor exists for.
func TestSuperviseKeepsTheChildAcrossADaemonStop(t *testing.T) {
	if _, err := os.Stat("/proc/self/stat"); err != nil {
		t.Skip("needs /proc to see the supervisor process")
	}
	e := newEnv(t, oneSupervisedServer)
	e.run("call", "demo.echo", `{"message":"before"}`)
	before := e.daemonState(t)
	child := before.childOf("demo")
	if child == 0 {
		t.Fatalf("no live supervised child: %+v", before)
	}
	sup := parentOf(child)
	t.Cleanup(func() {
		for _, pid := range []int{child, sup} {
			if pid > 1 && processExists(pid) {
				if proc, err := os.FindProcess(pid); err == nil {
					_ = proc.Kill()
				}
			}
		}
	})
	cmdline, err := os.ReadFile(fmt.Sprintf("/proc/%d/cmdline", sup))
	if err != nil {
		t.Fatalf("the supervised child's parent %d is unreadable: %v", sup, err)
	}
	args := strings.Split(strings.TrimRight(string(cmdline), "\x00"), "\x00")
	if len(args) < 4 || args[1] != "supervise" || args[2] != "--socket" {
		t.Fatalf("the supervised child should run under `mcpx supervise --socket`, its parent runs %q", args)
	}
	if sup == before.PID {
		t.Fatal("the supervisor should be its own process, not the daemon")
	}

	daemonProc, err := os.FindProcess(before.PID)
	if err != nil {
		t.Fatalf("find the daemon process %d: %v", before.PID, err)
	}
	if err := daemonProc.Kill(); err != nil {
		t.Fatalf("kill the daemon process %d: %v", before.PID, err)
	}
	awaitExit(t, before.PID, "the daemon")
	if !processExists(child) || !processExists(sup) {
		t.Fatalf("the supervisor %d and its child %d should outlive the dead daemon", sup, child)
	}
	if out := e.run("call", "demo.echo", `{"message":"after"}`); !strings.Contains(out, "after") {
		t.Fatalf("the next daemon should answer through the supervised child:\n%s", out)
	}
	if got := e.daemonState(t).childOf("demo"); got != child {
		t.Fatalf("the next daemon should attach to the same child: pid %d -> %d", child, got)
	}
	// The supervisor is spawned by the daemon, never through the harness's
	// runner, so its exercise is recorded here once it has been seen working.
	e.recordCLI([]string{"supervise"})
}
