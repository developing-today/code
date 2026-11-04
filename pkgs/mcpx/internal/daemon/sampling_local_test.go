package daemon

import (
	"context"
	"encoding/json"
	"io"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
	"time"
)

// sampling.answerLocally (#1925 slice 3): with the setting on, an
// upstream sampling request is answered by the configured model layer;
// with it off, the broker relay path is untouched; with it on but no
// provider configured, the failure is clean and deterministic.

func TestSampleLocallyAnswersViaConfiguredProvider(t *testing.T) {
	var gotModel string
	var gotMessages []map[string]any
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.URL.Path != "/chat/completions" {
			t.Errorf("path = %s, want /chat/completions", r.URL.Path)
		}
		body, _ := io.ReadAll(r.Body)
		var payload struct {
			Model    string           `json:"model"`
			Messages []map[string]any `json:"messages"`
		}
		if err := json.Unmarshal(body, &payload); err != nil {
			t.Errorf("payload: %v", err)
		}
		gotModel = payload.Model
		gotMessages = payload.Messages
		w.Header().Set("Content-Type", "application/json")
		_, _ = io.WriteString(w, `{"model":"test-model","choices":[{"message":{"role":"assistant","content":"local answer"},"finish_reason":"stop"}]}`)
	}))
	defer srv.Close()

	r := &Registry{consumer: ConsumerPolicy{
		SamplingAnswerLocally: true,
		ModelProvider:         "openai",
		ModelBaseURL:          srv.URL,
		ModelDefault:          "openai/test-model",
		ModelTimeout:          5 * time.Second,
	}}
	out, err := r.answerServer(context.Background(), "demo", "key", "sampling/createMessage",
		json.RawMessage(`{"messages":[{"role":"user","content":{"type":"text","text":"hi"}}],"systemPrompt":"be terse","maxTokens":7}`))
	if err != nil {
		t.Fatalf("answerServer: %v", err)
	}
	result, ok := out.(map[string]any)
	if !ok {
		t.Fatalf("result type %T, want map", out)
	}
	if result["role"] != "assistant" || result["model"] != "test-model" || result["stopReason"] != "endTurn" {
		t.Errorf("result = %v", result)
	}
	content, ok := result["content"].(map[string]any)
	if !ok || content["type"] != "text" || content["text"] != "local answer" {
		t.Errorf("content = %v", result["content"])
	}
	if gotModel != "test-model" {
		t.Errorf("provider was asked for model %q, want test-model (model.default, prefix stripped)", gotModel)
	}
	if len(gotMessages) != 2 || gotMessages[0]["content"] != "be terse" || gotMessages[1]["content"] != "hi" {
		t.Errorf("provider messages = %v, want system prompt then the user message", gotMessages)
	}
}

func TestSampleLocallyEnabledWithoutProviderFailsCleanly(t *testing.T) {
	r := &Registry{consumer: ConsumerPolicy{SamplingAnswerLocally: true}}
	_, err := r.answerServer(context.Background(), "demo", "key", "sampling/createMessage",
		json.RawMessage(`{"messages":[{"role":"user","content":{"type":"text","text":"hi"}}],"maxTokens":7}`))
	if err == nil || !strings.Contains(err.Error(), "no model provider is configured") {
		t.Fatalf("err = %v, want the clean no-provider failure", err)
	}
}

func TestSampleLocallySamplingOnlyProviderFailsCleanly(t *testing.T) {
	// model.provider=sampling would relay to the caller -- the very path
	// answerLocally replaces -- so with no other provider configured the
	// local answer is unavailable, not a silent relay.
	r := &Registry{consumer: ConsumerPolicy{
		SamplingAnswerLocally: true,
		ModelProvider:         "sampling",
		ModelTimeout:          5 * time.Second,
	}}
	_, err := r.answerServer(context.Background(), "demo", "key", "sampling/createMessage",
		json.RawMessage(`{"messages":[{"role":"user","content":{"type":"text","text":"hi"}}],"maxTokens":7}`))
	if err == nil || !strings.Contains(err.Error(), "sampling.answerLocally") {
		t.Fatalf("err = %v, want a clean sampling.answerLocally failure", err)
	}
}

func TestSamplingRelayUnchangedWhenAnswerLocallyOff(t *testing.T) {
	// Setting off (the default): answerServer keeps the relay behaviour,
	// which with no broker is the long-standing no-broker error.
	r := &Registry{}
	_, err := r.answerServer(context.Background(), "demo", "key", "sampling/createMessage",
		json.RawMessage(`{"messages":[{"role":"user","content":{"type":"text","text":"hi"}}],"maxTokens":7}`))
	if err == nil || !strings.Contains(err.Error(), "no broker") {
		t.Fatalf("err = %v, want the pre-existing no-broker error", err)
	}
}

func TestSamplingLocalRequestConversion(t *testing.T) {
	pol := ConsumerPolicy{ModelBudget: 10}
	req, err := samplingLocalRequest(json.RawMessage(
		`{"messages":[{"role":"user","content":[{"type":"text","text":"a"},{"type":"text","text":"b"}]}],"maxTokens":100,"modelPreferences":{"hints":[{"name":"openai/hinted"}]}}`), pol)
	if err != nil {
		t.Fatalf("samplingLocalRequest: %v", err)
	}
	if req.Model != "openai/hinted" {
		t.Errorf("model = %q, want the first preference hint", req.Model)
	}
	if req.MaxTokens != 10 {
		t.Errorf("maxTokens = %d, want model.budget's cap of 10", req.MaxTokens)
	}
	if len(req.Messages) != 1 || req.Messages[0].Content != "a\nb" {
		t.Errorf("messages = %+v, want the two text blocks joined", req.Messages)
	}

	if _, err := samplingLocalRequest(json.RawMessage(
		`{"messages":[{"role":"user","content":{"type":"text","text":"hi"}}],"tools":[{"name":"x"}],"maxTokens":7}`), pol); err == nil {
		t.Error("a request carrying tools must fail rather than be flattened to text")
	}
	if _, err := samplingLocalRequest(json.RawMessage(
		`{"messages":[{"role":"user","content":{"type":"image","data":"AA==","mimeType":"image/png"}}],"maxTokens":7}`), pol); err == nil {
		t.Error("a non-text content block must fail rather than be dropped silently")
	}
}
