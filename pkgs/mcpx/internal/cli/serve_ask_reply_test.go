package cli

import (
	"context"
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"

	"github.com/developing-today/code/pkgs/mcpx/internal/daemon"
)

// #8922: the daemon's ask-answers route reports refused answers in the
// problems map of an otherwise-200 response. daemonAsker.Reply used to
// discard that body, so the relay loop believed the question answered,
// re-polled, re-pushed the same question and burned every AskRound
// before failing the call. Reply must surface the refusal as an error
// naming it, so the loop fails the call fast with the reason.
func TestDaemonAskerReplySurfacesRefusals(t *testing.T) {
	daemonSrv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.URL.Path != "/v1/ask/call-1/answers" {
			http.Error(w, `{"error":"unexpected path"}`, http.StatusNotFound)
			return
		}
		_, _ = w.Write([]byte(`{"accepted":0,"problems":{"q1":` +
			`"accepted content does not match the requested schema: repo is required"}}`))
	}))
	defer daemonSrv.Close()

	app := &App{client: NewClientAt(daemon.Paths{}, "", daemonSrv.URL), connected: true}
	asker := daemonAsker{app: app}
	err := asker.Reply(context.Background(), "call-1", map[string]json.RawMessage{
		"q1": json.RawMessage(`{"action":"accept","content":{"count":1}}`),
	})
	if err == nil {
		t.Fatal("Reply dropped a refused answer: got nil error")
	}
	if !strings.Contains(err.Error(), "refused") ||
		!strings.Contains(err.Error(), "q1") ||
		!strings.Contains(err.Error(), "requested schema") {
		t.Fatalf("Reply error does not name the refusal: %v", err)
	}
}

// A receipt with no problems is not an error.
func TestDaemonAskerReplyAcceptsCleanReceipt(t *testing.T) {
	daemonSrv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		_, _ = w.Write([]byte(`{"accepted":1}`))
	}))
	defer daemonSrv.Close()

	app := &App{client: NewClientAt(daemon.Paths{}, "", daemonSrv.URL), connected: true}
	asker := daemonAsker{app: app}
	if err := asker.Reply(context.Background(), "call-1", map[string]json.RawMessage{
		"q1": json.RawMessage(`{"action":"accept","content":{"repo":"me/thing"}}`),
	}); err != nil {
		t.Fatalf("Reply failed on a clean receipt: %v", err)
	}
}
