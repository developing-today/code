package daemon

import (
	"bytes"
	"context"
	"net/http"
	"strings"
	"sync"
	"testing"
	"time"

	"github.com/dezren39/mcpx/internal/defaults"
	"github.com/dezren39/mcpx/internal/events"
	"github.com/dezren39/mcpx/internal/settings"
)

// A subscriber that falls behind LIVE must be told, in band, on the stream.
//
// Subscribe-time replay already reports its gap (handleEvents' first gap
// event, pinned by TestAReplayLargerThanTheSubscriberBufferMissesNothing in
// the events package). Post-subscribe drops were counted by
// Subscription.Dropped and read by nothing, so a client that stalled for a
// moment silently missed events while the docs promised it would be told.
//
// The stall is produced without timing luck: the writer's first Flush blocks
// until the test releases it, which is exactly a slow client, so every event
// published in that window is dropped by the subscription's non-blocking
// send. After the release the handler must emit the gap notice before the
// next event it delivers.
func TestEventsTellsAStalledSubscriberWhatItMissed(t *testing.T) {
	bus := events.New(defaults.EventHistory)
	s := &Server{Events: bus, set: gapTestSettings(t)}

	w := newGatedWriter()
	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()
	req, err := http.NewRequestWithContext(ctx, http.MethodGet, "/v1/events", nil)
	if err != nil {
		t.Fatal(err)
	}

	done := make(chan struct{})
	go func() {
		defer close(done)
		s.handleEvents(w, req)
	}()

	// The first flush happens after the subscription exists and before the
	// read loop starts, so once it is entered the subscription is live and
	// nothing has been drained yet.
	select {
	case <-w.entered:
	case <-time.After(2 * time.Second):
		t.Fatal("handler never reached its first flush")
	}

	// Far more than the subscriber buffer, published while the client is
	// stalled: the excess is what Dropped counts.
	for i := 0; i < defaults.EventSubscriberBuf+64; i++ {
		bus.Publish(events.Event{Kind: events.CallFinished})
	}
	close(w.release)

	deadline := time.Now().Add(2 * time.Second)
	for !strings.Contains(w.String(), "event: gap") {
		if time.Now().After(deadline) {
			t.Fatalf("no gap event on the stream after a live drop; body so far:\n%s", w.String())
		}
		time.Sleep(5 * time.Millisecond)
	}
	cancel()
	<-done

	body := w.String()
	gapLine := body[strings.Index(body, "event: gap"):]
	if !strings.Contains(gapLine, `"dropped":`) {
		t.Errorf("gap event should say how many were dropped, got %q", gapLine)
	}
	if !strings.Contains(gapLine, `"latest":`) {
		t.Errorf("gap event should name the latest sequence for resync, got %q", gapLine)
	}
	// The notice has to come before the event that follows the hole, or the
	// client reads a sequence jump first and only then learns why.
	if i, j := strings.Index(body, "event: gap"), strings.Index(body, "event: call.finished"); i > j {
		t.Errorf("gap event written after the event it precedes:\n%s", body)
	}
}

func gapTestSettings(t *testing.T) *settings.Set {
	t.Helper()
	sch, err := settings.New(settings.Registry())
	if err != nil {
		t.Fatal(err)
	}
	return settings.NewSet(sch).WithOverrides(nil, "test")
}

// gatedWriter is an http.ResponseWriter whose first Flush blocks until the
// test closes release. It records the body under a mutex because the handler
// writes from its own goroutine while the test reads.
type gatedWriter struct {
	mu      sync.Mutex
	buf     bytes.Buffer
	header  http.Header
	entered chan struct{}
	release chan struct{}
	once    sync.Once
}

func newGatedWriter() *gatedWriter {
	return &gatedWriter{
		header:  http.Header{},
		entered: make(chan struct{}),
		release: make(chan struct{}),
	}
}

func (g *gatedWriter) Header() http.Header { return g.header }

func (g *gatedWriter) Write(p []byte) (int, error) {
	g.mu.Lock()
	defer g.mu.Unlock()
	return g.buf.Write(p)
}

func (g *gatedWriter) WriteHeader(int) {}

func (g *gatedWriter) Flush() {
	g.once.Do(func() {
		close(g.entered)
		<-g.release
	})
}

func (g *gatedWriter) String() string {
	g.mu.Lock()
	defer g.mu.Unlock()
	return g.buf.String()
}
