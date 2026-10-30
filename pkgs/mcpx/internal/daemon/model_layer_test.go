package daemon

import (
	"context"
	"errors"
	"io"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
	"time"

	"github.com/dezren39/mcpx/internal/elicit"
	"github.com/dezren39/mcpx/internal/model"
	"github.com/dezren39/mcpx/internal/scripting"
)

// Codex P2 (PR #2592): a model layer that was configured but cannot
// resolve (plugin = not built yet) must surface as no-model, never fall
// through to the legacy repair provider -- that would silently spend a
// different provider than the one the user selected.
func TestGenerateModelLayerResolutionErrorSkipsLegacyProvider(t *testing.T) {
	s := intentTestServer()
	pol := defaultConsumerPolicy()
	pol.ModelProvider = "plugin"
	ctx := withPolicy(context.Background(), pol)
	called := false
	prov := scripting.NewCallbackProvider("legacy", func(ctx context.Context, req scripting.CompletionRequest) (string, error) {
		called = true
		return "return 1;", nil
	})
	_, _, err := s.generate(ctx, "get chicago weather", prov, "", "")
	if !errors.Is(err, errNoModel) {
		t.Fatalf("err = %v, want errNoModel", err)
	}
	if called {
		t.Fatal("legacy provider was invoked after a model-layer resolution failure")
	}
}

// Codex P1 (PR #2592): a configured model-layer provider must be usable
// with NO legacy provider at all -- the routes_consumer guard change is
// what lets the request reach this path, and generate must answer
// through the layer alone.
func TestGenerateModelLayerAloneAnswers(t *testing.T) {
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.Header().Set("Content-Type", "application/json")
		_, _ = io.WriteString(w, `{"model":"test-model","choices":[{"message":{"role":"assistant","content":"const data = await tools.weather.forecast({city: 'Chicago'});\nreturn data;"}}]}`)
	}))
	defer srv.Close()
	s := intentTestServer()
	pol := defaultConsumerPolicy()
	pol.ModelProvider = "openai"
	pol.ModelBaseURL = srv.URL
	pol.ModelDefault = "openai/test-model"
	pol.ModelTimeout = 5 * time.Second
	ctx := withPolicy(context.Background(), pol)
	source, used, err := s.generate(ctx, "get chicago weather", nil, "", "")
	if err != nil {
		t.Fatalf("generate: %v", err)
	}
	if !strings.Contains(source, "tools.weather.forecast") {
		t.Errorf("source = %q", source)
	}
	if used != "provider:openai/test-model" {
		t.Errorf("used = %q, want provider:openai/test-model", used)
	}
}

// Codex P2s (PR #2592): the sampling question carries the intent session
// (so Pending's session filter can find it) and model.timeout bounds the
// wait itself, not only the question's initial TTL.
func TestBrokerSamplerSessionAndTimeout(t *testing.T) {
	reg := samplingRegistry(t)
	s := &Server{reg: reg}
	pol := defaultConsumerPolicy()
	pol.ModelTimeout = 300 * time.Millisecond
	ctx := withPolicy(context.Background(), pol)
	fn := s.brokerSampler("sess-9")
	errc := make(chan error, 1)
	go func() {
		_, err := fn(ctx, model.Request{Messages: []model.Message{{Role: "user", Content: "hi"}}})
		errc <- err
	}()
	deadline := time.Now().Add(3 * time.Second)
	for {
		pending, err := reg.broker.Pending(elicit.Filter{Session: "sess-9"})
		if err != nil {
			t.Fatal(err)
		}
		if len(pending) == 1 {
			if pending[0].Session != "sess-9" {
				t.Fatalf("question session = %q, want sess-9", pending[0].Session)
			}
			break
		}
		if time.Now().After(deadline) {
			t.Fatalf("no pending question under session sess-9 (%d pending)", len(pending))
		}
		time.Sleep(10 * time.Millisecond)
	}
	select {
	case err := <-errc:
		if err == nil {
			t.Fatal("unanswered sampling returned no error")
		}
	case <-time.After(5 * time.Second):
		t.Fatal("sampling wait was not bounded by model.timeout")
	}
}
