package model

import (
	"context"
	"encoding/json"
	"errors"
	"io"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
	"time"
)

func TestSplitModelID(t *testing.T) {
	for _, c := range []struct{ in, provider, model string }{
		{"openai/gpt-5", "openai", "gpt-5"},
		{"ollama/qwen3:8b", "ollama", "qwen3:8b"},
		{"gpt-5", "", "gpt-5"},
		{"openai/org/model", "openai", "org/model"},
	} {
		p, m := SplitModelID(c.in)
		if p != c.provider || m != c.model {
			t.Errorf("SplitModelID(%q) = (%q, %q), want (%q, %q)", c.in, p, m, c.provider, c.model)
		}
	}
}

func TestOpenAIProviderSendsChatCompletion(t *testing.T) {
	var gotAuth, gotPath string
	var gotBody map[string]any
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		gotAuth = r.Header.Get("Authorization")
		gotPath = r.URL.Path
		b, _ := io.ReadAll(r.Body)
		_ = json.Unmarshal(b, &gotBody)
		w.Header().Set("Content-Type", "application/json")
		_, _ = io.WriteString(w, `{"model":"gpt-5","choices":[{"message":{"role":"assistant","content":"hello"}}]}`)
	}))
	defer srv.Close()

	p := NewOpenAIProvider("openai", srv.URL+"/v1", "secret-key", time.Second)
	resp, err := p.Generate(context.Background(), Request{
		Model:     "openai/gpt-5",
		Messages:  []Message{{Role: "system", Content: "sys"}, {Role: "user", Content: "hi"}},
		MaxTokens: 42,
		Stop:      []string{"END"},
		Schema:    json.RawMessage(`{"type":"object"}`),
	})
	if err != nil {
		t.Fatal(err)
	}
	if resp.Text != "hello" || resp.Model != "gpt-5" || resp.Provider != "openai" {
		t.Errorf("resp = %+v", resp)
	}
	if gotPath != "/v1/chat/completions" {
		t.Errorf("path = %q", gotPath)
	}
	if gotAuth != "Bearer secret-key" {
		t.Errorf("auth = %q", gotAuth)
	}
	if gotBody["model"] != "gpt-5" {
		t.Errorf("model sent = %v (provider prefix must be stripped)", gotBody["model"])
	}
	if gotBody["max_tokens"] != float64(42) {
		t.Errorf("max_tokens = %v", gotBody["max_tokens"])
	}
	rf, ok := gotBody["response_format"].(map[string]any)
	if !ok || rf["type"] != "json_schema" {
		t.Errorf("response_format = %v (schema must be passed through)", gotBody["response_format"])
	}
}

func TestOpenAIProviderHTTPError(t *testing.T) {
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.WriteHeader(http.StatusTooManyRequests)
		_, _ = io.WriteString(w, "slow down")
	}))
	defer srv.Close()
	p := NewOpenAIProvider("openai", srv.URL, "", time.Second)
	_, err := p.Generate(context.Background(), Request{Messages: []Message{{Content: "hi"}}})
	if err == nil || !strings.Contains(err.Error(), "429") {
		t.Fatalf("err = %v, want an HTTP 429 error the caller can fall back on", err)
	}
}

func TestSamplingProvider(t *testing.T) {
	p := NewSamplingProvider(nil)
	if _, err := p.Generate(context.Background(), Request{}); !errors.Is(err, ErrUnavailable) {
		t.Errorf("nil sampler: err = %v, want ErrUnavailable", err)
	}
	p = NewSamplingProvider(func(ctx context.Context, req Request) (Response, error) {
		return Response{Text: "sampled"}, nil
	})
	resp, err := p.Generate(context.Background(), Request{})
	if err != nil || resp.Text != "sampled" || resp.Provider != "sampling" {
		t.Errorf("resp = %+v, err = %v", resp, err)
	}
}

func TestResolveNoneIsNoProviderNoError(t *testing.T) {
	for _, name := range []string{"", "none", "NONE"} {
		p, err := Resolve(Config{Provider: name})
		if p != nil || err != nil {
			t.Errorf("Resolve(%q) = (%v, %v), want (nil, nil): configuring nothing must change nothing", name, p, err)
		}
	}
}

func TestResolveConfiguredProviderHonoursBaseURLAndKeyEnv(t *testing.T) {
	env := map[string]string{"MY_KEY": "k-1"}
	cfg := Config{
		Provider:  "openai",
		BaseURL:   "http://localhost:11434/v1",
		APIKeyEnv: "MY_KEY",
		LookupEnv: func(k string) string { return env[k] },
	}
	p, err := Resolve(cfg)
	if err != nil {
		t.Fatal(err)
	}
	op, ok := p.(*OpenAIProvider)
	if !ok {
		t.Fatalf("provider = %T, want *OpenAIProvider", p)
	}
	if op.baseURL != "http://localhost:11434/v1" || op.apiKey != "k-1" {
		t.Errorf("baseURL = %q, apiKey set = %v", op.baseURL, op.apiKey != "")
	}
}

func TestResolveAuto(t *testing.T) {
	// Nothing configured and no sampler: nil, nil -- auto fails cleanly.
	p, err := Resolve(Config{Provider: "auto", LookupEnv: func(string) string { return "" }})
	if p != nil || err != nil {
		t.Errorf("bare auto = (%v, %v), want (nil, nil)", p, err)
	}
	// A sampler wins over a configured key.
	p, err = Resolve(Config{
		Provider:  "auto",
		APIKeyEnv: "K",
		LookupEnv: func(string) string { return "v" },
		Sampler:   func(ctx context.Context, req Request) (Response, error) { return Response{}, nil },
	})
	if err != nil || p.Name() != "sampling" {
		t.Errorf("auto with sampler = (%v, %v), want the sampling backend", p, err)
	}
	// No sampler, but a named key that is set: the configured backend.
	p, err = Resolve(Config{
		Provider:  "auto",
		APIKeyEnv: "K",
		LookupEnv: func(string) string { return "v" },
	})
	if err != nil || p.Name() != "openai" {
		t.Errorf("auto with key = (%v, %v), want the openai backend", p, err)
	}
	// apiKeyEnv naming an unset variable is not configuration.
	p, err = Resolve(Config{
		Provider:  "auto",
		APIKeyEnv: "MISSING",
		LookupEnv: func(string) string { return "" },
	})
	if p != nil || err != nil {
		t.Errorf("auto with unset key env = (%v, %v), want (nil, nil)", p, err)
	}
}

func TestResolvePluginAndSamplingWithoutPath(t *testing.T) {
	if _, err := Resolve(Config{Provider: "plugin"}); !errors.Is(err, ErrUnavailable) {
		t.Errorf("plugin: err = %v, want ErrUnavailable (backend is a later slice, #47)", err)
	}
	if _, err := Resolve(Config{Provider: "sampling"}); !errors.Is(err, ErrUnavailable) {
		t.Errorf("sampling without a sampler: err = %v, want ErrUnavailable", err)
	}
	if _, err := Resolve(Config{Provider: "bogus"}); err == nil {
		t.Error("unknown provider must error, not silently resolve")
	}
}
