package daemon

import (
	"context"
	"encoding/json"
	"errors"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"github.com/dezren39/mcpx/internal/config"
	"github.com/dezren39/mcpx/internal/elicit"
	"github.com/dezren39/mcpx/internal/events"
	"github.com/dezren39/mcpx/internal/mcpclient"
)

// Typed sampling errors from the broker path (issue #1835, Phase 1 of
// #1834): sampleViaBroker must tell a real refusal apart from an expiry.
// A decline or cancel by an actual answerer is a typed -1 naming the
// action; the broker's synthesized expiry answer (Action cancel, By
// "expiry") is a typed -32603 naming the expiry, never a rejection.

func samplingRegistry(t *testing.T) *Registry {
	t.Helper()
	dir := t.TempDir()
	cfg := &config.Config{MCPServers: map[string]*config.Server{}}
	r, err := NewRegistry(cfg, Paths{State: dir, Cache: dir}, nil)
	if err != nil {
		t.Fatal(err)
	}
	broker, err := OpenBroker(Paths{State: filepath.Join(dir, "state")})
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { _ = broker.Close() })
	r.InstallHooks(events.New(0), broker, nil)
	return r
}

// answerSample runs sampleViaBroker and answers its broker question with
// ans (ID filled in) once the question is pending.
func answerSample(t *testing.T, r *Registry, ans elicit.Answer) error {
	t.Helper()
	ctx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
	defer cancel()
	errc := make(chan error, 1)
	go func() {
		_, err := r.sampleViaBroker(ctx, "demo", "key",
			json.RawMessage(`{"messages":[{"role":"user","content":{"type":"text","text":"hi"}}],"maxTokens":7}`))
		errc <- err
	}()
	deadline := time.Now().Add(5 * time.Second)
	for {
		pending, err := r.broker.Pending(elicit.Filter{})
		if err != nil {
			t.Fatal(err)
		}
		if len(pending) == 1 {
			ans.ID = pending[0].ID
			if err := r.broker.Respond(ans); err != nil {
				t.Fatal(err)
			}
			break
		}
		if time.Now().After(deadline) {
			t.Fatalf("the sampling question never reached the broker: %v pending", len(pending))
		}
		time.Sleep(10 * time.Millisecond)
	}
	return <-errc
}

func TestSampleViaBrokerTypedErrors(t *testing.T) {
	cases := []struct {
		name     string
		ans      elicit.Answer
		wantCode int
		wantText string
	}{
		{"decline by an answerer is -1 naming the decline",
			elicit.Answer{Action: elicit.Decline, By: "tester"}, -1, "decline"},
		{"cancel by an answerer is -1 naming the cancel",
			elicit.Answer{Action: elicit.Cancel, By: "tester"}, -1, "cancel"},
		{"synthesized expiry answer is -32603 naming expiry",
			elicit.Answer{Action: elicit.Cancel, By: "expiry"}, -32603, "expired"},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			r := samplingRegistry(t)
			err := answerSample(t, r, tc.ans)
			var ce *mcpclient.CodedError
			if !errors.As(err, &ce) {
				t.Fatalf("the sampling error is not typed: %v", err)
			}
			if ce.Code != tc.wantCode {
				t.Fatalf("code %d, want %d (err %v)", ce.Code, tc.wantCode, err)
			}
			if !strings.Contains(ce.Message, tc.wantText) {
				t.Fatalf("the message %q does not name %q", ce.Message, tc.wantText)
			}
		})
	}
}
