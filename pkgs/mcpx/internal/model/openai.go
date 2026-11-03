package model

import (
	"bytes"
	"context"
	"encoding/json"
	"fmt"
	"io"
	"net/http"
	"strings"
	"time"

	"github.com/developing-today/code/pkgs/mcpx/internal/defaults"
)

// OpenAIProvider is the one configured-provider backend of this slice: an
// OpenAI-compatible chat-completions client. The base URL is configurable,
// so the same code serves OpenAI itself, a company gateway, LiteLLM,
// Ollama and LM Studio -- the proxy reading of #1925's "proxy" bullet is
// this setting, not a second implementation.
type OpenAIProvider struct {
	name    string
	baseURL string
	apiKey  string
	client  *http.Client
	// defaultModel is model.default for this provider (bare id, prefix
	// stripped), set by Resolve. It is the only fallback Generate may
	// use: an API backend has no default of its own to fall back to.
	defaultModel string
}

// NewOpenAIProvider builds the backend. name is the provider's name in the
// provider/model namespace ("openai", "anthropic", "google", or whatever a
// model.baseURL configuration calls itself). An empty timeout gets a
// conservative default; the daemon normally passes model.timeout.
func NewOpenAIProvider(name, baseURL, apiKey string, timeout time.Duration) *OpenAIProvider {
	if timeout <= 0 {
		timeout = defaults.ModelTimeout
	}
	return &OpenAIProvider{
		name:    name,
		baseURL: strings.TrimRight(baseURL, "/"),
		apiKey:  apiKey,
		client:  &http.Client{Timeout: timeout},
	}
}

func (p *OpenAIProvider) Name() string { return p.name }

// tokenLimitParam names the request field that bounds the answer for one
// model. Most chat models take max_tokens, but OpenAI's o-series and
// GPT-5 models reject that field outright -- every request to them would
// fail and the caller would silently fall back to its deterministic
// answer -- so they are sent max_completion_tokens instead (Codex
// review, PR #2592). Other OpenAI-compatible endpoints keep max_tokens,
// the field they have always accepted.
func tokenLimitParam(modelID string) string {
	id := strings.ToLower(modelID)
	for _, prefix := range []string{"o1", "o3", "o4", "gpt-5"} {
		if strings.HasPrefix(id, prefix) {
			return "max_completion_tokens"
		}
	}
	return "max_tokens"
}

type chatMessage struct {
	Role    string `json:"role"`
	Content string `json:"content"`
}

func (p *OpenAIProvider) Generate(ctx context.Context, req Request) (Response, error) {
	modelID := ModelID(req.Model)
	if modelID == "" {
		modelID = p.defaultModel
	}
	if modelID == "" {
		// Never invent an id: sending the literal "default" (the old
		// behaviour) makes the endpoint reject a configuration the
		// settings describe as valid, and the caller cannot tell the
		// failure from a real answer attempt. Naming no model is a
		// configuration gap, reported as unavailable so callers fall
		// back deterministically (Codex review, PR #2592).
		return Response{}, fmt.Errorf("model: provider %s: no model named (set model.default or name a model in the request): %w", p.name, ErrUnavailable)
	}
	payload := map[string]any{
		"model":    modelID,
		"messages": chatMessages(req.Messages),
	}
	if req.MaxTokens > 0 {
		payload[tokenLimitParam(modelID)] = req.MaxTokens
	}
	if len(req.Stop) > 0 {
		payload["stop"] = req.Stop
	}
	if len(req.Schema) > 0 {
		payload["response_format"] = map[string]any{
			"type": "json_schema",
			"json_schema": map[string]any{
				"name":   "result",
				"schema": json.RawMessage(req.Schema),
			},
		}
	}
	body, err := json.Marshal(payload)
	if err != nil {
		return Response{}, err
	}
	url := p.baseURL + "/chat/completions"
	httpReq, err := http.NewRequestWithContext(ctx, http.MethodPost, url, bytes.NewReader(body))
	if err != nil {
		return Response{}, err
	}
	httpReq.Header.Set("Content-Type", "application/json")
	if p.apiKey != "" {
		httpReq.Header.Set("Authorization", "Bearer "+p.apiKey)
	}
	resp, err := p.client.Do(httpReq)
	if err != nil {
		return Response{}, fmt.Errorf("model: provider %s request failed: %w", p.name, err)
	}
	defer resp.Body.Close()
	respBody, err := io.ReadAll(resp.Body)
	if err != nil {
		return Response{}, err
	}
	if resp.StatusCode < 200 || resp.StatusCode >= 300 {
		return Response{}, fmt.Errorf("model: provider %s returned HTTP %d: %s",
			p.name, resp.StatusCode, strings.TrimSpace(string(respBody)))
	}
	var parsed struct {
		Model   string `json:"model"`
		Choices []struct {
			Message struct {
				Content string `json:"content"`
			} `json:"message"`
		} `json:"choices"`
	}
	if err := json.Unmarshal(respBody, &parsed); err != nil {
		return Response{}, fmt.Errorf("model: provider %s invalid JSON response: %w", p.name, err)
	}
	if len(parsed.Choices) == 0 {
		return Response{}, fmt.Errorf("model: provider %s returned no choices", p.name)
	}
	return Response{
		Text:     parsed.Choices[0].Message.Content,
		Model:    parsed.Model,
		Provider: p.name,
	}, nil
}

func chatMessages(ms []Message) []chatMessage {
	out := make([]chatMessage, 0, len(ms))
	for _, m := range ms {
		role := m.Role
		if role == "" {
			role = "user"
		}
		out = append(out, chatMessage{Role: role, Content: m.Content})
	}
	return out
}
