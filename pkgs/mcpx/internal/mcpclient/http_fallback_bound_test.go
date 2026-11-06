package mcpclient_test

import (
	"context"
	"fmt"
	"io"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
	"time"

	"github.com/developing-today/code/pkgs/mcpx/internal/defaults"
	"github.com/developing-today/code/pkgs/mcpx/internal/mcpclient"
)

// sendFrame POSTs one frame and returns the next queued reply frame, failing
// the test if none arrives.
func sendFrame(t *testing.T, tr *mcpclient.HTTPTransport, msg string) []byte {
	t.Helper()
	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()
	if err := tr.Send(ctx, []byte(msg)); err != nil {
		t.Fatalf("Send: %v", err)
	}
	type result struct {
		b   []byte
		err error
	}
	done := make(chan result, 1)
	go func() {
		b, err := tr.Recv()
		done <- result{b, err}
	}()
	select {
	case r := <-done:
		if r.err != nil {
			t.Fatalf("Recv: %v", r.err)
		}
		return r.b
	case <-time.After(5 * time.Second):
		t.Fatal("timed out waiting for the reply frame")
		return nil
	}
}

func plainServer(t *testing.T, contentType, body string) *httptest.Server {
	t.Helper()
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.Header().Set("Content-Type", contentType)
		w.WriteHeader(http.StatusOK)
		_, _ = io.WriteString(w, body)
	}))
	t.Cleanup(srv.Close)
	return srv
}

func dialTestTransport(t *testing.T, url string) *mcpclient.HTTPTransport {
	t.Helper()
	tr, err := mcpclient.NewHTTP(mcpclient.HTTPOptions{URL: url})
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { _ = tr.Close() })
	return tr
}

const fallbackPing = `{"jsonrpc":"2.0","id":1,"method":"ping"}`

// TestSendNonJSONFallbackIsBounded is the regression test for #4282: an
// oversized non-JSON 200 body must be truncated at HTTPErrorBodyLimit, never
// buffered and pushed whole. Bodies at or under the limit pass through
// byte-for-byte, as before.
func TestSendNonJSONFallbackIsBounded(t *testing.T) {
	limit := int64(defaults.HTTPErrorBodyLimit)
	for _, size := range []int64{16, limit - 1, limit, limit + 1, limit*4 + 123} {
		t.Run(fmt.Sprintf("bytes-%d", size), func(t *testing.T) {
			body := strings.Repeat("A", int(size)) // never valid JSON
			tr := dialTestTransport(t, plainServer(t, "text/html", body).URL)
			got := sendFrame(t, tr, fallbackPing)
			if size <= limit {
				if string(got) != body {
					t.Fatalf("small non-JSON body changed: got %d bytes, want %d whole", len(got), len(body))
				}
				return
			}
			if int64(len(got)) > limit {
				t.Fatalf("oversized non-JSON body not bounded: got %d bytes, limit %d", len(got), limit)
			}
			if int64(len(got)) != limit {
				t.Fatalf("oversized non-JSON body not truncated at the limit: got %d bytes, want %d", len(got), limit)
			}
			if len(got) >= len(body) {
				t.Fatalf("oversized non-JSON body pushed whole: got %d bytes of %d", len(got), len(body))
			}
			if string(got) != body[:len(got)] {
				t.Fatalf("bounded body is not the head of the reply: got %q", got)
			}
		})
	}
}

// TestSendEmptyNonJSONPushesNothing covers the empty-body corner of the same
// fallback: nothing to judge, nothing queued.
func TestSendEmptyNonJSONPushesNothing(t *testing.T) {
	tr := dialTestTransport(t, plainServer(t, "text/plain", "").URL)
	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()
	if err := tr.Send(ctx, []byte(fallbackPing)); err != nil {
		t.Fatalf("Send: %v", err)
	}
	select {
	case b := <-recvChan(t, tr):
		t.Fatalf("empty body queued a frame: %q", b)
	case <-time.After(200 * time.Millisecond):
	}
}

func recvChan(t *testing.T, tr *mcpclient.HTTPTransport) <-chan []byte {
	t.Helper()
	ch := make(chan []byte, 1)
	go func() {
		b, err := tr.Recv()
		if err == nil {
			ch <- b
		}
	}()
	return ch
}

// TestSendValidJSONUnchanged guards the success path the bound must not
// touch: a single JSON value, including one far larger than the fallback
// limit, arrives whole.
func TestSendValidJSONUnchanged(t *testing.T) {
	limit := int64(defaults.HTTPErrorBodyLimit)
	big := strings.Repeat("z", int(limit*2))
	for _, reply := range []string{
		`{"jsonrpc":"2.0","id":1,"result":{"ok":true}}`,
		`{"jsonrpc":"2.0","id":1,"result":{"blob":"` + big + `"}}`,
	} {
		tr := dialTestTransport(t, plainServer(t, "application/json", reply).URL)
		if got := sendFrame(t, tr, fallbackPing); string(got) != reply {
			t.Fatalf("valid JSON changed: got %d bytes, want %d", len(got), len(reply))
		}
	}
}

// TestSendSSEUnchanged guards the SSE path the bound must not touch.
func TestSendSSEUnchanged(t *testing.T) {
	payload := `{"jsonrpc":"2.0","id":1,"result":{"ok":true}}`
	tr := dialTestTransport(t, plainServer(t, "text/event-stream", "data: "+payload+"\n\n").URL)
	if got := sendFrame(t, tr, fallbackPing); string(got) != payload {
		t.Fatalf("SSE payload changed: got %q, want %q", got, payload)
	}
}
