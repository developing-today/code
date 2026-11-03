package daemon

import (
	"context"
	"encoding/json"
	"errors"
	"io"
	"net/http"
	"net/http/httptest"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"github.com/developing-today/code/pkgs/mcpx/internal/config"
	"github.com/developing-today/code/pkgs/mcpx/internal/elicit"
	"github.com/developing-today/code/pkgs/mcpx/internal/events"
	"github.com/developing-today/code/pkgs/mcpx/internal/model"
	"github.com/developing-today/code/pkgs/mcpx/internal/scripting"
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
	_, _, err := s.generate(ctx, "get chicago weather", prov, "", "", "")
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
	source, used, err := s.generate(ctx, "get chicago weather", nil, "", "", "")
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

// Codex P2 (PR #2592): an intent request's EXPLICIT provider is the
// caller's own choice for that request (the Client.Intent contract);
// a configured model.provider must not take precedence over it and
// send the caller's model id to a different endpoint.
func TestGenerateExplicitProviderSkipsModelLayer(t *testing.T) {
	layerHit := false
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		layerHit = true
		w.Header().Set("Content-Type", "application/json")
		_, _ = io.WriteString(w, `{"model":"test-model","choices":[{"message":{"role":"assistant","content":"return 2;"}}]}`)
	}))
	defer srv.Close()
	s := intentTestServer()
	pol := defaultConsumerPolicy()
	pol.ModelProvider = "openai"
	pol.ModelBaseURL = srv.URL
	pol.ModelTimeout = 5 * time.Second
	ctx := withPolicy(context.Background(), pol)
	legacyCalled := false
	prov := scripting.NewCallbackProvider("legacy", func(ctx context.Context, req scripting.CompletionRequest) (string, error) {
		legacyCalled = true
		return "return 1;", nil
	})
	source, _, err := s.generate(ctx, "get chicago weather", prov, "caller-model", "", "claude")
	if err != nil {
		t.Fatalf("generate: %v", err)
	}
	if !legacyCalled {
		t.Error("explicit provider was not used for generation")
	}
	if layerHit {
		t.Error("model layer was invoked despite the request's explicit provider")
	}
	if !strings.Contains(source, "return 1;") {
		t.Errorf("source = %q, want the explicit provider's answer", source)
	}
}

// Codex P2 (PR #2592): a negative per-call token limit is unset, not a
// way around model.budget -- the backend omits a nonpositive limit, so
// an unclamped -1 would leave the endpoint's own limit in force.
func TestModelMaxTokens(t *testing.T) {
	for _, tc := range []struct{ perCall, budget, want int }{
		{-1, 500, 500},
		{-1, 0, 0},
		{0, 500, 500},
		{0, 0, 0},
		{100, 500, 100},
		{900, 500, 500},
		{100, 0, 100},
	} {
		if got := modelMaxTokens(tc.perCall, tc.budget); got != tc.want {
			t.Errorf("modelMaxTokens(%d, %d) = %d, want %d", tc.perCall, tc.budget, got, tc.want)
		}
	}
}

type recordingModelProvider struct {
	name string
	got  model.Request
}

func (p *recordingModelProvider) Name() string { return p.name }
func (p *recordingModelProvider) Generate(ctx context.Context, req model.Request) (model.Response, error) {
	p.got = req
	return model.Response{Text: "return 3;", Provider: p.name}, nil
}

// Codex P2 (PR #2592): repair of a model-layer script goes through the
// layer's own backend, with the layer's budget clamping the repair
// request's own token ask and model.default supplying the model id.
func TestModelRepairProviderCarriesLayerBounds(t *testing.T) {
	s := intentTestServer()
	pol := defaultConsumerPolicy()
	pol.ModelBudget = 500
	pol.ModelDefault = "openai/test-model"
	ctx := withPolicy(context.Background(), pol)
	rec := &recordingModelProvider{name: "openai"}
	rp := modelRepairProvider{s: s, mp: rec}
	out, err := rp.Complete(ctx, scripting.CompletionRequest{
		SystemPrompt: "sys", UserPrompt: "fix it", MaxTokens: 3000,
	})
	if err != nil {
		t.Fatalf("Complete: %v", err)
	}
	if out != "return 3;" {
		t.Errorf("out = %q", out)
	}
	if rec.got.MaxTokens != 500 {
		t.Errorf("repair MaxTokens = %d, want the model.budget cap 500", rec.got.MaxTokens)
	}
	if rec.got.Model != "openai/test-model" {
		t.Errorf("repair Model = %q, want model.default", rec.got.Model)
	}
	if rp.Name() != "openai" {
		t.Errorf("Name = %q", rp.Name())
	}
}

// Codex P2 (PR #2592): the model-layer sampling path must publish the
// sampling lifecycle like the upstream path does -- sample.opened
// BEFORE the wait (a sample.opened hook answerer can only answer a
// question it hears about) and sample.answered after it.
func TestBrokerSamplerPublishesSampleLifecycle(t *testing.T) {
	bus := events.New(0)
	dir := t.TempDir()
	cfg := &config.Config{MCPServers: map[string]*config.Server{}}
	reg, err := NewRegistry(cfg, Paths{State: dir, Cache: dir}, nil)
	if err != nil {
		t.Fatal(err)
	}
	broker, err := OpenBroker(Paths{State: filepath.Join(dir, "state")})
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { _ = broker.Close() })
	reg.InstallHooks(bus, broker, nil)
	sub, _ := bus.Subscribe(events.Filter{Kinds: []string{"sample"}}, 0)
	defer sub.Close()

	s := &Server{reg: reg}
	pol := defaultConsumerPolicy()
	pol.ModelTimeout = 5 * time.Second
	ctx := withPolicy(context.Background(), pol)
	fn := s.brokerSampler("sess-1")
	type outcome struct {
		resp model.Response
		err  error
	}
	done := make(chan outcome, 1)
	go func() {
		resp, err := fn(ctx, model.Request{Messages: []model.Message{{Role: "user", Content: "hi"}}})
		done <- outcome{resp, err}
	}()

	recv := func(want events.Kind) events.Event {
		t.Helper()
		select {
		case e := <-sub.C:
			if e.Kind != want {
				t.Fatalf("got event %s, want %s", e.Kind, want)
			}
			return e
		case <-time.After(5 * time.Second):
			t.Fatalf("no %s event arrived", want)
		}
		return events.Event{}
	}
	// The opened event must arrive while the question is still
	// unanswered -- that ordering is the whole finding.
	recv(events.SampleOpened)
	pending, err := reg.broker.Pending(elicit.Filter{Session: "sess-1"})
	if err != nil {
		t.Fatal(err)
	}
	if len(pending) != 1 {
		t.Fatalf("pending = %d, want 1", len(pending))
	}
	err = reg.broker.Respond(elicit.Answer{
		ID:      pending[0].ID,
		Action:  elicit.Accept,
		By:      "tester",
		Content: json.RawMessage(`{"model":"m1","role":"assistant","content":{"type":"text","text":"hi back"}}`),
	})
	if err != nil {
		t.Fatal(err)
	}
	recv(events.SampleAnswered)
	out := <-done
	if out.err != nil {
		t.Fatalf("sampler: %v", out.err)
	}
	if out.resp.Text != "hi back" {
		t.Errorf("resp = %+v, want the answered text", out.resp)
	}
}
