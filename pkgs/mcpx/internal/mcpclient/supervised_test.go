package mcpclient_test

import (
	"bufio"
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"net"
	"os"
	"strings"
	"syscall"
	"testing"
	"time"

	"github.com/dezren39/mcpx/internal/mcpclient"
)

// TestMain doubles as the supervisor re-exec target and as scripted
// children, the same way the real binary is re-execed by the
// supervised transport: `mcpx supervise --socket <path> -- <child>`.
// Modes:
//
//	supervise  --socket <path> -- <command> [args...]  (supervisor)
//	noisychild                                        (flood, then echo)
//	exitchild                                         (one frame, exit 3)
//	floodchild                                        (flood > any socket buffer, then exit)
//	burstchild                                        (numbered burst, then echo)
func TestMain(m *testing.M) {
	if len(os.Args) > 1 {
		switch os.Args[1] {
		case "supervise":
			os.Exit(runSuperviseMode(os.Args[2:]))
		case "noisychild":
			runNoiseChild()
			os.Exit(0)
		case "exitchild":
			fmt.Println(`{"ready":true}`)
			os.Exit(3)
		case "floodchild":
			runFloodChild()
			os.Exit(0)
		case "burstchild":
			runBurstChild()
			os.Exit(0)
		}
	}
	os.Exit(m.Run())
}

// runFloodChild writes far more than any socket buffer holds
// (~5 MiB of ~100-byte frames), then exits.
func runFloodChild() {
	for i := 0; i < 50000; i++ {
		fmt.Printf("{\"n\":%06d}%s\n", i, strings.Repeat(" ", 88))
	}
}

// runBurstChild writes a small numbered burst, then echoes stdin.
func runBurstChild() {
	for i := 0; i < 100; i++ {
		fmt.Printf("{\"burst\":%06d}%s\n", i, strings.Repeat(" ", 84))
	}
	_, _ = io.Copy(os.Stdout, os.Stdin)
}

func runSuperviseMode(args []string) int {
	var socket string
	var child []string
	for i := 0; i < len(args); i++ {
		switch {
		case args[i] == "--socket" && i+1 < len(args):
			socket = args[i+1]
			i++
		case args[i] == "--":
			child = args[i+1:]
			i = len(args)
		default:
			fmt.Fprintln(os.Stderr, "supervise mode: unexpected argument", args[i])
			return 2
		}
	}
	if socket == "" || len(child) == 0 {
		fmt.Fprintln(os.Stderr, "supervise mode: want --socket <path> -- <command>")
		return 2
	}
	if err := mcpclient.RunSupervisor(mcpclient.SupervisorOptions{
		SocketPath: socket,
		Command:    child[0],
		Args:       child[1:],
		InheritEnv: true,
	}); err != nil {
		fmt.Fprintln(os.Stderr, "supervise mode:", err)
		return 1
	}
	return 0
}

// runNoiseChild writes far more than the unattached buffer holds, then
// echoes stdin like cat. Lines are ~100-byte JSON-ish frames.
func runNoiseChild() {
	for i := 0; i < 2000; i++ {
		fmt.Printf("{\"n\":%06d}%s\n", i, strings.Repeat(" ", 88))
	}
	_, _ = io.Copy(os.Stdout, os.Stdin)
}

func selfExe(t *testing.T) string {
	t.Helper()
	exe, err := os.Executable()
	if err != nil {
		t.Fatal(err)
	}
	return exe
}

func supervisedOpts(t *testing.T, name string, child ...string) mcpclient.SupervisedOptions {
	t.Helper()
	return mcpclient.SupervisedOptions{
		Name:       name,
		StateDir:   t.TempDir(),
		Command:    child[0],
		Args:       child[1:],
		InheritEnv: true,
	}
}

func killChild(t *testing.T, tr *mcpclient.SupervisedTransport) {
	t.Helper()
	if pid := tr.PID(); pid > 0 {
		_ = syscall.Kill(pid, syscall.SIGKILL)
	}
}

// The pump moves frames both ways, and a detach leaves the child
// running for the next attach — the defining property of #1385,
// exercised at transport level (the daemon here is this process; its
// transports detach where a SIGKILLed daemon's sockets would simply
// vanish, which the supervisor sees the same way).
func TestSupervisedPumpDetachReattach(t *testing.T) {
	opts := supervisedOpts(t, "echo", "cat")

	tr1, err := mcpclient.NewSupervised(opts)
	if err != nil {
		t.Fatalf("attach 1: %v", err)
	}
	pid := tr1.PID()
	if pid <= 0 {
		t.Fatalf("pid = %d", pid)
	}
	defer killChild(t, tr1)

	ctx := context.Background()
	frame := []byte(`{"jsonrpc":"2.0","id":1,"method":"ping"}`)
	if err := tr1.Send(ctx, frame); err != nil {
		t.Fatalf("send: %v", err)
	}
	got, err := tr1.Recv()
	if err != nil {
		t.Fatalf("recv: %v", err)
	}
	if string(got) != string(frame) {
		t.Fatalf("echo = %s, want %s", got, frame)
	}

	// Detach: Close signals nothing; the child survives.
	if err := tr1.Close(); err != nil {
		t.Fatalf("close: %v", err)
	}
	// Close returns once the local connection is closed; the supervisor
	// clears its attach slot when it notices the disconnect, which is
	// asynchronous. Poll the reattach until the slot frees instead of
	// assuming the release is instant (the #2609 flake: under load the
	// immediate reattach was refused "already attached").
	var tr2 *mcpclient.SupervisedTransport
	deadline := time.Now().Add(10 * time.Second)
	for {
		tr2, err = mcpclient.NewSupervised(opts)
		if err == nil {
			break
		}
		if !strings.Contains(err.Error(), "already attached") || time.Now().After(deadline) {
			t.Fatalf("attach 2: %v", err)
		}
		time.Sleep(10 * time.Millisecond)
	}
	defer tr2.Close()
	if tr2.PID() != pid {
		t.Fatalf("child pid changed across detach: %d -> %d", pid, tr2.PID())
	}
	if err := tr2.Send(ctx, frame); err != nil {
		t.Fatalf("send after reattach: %v", err)
	}
	got, err = tr2.Recv()
	if err != nil || string(got) != string(frame) {
		t.Fatalf("recv after reattach: %q, %v", got, err)
	}
}

// A second attach while a daemon is attached is refused in the
// handshake, and the refusal neither disturbs the first attach nor is
// mistaken for a dead socket worth respawning over.
func TestSupervisedSingleAttachRefused(t *testing.T) {
	opts := supervisedOpts(t, "echo", "cat")
	tr1, err := mcpclient.NewSupervised(opts)
	if err != nil {
		t.Fatalf("attach 1: %v", err)
	}
	defer tr1.Close()
	defer killChild(t, tr1)

	if _, err := mcpclient.NewSupervised(opts); err == nil ||
		!strings.Contains(err.Error(), "already attached") {
		t.Fatalf("second attach err = %v, want already attached refusal", err)
	}

	// The first attach still works.
	ctx := context.Background()
	frame := []byte(`{"jsonrpc":"2.0","id":2,"method":"ping"}`)
	if err := tr1.Send(ctx, frame); err != nil {
		t.Fatalf("send: %v", err)
	}
	if _, err := tr1.Recv(); err != nil {
		t.Fatalf("recv: %v", err)
	}
}

// Output written while no daemon is attached is buffered up to the
// bound: the tail is kept, the stream resumes on a frame boundary, and
// the dropped-byte count is reported at attach.
func TestSupervisedBufferOverflow(t *testing.T) {
	dir := t.TempDir()
	sock := mcpclient.SupervisorSocketPath(dir, "noise")
	done := make(chan error, 1)
	go func() {
		done <- mcpclient.RunSupervisor(mcpclient.SupervisorOptions{
			SocketPath: sock,
			Command:    selfExe(t),
			Args:       []string{"noisychild"},
			InheritEnv: true,
			BufferSize: 8192,
		})
	}()
	// Give the child time to flood the buffer while unattached.
	time.Sleep(2 * time.Second)

	tr, err := mcpclient.NewSupervised(mcpclient.SupervisedOptions{
		Name: "noise", StateDir: dir, Command: selfExe(t),
		Args: []string{"noisychild"}, InheritEnv: true,
	})
	if err != nil {
		t.Fatalf("attach: %v", err)
	}
	defer tr.Close()
	defer killChild(t, tr)
	if tr.DroppedBytes() <= 0 {
		t.Fatalf("droppedBytes = %d, want > 0 after overflow", tr.DroppedBytes())
	}
	// The first frame off the buffer is whole: Recv trims the padding,
	// so a frame-boundary start shows as one complete short frame —
	// a mid-line start would show as a fragment without the prefix.
	line, err := tr.Recv()
	if err != nil {
		t.Fatalf("recv buffered: %v", err)
	}
	if !strings.HasPrefix(string(line), `{"n":`) || !strings.HasSuffix(string(line), "}") {
		t.Fatalf("first buffered frame is not a whole frame boundary: %q", line)
	}
	// After the flood drains, the echo path works.
	deadline := time.Now().Add(10 * time.Second)
	var lastErr error
	for time.Now().Before(deadline) {
		frame := []byte(`{"jsonrpc":"2.0","id":9,"method":"ping"}`)
		if err := tr.Send(context.Background(), frame); err != nil {
			lastErr = err
			break
		}
		got, err := tr.Recv()
		if err != nil {
			lastErr = err
			break
		}
		if string(got) == string(frame) {
			lastErr = nil
			break
		}
	}
	if lastErr != nil {
		t.Fatalf("echo after drain: %v", lastErr)
	}
	killChild(t, tr)
	select {
	case <-done:
	case <-time.After(10 * time.Second):
		t.Fatal("supervisor did not exit after child exit")
	}
}

// A supervised child's exit surfaces to the attached daemon as a Recv
// error, and the supervisor exits afterwards.
func TestSupervisedChildExitSurfaced(t *testing.T) {
	dir := t.TempDir()
	opts := mcpclient.SupervisedOptions{
		Name: "exit", StateDir: dir, Command: selfExe(t),
		Args: []string{"exitchild"}, InheritEnv: true,
	}
	tr, err := mcpclient.NewSupervised(opts)
	if err != nil {
		t.Fatalf("attach: %v", err)
	}
	defer tr.Close()
	line, err := tr.Recv()
	if err != nil || string(line) != `{"ready":true}` {
		t.Fatalf("first frame: %q, %v", line, err)
	}
	if _, err := tr.Recv(); err == nil || !strings.Contains(err.Error(), "exited") {
		t.Fatalf("recv after child exit = %v, want an exited error", err)
	}
	// The supervisor exits once the child is gone: its socket stops
	// answering (a fresh attach would spawn a new supervisor, so probe
	// the socket file's listener by path existence + dial refusal via
	// a short-lived raw check is racy; instead assert the supervisor
	// process is gone by checking the child pid is dead).
	deadline := time.Now().Add(5 * time.Second)
	for time.Now().Before(deadline) {
		if err := syscall.Kill(tr.PID(), 0); err != nil {
			return // child reaped, as expected
		}
		time.Sleep(50 * time.Millisecond)
	}
	t.Fatal("child pid still alive after exit")
}

// rawAttach dials the supervisor socket and sends the attach
// handshake, retrying briefly while the listener comes up. It does
// not read the reply.
func rawAttach(t *testing.T, sock string) net.Conn {
	t.Helper()
	deadline := time.Now().Add(10 * time.Second)
	for {
		conn, err := net.DialTimeout("unix", sock, 2*time.Second)
		if err == nil {
			if _, err := conn.Write([]byte("{\"version\":1}\n")); err != nil {
				conn.Close()
				t.Fatalf("write handshake: %v", err)
			}
			return conn
		}
		if time.Now().After(deadline) {
			t.Fatalf("dial supervisor socket: %v", err)
		}
		time.Sleep(50 * time.Millisecond)
	}
}

// Regression for #2390: a daemon that attaches and never reads must
// not wedge the supervisor. Before the fix, pumpStdout wrote to the
// attached connection while holding s.mu, so once the socket buffer
// filled, the write blocked forever, the child exit was never
// recorded, and RunSupervisor never returned. Now the write carries a
// bounded deadline, the stalled attach is dropped, the pump drains
// the child to EOF, and the supervisor finishes.
func TestSupervisedStalledReaderAttachDropped(t *testing.T) {
	old := mcpclient.AttachedWriteTimeout
	mcpclient.AttachedWriteTimeout = 250 * time.Millisecond
	defer func() { mcpclient.AttachedWriteTimeout = old }()

	dir := t.TempDir()
	sock := mcpclient.SupervisorSocketPath(dir, "flood")
	done := make(chan error, 1)
	go func() {
		done <- mcpclient.RunSupervisor(mcpclient.SupervisorOptions{
			SocketPath: sock,
			Command:    selfExe(t),
			Args:       []string{"floodchild"},
			InheritEnv: true,
		})
	}()

	// Attach and never read: the stalled daemon.
	conn := rawAttach(t, sock)
	defer conn.Close()

	select {
	case err := <-done:
		if err != nil {
			t.Fatalf("RunSupervisor: %v", err)
		}
	case <-time.After(30 * time.Second):
		t.Fatal("supervisor did not finish: a stalled daemon wedged the child exit (#2390)")
	}

	// The stalled attach was dropped: the supervisor closed the
	// connection, so reads drain what was buffered and then hit EOF
	// rather than timing out on a live connection.
	_ = conn.SetReadDeadline(time.Now().Add(10 * time.Second))
	buf := make([]byte, 32<<10)
	for {
		_, err := conn.Read(buf)
		if err != nil {
			var nerr net.Error
			if errors.As(err, &nerr) && nerr.Timeout() {
				t.Fatal("stalled connection still open after supervisor finished: attach was not dropped")
			}
			break
		}
	}
}

// The attach flush keeps its ordering guarantee without holding s.mu
// across the writes (#2390): the daemon receives the handshake reply
// first, then the output buffered while unattached, then live output
// — no live chunk overtakes the flush.
func TestSupervisedAttachFlushOrdering(t *testing.T) {
	dir := t.TempDir()
	sock := mcpclient.SupervisorSocketPath(dir, "burst")
	done := make(chan error, 1)
	go func() {
		done <- mcpclient.RunSupervisor(mcpclient.SupervisorOptions{
			SocketPath: sock,
			Command:    selfExe(t),
			Args:       []string{"burstchild"},
			InheritEnv: true,
		})
	}()
	// Let the burst land in the unattached tail buffer.
	time.Sleep(1 * time.Second)

	conn := rawAttach(t, sock)
	defer conn.Close()
	_ = conn.SetReadDeadline(time.Now().Add(15 * time.Second))
	r := bufio.NewReader(conn)

	// 1. The handshake reply comes first.
	line, err := r.ReadBytes('\n')
	if err != nil {
		t.Fatalf("read reply: %v", err)
	}
	var reply struct {
		Version int `json:"version"`
		PID     int `json:"pid"`
	}
	if err := json.Unmarshal(line, &reply); err != nil {
		t.Fatalf("reply is not the handshake reply: %q: %v", line, err)
	}
	if reply.Version != mcpclient.SupervisorProtocolVersion || reply.PID <= 0 {
		t.Fatalf("reply = %+v, want version %d and a child pid", reply, mcpclient.SupervisorProtocolVersion)
	}
	defer func() { _ = syscall.Kill(reply.PID, syscall.SIGKILL) }()

	// 2. Then the buffered burst, complete and in order.
	for i := 0; i < 100; i++ {
		line, err := r.ReadBytes('\n')
		if err != nil {
			t.Fatalf("read buffered frame %d: %v", i, err)
		}
		want := fmt.Sprintf("{\"burst\":%06d}", i)
		if !strings.HasPrefix(string(line), want) {
			t.Fatalf("buffered frame %d = %q, want prefix %q (flush ordering broken)", i, line, want)
		}
	}

	// 3. Then live output: a frame sent now echoes back only after
	// everything buffered.
	if _, err := conn.Write([]byte("{\"live\":true}\n")); err != nil {
		t.Fatalf("write live frame: %v", err)
	}
	line, err = r.ReadBytes('\n')
	if err != nil {
		t.Fatalf("read live echo: %v", err)
	}
	if string(line) != "{\"live\":true}\n" {
		t.Fatalf("live frame = %q, want the echo after the buffered burst", line)
	}

	conn.Close()
	_ = syscall.Kill(reply.PID, syscall.SIGKILL)
	select {
	case <-done:
	case <-time.After(10 * time.Second):
		t.Fatal("supervisor did not exit after child exit")
	}
}
