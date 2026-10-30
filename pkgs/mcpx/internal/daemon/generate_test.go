package daemon

import (
	"context"
	"errors"
	"strings"
	"testing"

	"github.com/dezren39/mcpx/internal/scripting"
)

// #1961: a caller-supplied provider/model layers over the daemon's own
// repair.* configuration; empty caller values leave it untouched.
func TestCallerProviderConfig(t *testing.T) {
	base := scripting.ProviderConfig{Type: "claude", Model: "daemon-model", BaseURL: "http://localhost:1"}

	got := callerProviderConfig(base, "opencode", "caller-model")
	if got.Type != "opencode" || got.Model != "caller-model" {
		t.Errorf("override = %+v, want type/model replaced", got)
	}
	if got.BaseURL != base.BaseURL {
		t.Errorf("base URL changed: %+v", got)
	}

	got = callerProviderConfig(base, "", "")
	if got != base {
		t.Errorf("empty override = %+v, want base %+v", got, base)
	}
}

func intentTestServer() *Server {
	reg := &Registry{
		testTools: []ToolInfo{
			{
				Namespace:   "weather",
				Tool:        "forecast",
				Function:    "weather.forecast",
				Description: "Get weather forecast for a city",
				InputSchema: []byte(`{"type":"object","required":["city"],"properties":{"city":{"type":"string"}}}`),
			},
		},
	}
	return &Server{reg: reg}
}

func intentCtx() context.Context {
	return withPolicy(context.Background(), defaultConsumerPolicy())
}

// #1961: with no provider, generate returns errNoModel. It must not fall
// back to an MCP sampling request (the registry here has no broker at
// all, and the old code path would have opened one on it).
func TestGenerateNoProvider(t *testing.T) {
	s := intentTestServer()
	_, _, err := s.generate(intentCtx(), "get chicago weather", nil, "", "")
	if !errors.Is(err, errNoModel) {
		t.Fatalf("err = %v, want errNoModel", err)
	}
}

func TestGenerateProviderAnswers(t *testing.T) {
	s := intentTestServer()
	prov := scripting.NewCallbackProvider("mock-ai", func(ctx context.Context, req scripting.CompletionRequest) (string, error) {
		if !strings.Contains(req.UserPrompt, "chicago") {
			t.Errorf("user prompt = %q", req.UserPrompt)
		}
		return "const data = await tools.weather.forecast({city: 'Chicago'});\nreturn data;", nil
	})
	source, used, err := s.generate(intentCtx(), "get chicago weather", prov, "mock-model", "")
	if err != nil {
		t.Fatalf("generate: %v", err)
	}
	if !strings.Contains(source, "tools.weather.forecast") {
		t.Errorf("source = %q", source)
	}
	if used != "provider:mock-ai/mock-model" {
		t.Errorf("used = %q, want provider:mock-ai/mock-model", used)
	}
}

func TestGenerateProviderFailureIsNoModel(t *testing.T) {
	s := intentTestServer()
	prov := scripting.NewCallbackProvider("down", func(ctx context.Context, req scripting.CompletionRequest) (string, error) {
		return "", errors.New("connection refused")
	})
	_, _, err := s.generate(intentCtx(), "get chicago weather", prov, "", "")
	if !errors.Is(err, errNoModel) {
		t.Fatalf("err = %v, want errNoModel", err)
	}
}
