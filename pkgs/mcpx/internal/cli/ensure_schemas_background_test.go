package cli

import (
	"context"
	"net/http"
	"net/http/httptest"
	"sync/atomic"
	"testing"
	"time"

	"github.com/developing-today/code/pkgs/mcpx/internal/daemon"
)

// Failing-first tests for #1868 point 3 slice 2: uncached namespaces must
// not force a blocking full Refresh on implicit entry paths. Cached
// namespaces serve immediately from typed stubs; the first use of a cold
// namespace awaits its schema read daemon-side (CallGenerated), while the
// CLI kicks the read off in the background.

// ensureAnySchemas must return quickly on a cold cache and start the schema
// read in the background instead of blocking on POST /v1/refresh.
func TestEnsureAnySchemasBackgroundsColdRefresh1868(t *testing.T) {
	var refreshCalls atomic.Int32
	release := make(chan struct{})
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		switch r.URL.Path {
		case "/v1/namespaces":
			// One warm namespace serves immediately; one cold forces a read.
			_, _ = w.Write([]byte(`[{"namespace":"warm","server":"warm","cached":true},{"namespace":"cold","server":"cold","cached":false}]`))
		case "/v1/refresh":
			refreshCalls.Add(1)
			select {
			case <-release:
			case <-r.Context().Done():
				return
			}
			_, _ = w.Write([]byte(`{}`))
		default:
			http.NotFound(w, r)
		}
	}))
	defer srv.Close()
	defer close(release)

	app := &App{client: NewClientAt(daemon.Paths{}, "", srv.URL), connected: true}
	ctx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
	defer cancel()

	start := time.Now()
	if err := app.ensureAnySchemas(ctx, app.client); err != nil {
		t.Fatalf("ensureAnySchemas: %v", err)
	}
	if elapsed := time.Since(start); elapsed > 2*time.Second {
		t.Fatalf("ensureAnySchemas blocked %s on a cold cache; must return immediately and refresh in the background", elapsed)
	}
	// The background read must actually start: the daemon sees the refresh
	// even though the CLI did not wait for it.
	deadline := time.Now().Add(5 * time.Second)
	for refreshCalls.Load() == 0 && time.Now().Before(deadline) {
		time.Sleep(10 * time.Millisecond)
	}
	if refreshCalls.Load() == 0 {
		t.Fatal("no background schema refresh started for the cold namespace")
	}
}

// A namespace whose fetch already failed carries its error on /v1/namespaces
// and must never trigger another blocking refresh: the error surfaces via
// ls, without hanging the CLI.
func TestEnsureAnySchemasSkipsBrokenWithoutHanging1868(t *testing.T) {
	var refreshCalls atomic.Int32
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		switch r.URL.Path {
		case "/v1/namespaces":
			_, _ = w.Write([]byte(`[{"namespace":"warm","server":"warm","cached":true},{"namespace":"broken","server":"broken","cached":false,"error":"dial refused"}]`))
		case "/v1/refresh":
			refreshCalls.Add(1)
			_, _ = w.Write([]byte(`{}`))
		default:
			http.NotFound(w, r)
		}
	}))
	defer srv.Close()

	appSkip := &App{client: NewClientAt(daemon.Paths{}, "", srv.URL), connected: true}
	ctxSkip, cancelSkip := context.WithTimeout(context.Background(), 10*time.Second)
	defer cancelSkip()

	start := time.Now()
	if err := appSkip.ensureAnySchemas(ctxSkip, appSkip.client); err != nil {
		t.Fatalf("ensureAnySchemas: %v", err)
	}
	if elapsed := time.Since(start); elapsed > 2*time.Second {
		t.Fatalf("ensureAnySchemas blocked %s with only a broken namespace cold", elapsed)
	}
	// Give any mistaken background refresh a moment to (not) appear.
	time.Sleep(200 * time.Millisecond)
	if n := refreshCalls.Load(); n != 0 {
		t.Fatalf("broken namespace triggered %d refresh(es); its error must surface via ls, not a retry on every command", n)
	}
}

// An explicit --ns selection of a cold namespace is a first use: it awaits
// that namespace's schema read instead of serving a stub.
func TestEnsureSchemasBlocksForExplicitColdNamespace1868(t *testing.T) {
	release := make(chan struct{})
	var refreshCalls atomic.Int32
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		switch r.URL.Path {
		case "/v1/namespaces":
			_, _ = w.Write([]byte(`[{"namespace":"warm","server":"warm","cached":true},{"namespace":"cold","server":"cold","cached":false}]`))
		case "/v1/refresh":
			refreshCalls.Add(1)
			select {
			case <-release:
			case <-r.Context().Done():
				return
			}
			_, _ = w.Write([]byte(`{}`))
		default:
			http.NotFound(w, r)
		}
	}))
	defer srv.Close()

	app := &App{client: NewClientAt(daemon.Paths{}, "", srv.URL), connected: true}
	ctx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
	defer cancel()

	done := make(chan error, 1)
	go func() {
		done <- app.ensureSchemas(ctx, app.client, []string{"cold"})
	}()

	// The explicit first use must still be waiting for the schema read.
	select {
	case err := <-done:
		t.Fatalf("ensureSchemas returned %v before the cold read landed; explicit first use must await it", err)
	case <-time.After(300 * time.Millisecond):
	}
	close(release)

	select {
	case err := <-done:
		if err != nil {
			t.Fatalf("ensureSchemas: %v", err)
		}
	case <-ctx.Done():
		t.Fatal("ensureSchemas did not return after the schema read landed")
	}
	if refreshCalls.Load() != 1 {
		t.Fatalf("explicit cold selection did %d refresh(es), want exactly 1", refreshCalls.Load())
	}
}
