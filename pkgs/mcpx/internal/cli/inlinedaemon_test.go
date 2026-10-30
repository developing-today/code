package cli

import (
	"context"
	"errors"
	"io"
	"net/http"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"github.com/dezren39/mcpx/internal/daemon"
	"github.com/dezren39/mcpx/internal/testsupport"
)

// inlineDaemonApp builds an App whose configuration declares no servers,
// so its in-process daemon has nothing to spawn and should answer at once.
func inlineDaemonApp(t *testing.T) *App {
	t.Helper()
	root := testsupport.TempDir(t)
	t.Setenv("HOME", root)
	t.Setenv("XDG_CONFIG_HOME", filepath.Join(root, "xdg"))
	cfg := filepath.Join(root, "mcpx.json")
	if err := os.WriteFile(cfg, []byte(`{"mcpServers":{}}`), 0o600); err != nil {
		t.Fatal(err)
	}
	return &App{ConfigPath: cfg, Paths: daemon.PathsAt(root, "")}
}

// A context that is already cancelled must not start an in-process daemon
// and then wait out the full inline start timeout (5s by default): Ping is
// ctx-bound, so it could never succeed, and the old code reported the
// generic "did not answer" error instead of the cancellation (#430, the
// inline counterpart of the #233 EnsureDaemon fix).
func TestInlineDaemonAlreadyCancelledContext(t *testing.T) {
	a := inlineDaemonApp(t)
	ctx, cancel := context.WithCancel(context.Background())
	cancel()
	start := time.Now()
	_, err := a.inlineDaemon(ctx)
	if !errors.Is(err, context.Canceled) {
		t.Fatalf("inlineDaemon(cancelled ctx) = %v, want context.Canceled", err)
	}
	if elapsed := time.Since(start); elapsed > 2*time.Second {
		t.Fatalf("inlineDaemon(cancelled ctx) took %v, want an immediate return", elapsed)
	}
	if a.inlineStop != nil {
		t.Fatal("inlineDaemon started an in-process daemon despite the cancelled context")
	}
}

// neverAnswers replaces inlineDaemon's Ping with one that never reports
// the daemon up, so inlineDaemon stays in its poll loop until the context
// or the deadline ends the wait. The empty-config daemon answers the
// first real Ping, which is why the #430 tests never entered the loop
// (#323).
func neverAnswers(t *testing.T) {
	t.Helper()
	orig := inlineDaemonPing
	inlineDaemonPing = func(*Client, context.Context) bool { return false }
	t.Cleanup(func() { inlineDaemonPing = orig })
}

// The deadline-exit test below reuses errAfterContext from
// ensuredaemon_test.go: it reports context.Canceled from Err once its
// moment passes, but its Done channel never fires — a cancellation only
// a post-loop Err check can catch (the #328 shape, here for #323).

// inlineTempDirs lists the mcpx-inline-* directories currently in the
// temp dir, so a test can tell whether inlineDaemon removed the private
// directory of the daemon it started (its stop path) or leaked it.
func inlineTempDirs(t *testing.T) map[string]bool {
	t.Helper()
	matches, err := filepath.Glob(filepath.Join(os.TempDir(), "mcpx-inline-*"))
	if err != nil {
		t.Fatal(err)
	}
	out := make(map[string]bool, len(matches))
	for _, m := range matches {
		out[m] = true
	}
	return out
}

// Regression for #323: cancelling while inlineDaemon is still polling for
// its in-process daemon to answer must return context.Canceled promptly
// and stop the daemon, not burn the whole inline start timeout (5s by
// default) and report "the in-process daemon did not answer". Before this
// test, replacing the poll's select on ctx.Done() with a plain sleep left
// the suite green — neither #430 test ever entered the poll wait.
func TestInlineDaemonCancelledWhileWaiting(t *testing.T) {
	neverAnswers(t)
	a := inlineDaemonApp(t)
	before := inlineTempDirs(t)
	ctx, cancel := context.WithCancel(context.Background())
	go func() {
		time.Sleep(200 * time.Millisecond)
		cancel()
	}()
	start := time.Now()
	c, err := a.inlineDaemon(ctx)
	if c != nil {
		t.Fatalf("inlineDaemon(cancelled while waiting) returned a client %v, want none", c)
	}
	if !errors.Is(err, context.Canceled) {
		t.Fatalf("inlineDaemon(cancelled while waiting) = %v, want context.Canceled", err)
	}
	if elapsed := time.Since(start); elapsed > 2*time.Second {
		t.Fatalf("inlineDaemon took %v after cancellation, want a prompt return well under the 5s start timeout", elapsed)
	}
	for dir := range inlineTempDirs(t) {
		if !before[dir] {
			_ = os.RemoveAll(dir)
			t.Fatalf("inlineDaemon left its in-process daemon's directory %s behind after cancellation", dir)
		}
	}
}

// Regression for #323, the poll loop's other exit: a cancellation that
// only becomes visible as the loop leaves on its own deadline — its Done
// channel never fires, so the in-loop select cannot catch it — must still
// report context.Canceled via the post-loop check, not the generic
// "did not answer" timeout error. Removing that check leaves this test
// red while the mid-poll test above stays green.
func TestInlineDaemonCancelVisibleOnlyAtDeadline(t *testing.T) {
	neverAnswers(t)
	a := inlineDaemonApp(t)
	if err := a.Settings().SetRuntime("daemon.inlineStartTimeout", "800ms"); err != nil {
		t.Fatal(err)
	}
	ctx := errAfterContext{Context: context.Background(), at: time.Now().Add(150 * time.Millisecond)}
	c, err := a.inlineDaemon(ctx)
	if c != nil {
		t.Fatalf("inlineDaemon(cancel visible only at deadline) returned a client %v, want none", c)
	}
	if !errors.Is(err, context.Canceled) {
		t.Fatalf("inlineDaemon(cancel visible only at deadline) = %v, want context.Canceled", err)
	}
}

// The genuine timeout path, also untested before #323: with a live
// context and a daemon that never answers, inlineDaemon gives up at the
// inline start timeout with its "did not answer" error (and stops the
// daemon), rather than reporting a spurious cancellation.
func TestInlineDaemonNeverAnswersStillTimesOut(t *testing.T) {
	neverAnswers(t)
	a := inlineDaemonApp(t)
	if err := a.Settings().SetRuntime("daemon.inlineStartTimeout", "800ms"); err != nil {
		t.Fatal(err)
	}
	before := inlineTempDirs(t)
	start := time.Now()
	c, err := a.inlineDaemon(context.Background())
	if c != nil {
		t.Fatalf("inlineDaemon(never answers) returned a client %v, want none", c)
	}
	if err == nil || !strings.Contains(err.Error(), "the in-process daemon did not answer") {
		t.Fatalf("inlineDaemon(never answers) = %v, want the did-not-answer timeout error", err)
	}
	if elapsed := time.Since(start); elapsed < 700*time.Millisecond {
		t.Fatalf("inlineDaemon gave up after %v, before the 800ms start timeout", elapsed)
	}
	for dir := range inlineTempDirs(t) {
		if !before[dir] {
			_ = os.RemoveAll(dir)
			t.Fatalf("inlineDaemon left its in-process daemon's directory %s behind after the timeout", dir)
		}
	}
}

// The uncancelled path is unchanged: the in-process daemon starts and the
// returned client can ping it.
func TestInlineDaemonStartsWithLiveContext(t *testing.T) {
	a := inlineDaemonApp(t)
	ctx := context.Background()
	c, err := a.inlineDaemon(ctx)
	if err != nil {
		t.Fatalf("inlineDaemon(live ctx): %v", err)
	}
	defer a.CloseInline()
	if !c.Ping(ctx) {
		t.Fatal("the client returned by inlineDaemon does not ping its daemon")
	}
}

// Point 1 of #1868: inline mode must not create a socket for the CLI's
// own calls — the client talks to the daemon's handler in-process — and
// the socket is opened only when a script asks for it (Client.Socket).
func TestInlineDaemonCreatesNoSocketUntilAsked(t *testing.T) {
	a := inlineDaemonApp(t)
	before := inlineTempDirs(t)
	c, err := a.inlineDaemon(context.Background())
	if err != nil {
		t.Fatalf("inlineDaemon: %v", err)
	}
	defer a.CloseInline()
	if !c.Ping(context.Background()) {
		t.Fatal("the in-process client does not ping its daemon")
	}
	var dir string
	for d := range inlineTempDirs(t) {
		if !before[d] {
			dir = d
		}
	}
	if dir == "" {
		t.Fatal("no inline temp dir found")
	}
	sock := filepath.Join(dir, "d.sock")
	if _, err := os.Stat(sock); !os.IsNotExist(err) {
		t.Fatalf("socket %s exists before any script asked for it (stat err %v)", sock, err)
	}
	// A script asks: Socket opens it, and it answers like the daemon.
	if got := c.Socket(); got != sock {
		t.Fatalf("Socket() = %q, want %q", got, sock)
	}
	if _, err := os.Stat(sock); err != nil {
		t.Fatalf("socket not created by Socket(): %v", err)
	}
	paths := daemon.Paths{Socket: sock}
	if !NewClient(paths, a.ConfigPath).Ping(context.Background()) {
		t.Fatal("a socket client cannot ping the lazily opened socket")
	}
}

// The in-process transport is a real HTTP round trip to the handler:
// status, headers and a request body all survive it.
func TestInProcessTransportRoundTrip(t *testing.T) {
	handler := http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		body, _ := io.ReadAll(r.Body)
		w.Header().Set("X-Echo-Path", r.URL.Path)
		w.WriteHeader(http.StatusTeapot)
		_, _ = w.Write(body)
	})
	tr := newInProcessTransport(handler)
	defer tr.Close()
	hc := &http.Client{Transport: tr}
	resp, err := hc.Post("http://mcpx/v1/echo", "text/plain", strings.NewReader("ping-body"))
	if err != nil {
		t.Fatalf("RoundTrip: %v", err)
	}
	defer resp.Body.Close()
	if resp.StatusCode != http.StatusTeapot {
		t.Errorf("status = %d, want %d", resp.StatusCode, http.StatusTeapot)
	}
	if resp.Header.Get("X-Echo-Path") != "/v1/echo" {
		t.Errorf("X-Echo-Path = %q, want /v1/echo", resp.Header.Get("X-Echo-Path"))
	}
	body, err := io.ReadAll(resp.Body)
	if err != nil {
		t.Fatal(err)
	}
	if string(body) != "ping-body" {
		t.Errorf("body = %q, want the echoed request body", body)
	}
}
