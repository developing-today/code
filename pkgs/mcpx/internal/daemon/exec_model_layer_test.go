package daemon

import (
	"context"
	"io"
	"net/http"
	"net/http/httptest"
	"testing"
	"time"

	"github.com/developing-today/code/pkgs/mcpx/internal/execsvc"
)

// Codex P2 (PR #9021): every execution entry point must resolve the
// model layer onto its exec service — /v1/ask (kind: exec) and
// script-backed tools built theirs bare, so config-declared prompt
// hooks there always reported no provider configured. The shared
// setup is Server.modelLayerForExec; these tests pin what it stamps.

func TestModelLayerForExecStampsCapAndProvider(t *testing.T) {
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.Header().Set("Content-Type", "application/json")
		_, _ = io.WriteString(w, `{"model":"test-model","choices":[{"message":{"role":"assistant","content":"ok"}}]}`)
	}))
	defer srv.Close()
	s := intentTestServer()
	pol := defaultConsumerPolicy()
	pol.ModelProvider = "openai"
	pol.ModelBaseURL = srv.URL
	pol.ModelDefault = "openai/test-model"
	pol.ModelTimeout = 5 * time.Second
	pol.PromptMaxTokens = 100
	pol.ModelBudget = 40
	ctx := withPolicy(context.Background(), pol)

	svc := &execsvc.Service{}
	s.modelLayerForExec(ctx, svc, "sess-1")
	if svc.ModelProvider == nil {
		t.Fatal("a configured layer must reach the exec service")
	}
	if svc.ModelProviderErr != nil {
		t.Fatalf("unexpected build error: %v", svc.ModelProviderErr)
	}
	// prompt.maxTokens 100 capped by model.budget 40 — the same
	// modelMaxTokens rule the generation paths use.
	if svc.ModelMaxTokens != 40 {
		t.Fatalf("ModelMaxTokens = %d, want 40", svc.ModelMaxTokens)
	}
	if svc.ModelDefault != "openai/test-model" {
		t.Fatalf("ModelDefault = %q, want the policy's model.default", svc.ModelDefault)
	}
}

func TestModelLayerForExecUnbuildableLayerNamesItself(t *testing.T) {
	s := intentTestServer()
	pol := defaultConsumerPolicy()
	pol.ModelProvider = "plugin" // configured, not built yet
	ctx := withPolicy(context.Background(), pol)

	svc := &execsvc.Service{}
	s.modelLayerForExec(ctx, svc, "sess-1")
	if svc.ModelProvider != nil {
		t.Fatal("an unbuildable layer must not substitute another provider")
	}
	if svc.ModelProviderErr == nil {
		t.Fatal("the build error must be kept so a prompt hook's frame can name it")
	}
}

func TestModelLayerForExecNoLayerStaysBare(t *testing.T) {
	s := intentTestServer()
	pol := defaultConsumerPolicy()
	pol.ModelProvider = "none"
	ctx := withPolicy(context.Background(), pol)

	svc := &execsvc.Service{}
	s.modelLayerForExec(ctx, svc, "sess-1")
	if svc.ModelProvider != nil || svc.ModelProviderErr != nil {
		t.Fatalf("no layer configured must stay (nil, nil): %+v", svc)
	}
	if svc.ModelMaxTokens != modelMaxTokens(pol.PromptMaxTokens, pol.ModelBudget) {
		t.Fatalf("ModelMaxTokens = %d, want the policy rule's %d",
			svc.ModelMaxTokens, modelMaxTokens(pol.PromptMaxTokens, pol.ModelBudget))
	}
}
