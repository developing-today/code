package mcpclient

import (
	"context"
	"errors"
	"net/http"
	"strings"
	"sync/atomic"
	"testing"
)

// #290: a transport must not share http.DefaultTransport with any other
// transport. Abandon closes the idle connections of the client's pool, and
// on the process-wide default pool that reaches every other mcpx upstream
// and stream; a connection canceled that way can be handed to another
// transport's request, which net/http then fails as context.Canceled
// without writing it.
func TestEachHTTPTransportOwnsItsConnectionPool(t *testing.T) {
	a, err := NewHTTP(HTTPOptions{URL: "http://127.0.0.1:1/mcp"})
	if err != nil {
		t.Fatal(err)
	}
	b, err := NewHTTP(HTTPOptions{URL: "http://127.0.0.1:1/mcp"})
	if err != nil {
		t.Fatal(err)
	}
	for name, client := range map[string]*http.Client{"request": a.hc, "stream": a.streamHC} {
		if client.Transport == http.DefaultTransport {
			t.Fatalf("the %s client uses http.DefaultTransport; its connections are shared with every other transport", name)
		}
	}
	if a.hc.Transport == b.hc.Transport {
		t.Fatal("two transports share one connection pool")
	}
	if a.hc.Transport == a.streamHC.Transport {
		t.Fatal("a transport's request and stream clients share one connection pool; an abandoned stream can cancel a request queued on it")
	}
}

// countingRT answers with fn and counts the requests it saw.
type countingRT struct {
	n  atomic.Int32
	fn func(*http.Request) (*http.Response, error)
}

func (c *countingRT) RoundTrip(req *http.Request) (*http.Response, error) {
	c.n.Add(1)
	return c.fn(req)
}

func okResponse(req *http.Request) (*http.Response, error) {
	return &http.Response{StatusCode: http.StatusOK, Body: http.NoBody, Request: req}, nil
}

// closeSessionOn builds a transport whose session is resumed and whose
// request client is rt.
func closeSessionOn(t *testing.T, rt http.RoundTripper) *HTTPTransport {
	t.Helper()
	tr, err := NewHTTP(HTTPOptions{URL: "http://127.0.0.1:1/mcp"})
	if err != nil {
		t.Fatal(err)
	}
	tr.hc = &http.Client{Transport: rt}
	tr.Resume(HTTPHandoff{SessionID: "sess-1"})
	return tr
}

// cancelFirstTransport fails the first DELETE with context.Canceled, the way
// net/http reports a request whose pooled connection was torn down by
// another request's cancellation, and answers the reissue normally.
func cancelFirstTransport(t *testing.T) *countingRT {
	t.Helper()
	return &countingRT{fn: func(req *http.Request) (*http.Response, error) {
		if req.Method == http.MethodDelete {
			return nil, context.Canceled
		}
		return okResponse(req)
	}}
}

func TestCloseSessionReissuesADeleteTheTransportCancels(t *testing.T) {
	first := cancelFirstTransport(t)
	var deletes atomic.Int32
	rt := &countingRT{fn: func(req *http.Request) (*http.Response, error) {
		if req.Method == http.MethodDelete && deletes.Add(1) > 1 {
			return okResponse(req)
		}
		return first.fn(req)
	}}
	tr := closeSessionOn(t, rt)

	if err := tr.closeSession(); err != nil {
		t.Fatalf("closeSession: %v", err)
	}
	if got := deletes.Load(); got != 2 {
		t.Fatalf("the DELETE was issued %d time(s), want 2: the reissue is what ends the session after a transport cancellation", got)
	}
}

// A cancel that outlives the reissue is still reported, not retried forever.
func TestCloseSessionReportsAPersistentCancel(t *testing.T) {
	rt := &countingRT{fn: func(req *http.Request) (*http.Response, error) {
		return nil, context.Canceled
	}}
	tr := closeSessionOn(t, rt)

	err := tr.closeSession()
	if !errors.Is(err, context.Canceled) {
		t.Fatalf("closeSession = %v, want context.Canceled", err)
	}
	if n := rt.n.Load(); n != deleteSessionAttempts {
		t.Fatalf("the DELETE was attempted %d time(s), want %d: a persistent cancel must not loop", n, deleteSessionAttempts)
	}
}

// A server status is a conclusive answer: never reissued.
func TestCloseSessionDoesNotReissueAServerStatus(t *testing.T) {
	rt := &countingRT{fn: func(req *http.Request) (*http.Response, error) {
		return &http.Response{StatusCode: http.StatusInternalServerError, Body: http.NoBody, Request: req}, nil
	}}
	tr := closeSessionOn(t, rt)

	err := tr.closeSession()
	if err == nil || !strings.Contains(err.Error(), "http 500") {
		t.Fatalf("closeSession = %v, want the 500 reported", err)
	}
	if n := rt.n.Load(); n != 1 {
		t.Fatalf("the DELETE was issued %d time(s), want 1", n)
	}
}
