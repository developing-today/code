package mcpclient

import (
	"context"
	"encoding/json"
	"errors"
	"strings"
	"sync"
	"testing"
	"time"
)

const echoScript = `while IFS= read -r l; do printf '%s\n' "$l"; done`

func TestDetachedChildKeepsItsPipesAndAdoptionResumesThem(t *testing.T) {
	first, err := NewStdio(StdioOptions{Command: "sh", Args: []string{"-c", echoScript}, InheritEnv: true})
	if err != nil {
		t.Fatal(err)
	}
	ctx := context.Background()
	if err := first.Send(ctx, []byte(`{"n":1}`)); err != nil {
		t.Fatal(err)
	}
	if got, err := first.Recv(); err != nil || string(got) != `{"n":1}` {
		t.Fatalf("before handoff: %q %v", got, err)
	}

	h, err := first.Detach()
	if err != nil {
		t.Fatal(err)
	}
	if _, err := first.Detach(); err == nil {
		t.Error("a second Detach of the same transport succeeded")
	}
	if err := first.Send(ctx, []byte(`{"n":2}`)); err == nil {
		t.Error("Send succeeded on a detached transport")
	}

	second, err := AdoptStdio(*h)
	if err != nil {
		t.Fatal(err)
	}
	if second.PID() != h.PID {
		t.Fatalf("adopted pid %d, want the child's %d", second.PID(), h.PID)
	}
	if err := second.Send(ctx, []byte(`{"n":3}`)); err != nil {
		t.Fatal(err)
	}
	if got, err := second.Recv(); err != nil || string(got) != `{"n":3}` {
		t.Fatalf("after adoption: %q %v", got, err)
	}
	if !childAlive(h.PID, h.StartTime) {
		t.Fatal("child not alive after adoption")
	}
	_ = second.Close()
	<-second.exited
}

func TestHandoffDoesNotSplitAFrameAcrossTheTwoProcesses(t *testing.T) {
	script := `printf '{"par'; sleep 1; printf 'tial":1}\n'; ` + echoScript
	first, err := NewStdio(StdioOptions{Command: "sh", Args: []string{"-c", script}, InheritEnv: true})
	if err != nil {
		t.Fatal(err)
	}
	got := make(chan []byte, 1)
	go func() {
		line, _ := first.Recv()
		got <- line
	}()
	time.Sleep(200 * time.Millisecond)
	h, err := first.Detach()
	if err != nil {
		t.Fatal(err)
	}
	if line := <-got; line != nil {
		t.Fatalf("a read interrupted mid-frame returned %q", line)
	}

	second, err := AdoptStdio(*h)
	if err != nil {
		t.Fatal(err)
	}
	line, err := second.Recv()
	if err != nil || string(line) != `{"partial":1}` {
		t.Fatalf("frame after handoff: %q %v", line, err)
	}
	_ = second.Close()
	<-second.exited
}

func TestAdoptRefusesAChildThatEnded(t *testing.T) {
	tr, err := NewStdio(StdioOptions{Command: "sh", Args: []string{"-c", "exit 0"}, InheritEnv: true})
	if err != nil {
		t.Fatal(err)
	}
	<-tr.exited
	_, err = AdoptStdio(StdioHandoff{PID: tr.PID(), StartTime: 0, Label: "gone"})
	if err == nil || !strings.Contains(err.Error(), "not running") {
		t.Fatalf("adopting an ended child: %v", err)
	}
}

func TestClientHandoffCommitsOnlyWhenActivated(t *testing.T) {
	const responder = `sed -u 's/.*"id":\([0-9]*\).*/{"jsonrpc":"2.0","id":\1,"result":{}}/'`
	tr, err := NewStdio(StdioOptions{Command: "sh", Args: []string{"-c", responder}, InheritEnv: true})
	if err != nil {
		t.Fatal(err)
	}
	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()
	ping := json.RawMessage(`{}`)

	first := Resume(tr, Session{Era: EraLegacy}, Options{})
	first.Activate()
	if _, err := first.Request(ctx, "ping", ping); err != nil {
		t.Fatalf("before handoff: %v", err)
	}

	h, err := first.Detach()
	if err != nil {
		t.Fatal(err)
	}
	first.Reattach()
	if _, err := first.Request(ctx, "ping", ping); err != nil {
		t.Fatalf("after a reattach that was not handed on: %v", err)
	}

	h, err = first.Detach()
	if err != nil {
		t.Fatal(err)
	}
	first.Release()
	adopted, err := AdoptStdio(*h.Stdio)
	if err != nil {
		t.Fatal(err)
	}
	second := Resume(adopted, h.Session, Options{})
	second.Activate()
	if _, err := second.Request(ctx, "ping", ping); err != nil {
		t.Fatalf("after adoption: %v", err)
	}
	_ = second.Close()
	<-adopted.exited
}

// TestRecvQueuedBehindDetachDoesNotConsumeSuccessorFrame pins the #2842
// exclusion: a Recv queued behind Detach must refuse instead of consuming a
// frame the successor owns.
func TestRecvQueuedBehindDetachDoesNotConsumeSuccessorFrame(t *testing.T) {
	const successorFrame = `{"n":"successor"}`
	script := `printf '{"n":"successor"}\n'; printf 'ready\n' >&2; ` + echoScript
	tr, err := NewStdio(StdioOptions{Command: "sh", Args: []string{"-c", script}, InheritEnv: true})
	if err != nil {
		t.Fatal(err)
	}
	transferred := false
	t.Cleanup(func() {
		if !transferred {
			_ = tr.Close()
		}
	})

	// Keep Detach inside its readMu critical section so the receiver below is
	// queued behind it. The real stderr drain owns its original channel; this
	// replacement only provides a deterministic gate for the test.
	readRelease := make(chan struct{})
	tr.stderrDone = readRelease
	var releaseOnce sync.Once
	release := func() { releaseOnce.Do(func() { close(readRelease) }) }
	defer release()

	readyDeadline := time.NewTimer(2 * time.Second)
	defer readyDeadline.Stop()
	readyPoll := time.NewTicker(time.Millisecond)
	defer readyPoll.Stop()
	for !strings.Contains(tr.Stderr(100), "ready") {
		select {
		case <-readyDeadline.C:
			t.Fatal("child did not write its successor frame")
		case <-readyPoll.C:
		}
	}

	detachDone := make(chan struct {
		handoff *StdioHandoff
		err     error
	}, 1)
	go func() {
		handoff, err := tr.Detach()
		detachDone <- struct {
			handoff *StdioHandoff
			err     error
		}{handoff, err}
	}()

	// Wait until Detach owns readMu and is held at readRelease.
	deadline := time.Now().Add(2 * time.Second)
	for {
		if tr.readMu.TryLock() {
			tr.readMu.Unlock()
			if time.Now().After(deadline) {
				t.Fatal("Detach did not acquire readMu")
			}
			time.Sleep(time.Millisecond)
			continue
		}
		break
	}

	recvStarted := make(chan struct{})
	recvDone := make(chan struct {
		line []byte
		err  error
	}, 1)
	go func() {
		close(recvStarted)
		line, err := tr.Recv()
		recvDone <- struct {
			line []byte
			err  error
		}{line, err}
	}()
	<-recvStarted
	select {
	case result := <-recvDone:
		t.Fatalf("Recv escaped Detach's read lock: line=%q err=%v", result.line, result.err)
	case <-time.After(20 * time.Millisecond):
	}

	release()
	var detached struct {
		handoff *StdioHandoff
		err     error
	}
	select {
	case detached = <-detachDone:
	case <-time.After(2 * time.Second):
		t.Fatal("Detach did not return after releasing its read lock")
	}
	if detached.err != nil {
		t.Fatal(detached.err)
	}
	select {
	case result := <-recvDone:
		if result.err == nil || !strings.Contains(result.err.Error(), "detached") {
			t.Fatalf("queued Recv consumed the successor's frame: line=%q err=%v", result.line, result.err)
		}
	case <-time.After(2 * time.Second):
		t.Fatal("queued Recv did not return after Detach")
	}

	successor, err := AdoptStdio(*detached.handoff)
	if err != nil {
		t.Fatal(err)
	}
	transferred = true
	t.Cleanup(func() {
		_ = successor.Close()
		<-successor.exited
	})
	type recvResult struct {
		line []byte
		err  error
	}
	successorRecv := make(chan recvResult, 1)
	go func() {
		line, err := successor.Recv()
		successorRecv <- recvResult{line, err}
	}()
	select {
	case result := <-successorRecv:
		if result.err != nil || string(result.line) != successorFrame {
			t.Fatalf("successor frame after handoff: line=%q err=%v", result.line, result.err)
		}
	case <-time.After(2 * time.Second):
		t.Fatal("successor could not read its frame")
	}
}

// listenFakeTransport is a Transport that also has the HTTP transport's
// standalone GET stream opener, so a test can see whether Activate
// reopened it for a resumed legacy session.
type listenFakeTransport struct {
	closed  chan struct{}
	mu      sync.Mutex
	listens int
}

func (f *listenFakeTransport) Send(context.Context, []byte) error { return nil }
func (f *listenFakeTransport) Recv() ([]byte, error) {
	<-f.closed
	return nil, errors.New("transport closed")
}
func (f *listenFakeTransport) Close() error {
	select {
	case <-f.closed:
	default:
		close(f.closed)
	}
	return nil
}
func (f *listenFakeTransport) Info() string { return "listen-fake" }
func (f *listenFakeTransport) listen() {
	f.mu.Lock()
	f.listens++
	f.mu.Unlock()
}
func (f *listenFakeTransport) listenCount() int {
	f.mu.Lock()
	defer f.mu.Unlock()
	return f.listens
}

// A resumed legacy session at 2025-03-26 or later must reopen the
// standalone GET stream, as finishLegacy does on a fresh initialize:
// server-initiated requests and list-change notifications arrive on it,
// and a cold resume that leaves it closed hangs them. The pre-stream
// revision (2024-11-05) has no such stream to reopen.
func TestActivateReopensTheLegacyListenStreamOnResume(t *testing.T) {
	for _, tc := range []struct {
		negotiated string
		want       int
	}{
		{"2025-03-26", 1},
		{"2025-06-18", 1},
		{"2025-11-25", 1},
		{"2024-11-05", 0},
	} {
		tr := &listenFakeTransport{closed: make(chan struct{})}
		c := Resume(tr, Session{Era: EraLegacy, Negotiated: tc.negotiated}, Options{})
		c.Activate()
		if got := tr.listenCount(); got != tc.want {
			t.Errorf("negotiated %s: Activate opened the legacy GET stream %d time(s), want %d", tc.negotiated, got, tc.want)
		}
		_ = tr.Close()
	}
}

// Resume must carry the URL-elicitation handler a fresh client gets
// from the same Options: without it, a resumed 2025-11-25 upstream's
// -32042 errors come back unanswered instead of opening the broker
// interaction (Codex review on PR 5455).
// Resume must carry the workspace root a fresh client gets from the
// same Options: without it, a cold-resumed client that later recovers
// from a session-expired 404 re-initializes with an empty root and the
// replacement session loses the instance's workspace (Codex review on
// PR 5455).
func TestResumePreservesTheWorkspaceRoot(t *testing.T) {
	tr := &listenFakeTransport{closed: make(chan struct{})}
	defer func() { _ = tr.Close() }()
	c := Resume(tr, Session{Era: EraLegacy, Negotiated: "2025-03-26"}, Options{Root: "/home/user/git/code"})
	if c.root != "/home/user/git/code" {
		t.Fatalf("Resume dropped Options.Root: root = %q", c.root)
	}
	uri, name, ok := c.workspaceRoot()
	if !ok || uri != "file:///home/user/git/code" || name != "code" {
		t.Fatalf("workspaceRoot = %q, %q, %v; want the resumed root rendered", uri, name, ok)
	}
}

func TestResumePreservesTheURLElicitationHandler(t *testing.T) {
	handler := func(context.Context, []URLElicitation) (bool, error) { return true, nil }
	tr := &listenFakeTransport{closed: make(chan struct{})}
	defer func() { _ = tr.Close() }()
	c := Resume(tr, Session{Era: EraLegacy, Negotiated: "2025-11-25"}, Options{OnURLElicitationRequired: handler})
	if c.onURLElicitationRequired == nil {
		t.Fatal("Resume dropped Options.OnURLElicitationRequired")
	}
}
