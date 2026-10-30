package mcpclient_test

import (
	"context"
	"encoding/json"
	"strings"
	"sync"
	"testing"
	"time"

	"github.com/dezren39/mcpx/internal/mcpclient"
)

// PG-07 (#1924): ListResourceTemplates pages like every other list call.
func TestListResourceTemplatesPages(t *testing.T) {
	s := newScripted(func(method string, params map[string]any) any {
		switch method {
		case "server/discover":
			return modernDiscover(params)
		case "resources/templates/list":
			if params["cursor"] == nil {
				return map[string]any{
					"resultType":        "complete",
					"resourceTemplates": []any{map[string]any{"uriTemplate": "demo://t/{id}", "name": "t1"}},
					"nextCursor":        "t2",
				}
			}
			return map[string]any{
				"resultType":        "complete",
				"resourceTemplates": []any{map[string]any{"uriTemplate": "demo://u/{id}", "name": "t2"}},
			}
		}
		return nil
	})
	c := dialModern(t, s, mcpclient.Options{})
	got, err := c.ListResourceTemplates(context.Background())
	if err != nil {
		t.Fatal(err)
	}
	if len(got) != 2 || got[0].Name != "t1" || got[1].Name != "t2" {
		t.Fatalf("templates = %+v, want both pages (t1, t2)", got)
	}
}

// hangTransport answers server/discover, never answers tools/list, fails
// prompts/list with a real (non method-not-found) error, and records every
// frame — enough to watch the cancellation notification the client sends.
type hangTransport struct {
	mu     sync.Mutex
	in     chan []byte
	frames []map[string]any
	closed bool
}

func newHangTransport() *hangTransport { return &hangTransport{in: make(chan []byte, 64)} }

func (h *hangTransport) Send(_ context.Context, msg []byte) error {
	var f map[string]any
	_ = json.Unmarshal(msg, &f)
	h.mu.Lock()
	h.frames = append(h.frames, f)
	h.mu.Unlock()
	method, _ := f["method"].(string)
	if f["id"] == nil {
		return nil // a notification: recorded above, nothing to answer
	}
	reply := func(key string, v any) {
		b, _ := json.Marshal(map[string]any{"jsonrpc": "2.0", "id": f["id"], key: v})
		h.in <- b
	}
	switch method {
	case "server/discover":
		reply("result", modernDiscover(nil))
	case "prompts/list":
		reply("error", map[string]any{"code": -32603, "message": "prompts exploded"})
	}
	// tools/list is never answered: the call hangs until its ctx ends.
	return nil
}

func (h *hangTransport) Recv() ([]byte, error) {
	msg, ok := <-h.in
	if !ok {
		return nil, context.Canceled
	}
	return msg, nil
}

func (h *hangTransport) Info() string { return "hang" }

func (h *hangTransport) Close() error {
	h.mu.Lock()
	defer h.mu.Unlock()
	if !h.closed {
		h.closed = true
		close(h.in)
	}
	return nil
}

func (h *hangTransport) cancelReasons() []string {
	h.mu.Lock()
	defer h.mu.Unlock()
	var out []string
	for _, f := range h.frames {
		if f["method"] == "notifications/cancelled" {
			if p, ok := f["params"].(map[string]any); ok {
				r, _ := p["reason"].(string)
				out = append(out, r)
			}
		}
	}
	return out
}

func dialHang(t *testing.T, h *hangTransport) *mcpclient.Client {
	t.Helper()
	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()
	c, err := mcpclient.NewWithOptions(ctx, h, mcpclient.Options{
		ClientName: "mcpx", ClientVersion: "test", Preference: mcpclient.ForceModern,
	})
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { c.Close() })
	return c
}

// PG-07 (#1924): a prompts/list failure that is not method-not-found
// surfaces instead of reading as "this server has no prompts".
func TestListPromptsSurfacesRealErrors(t *testing.T) {
	c := dialHang(t, newHangTransport())
	_, _, err := c.ListPromptsWithHints(context.Background())
	if err == nil || !strings.Contains(err.Error(), "prompts exploded") {
		t.Fatalf("err = %v, want the server's prompts/list failure", err)
	}
}

// PC-08 (#1924): the cancellation reason is the truth — a caller's own
// cancel is not reported as a timeout, and a deadline still is.
func TestCancellationReasonIsTruthful(t *testing.T) {
	h := newHangTransport()
	c := dialHang(t, h)

	ctx, cancel := context.WithCancel(context.Background())
	done := make(chan error, 1)
	go func() {
		_, err := c.ListTools(ctx)
		done <- err
	}()
	time.Sleep(100 * time.Millisecond)
	cancel()
	if err := <-done; err == nil {
		t.Fatal("the hung call should have failed on cancel")
	}
	if got := h.cancelReasons(); len(got) != 1 || got[0] != "context canceled" {
		t.Fatalf("cancel reasons = %v, want [context canceled]", got)
	}

	h2 := newHangTransport()
	c2 := dialHang(t, h2)
	dctx, dcancel := context.WithTimeout(context.Background(), 100*time.Millisecond)
	defer dcancel()
	if _, err := c2.ListTools(dctx); err == nil {
		t.Fatal("the hung call should have failed on deadline")
	}
	if got := h2.cancelReasons(); len(got) != 1 || got[0] != "timeout" {
		t.Fatalf("cancel reasons = %v, want [timeout]", got)
	}
}
