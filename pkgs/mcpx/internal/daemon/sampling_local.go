package daemon

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"strings"

	"github.com/developing-today/code/pkgs/mcpx/internal/elicit"
	"github.com/developing-today/code/pkgs/mcpx/internal/mcpclient"
	"github.com/developing-today/code/pkgs/mcpx/internal/model"
)

// sampling.answerLocally (#1925, slice 3): answer an upstream server's
// sampling/createMessage request from mcpx's own configured provider
// layer instead of relaying it through the broker to the caller.
//
// The relay stays the default (the setting is off) and is untouched when
// it is off: answerServer only diverts here when the setting is on. When
// it is on, the request is answered by internal/model Resolve over the
// daemon's model.* policy -- provider or profile -- with two deliberate
// differences from the consumer generation path:
//
//   - Sampler is nil. Resolving the sampling backend here would relay
//     the request to the caller, which is the path this setting exists
//     to replace; a configuration that names only sampling therefore
//     resolves to unavailable, honestly.
//   - There is no broker fallback. Enabled with no provider configured,
//     or with a provider that cannot answer, the request fails cleanly
//     and deterministically (an internal error naming the cause) rather
//     than hanging on a broker question nobody will answer or silently
//     succeeding with an invented answer.
//
// Bounds: the request's own maxTokens, capped by model.budget, and
// model.timeout as the deadline -- the same bounds the model layer's
// other callers get (modelMaxTokens, Server.modelLayer).
func (r *Registry) sampleLocally(ctx context.Context, server, key string, params json.RawMessage) (any, error) {
	pol := r.consumer

	// The request is checked against the era the upstream negotiated
	// before anything is spent on it, exactly as the broker path checks
	// it: a malformed sampling request is the server's mistake (-32602),
	// not a generation to attempt.
	era, known := mcpclient.SamplingEraFrom(ctx)
	arrayOK := !known || era.Negotiated == "" || era.Negotiated >= "2025-11-25"
	if err := elicit.ValidateSampleRequest(params, arrayOK); err != nil {
		return nil, &elicit.SamplingInvalidParams{Err: err}
	}

	if !modelLayerConfigured(pol) {
		return nil, mcpclient.InternalError("sampling.answerLocally is set but no model provider is configured " +
			"(set model.provider, model.default or model.profiles); the request was not relayed")
	}
	prov, err := model.Resolve(model.Config{
		Provider:             pol.ModelProvider,
		BaseURL:              pol.ModelBaseURL,
		APIKeyEnv:            pol.ModelAPIKeyEnv,
		ProviderSettingsJSON: pol.ModelProviders,
		ProfilesJSON:         pol.ModelProfiles,
		DefaultModel:         pol.ModelDefault,
		Timeout:              pol.ModelTimeout,
		// Sampler deliberately nil: see the package comment above.
	})
	if err != nil {
		return nil, mcpclient.InternalError("sampling.answerLocally: " + err.Error())
	}
	if prov == nil {
		return nil, mcpclient.InternalError("sampling.answerLocally is set but model.provider resolved to no provider; " +
			"the request was not relayed")
	}

	req, err := samplingLocalRequest(params, pol)
	if err != nil {
		return nil, &elicit.SamplingInvalidParams{Err: err}
	}

	if pol.ModelTimeout > 0 {
		var cancel context.CancelFunc
		ctx, cancel = context.WithTimeout(ctx, pol.ModelTimeout)
		defer cancel()
	}
	resp, err := prov.Generate(ctx, req)
	if err != nil {
		return nil, mcpclient.InternalError("sampling.answerLocally: provider " + prov.Name() + " did not answer: " + err.Error())
	}
	if strings.TrimSpace(resp.Text) == "" {
		return nil, mcpclient.InternalError("sampling.answerLocally: provider " + prov.Name() + " answered with no text")
	}

	modelName := resp.Model
	if modelName == "" {
		modelName = model.ModelID(req.Model)
	}
	if modelName == "" {
		modelName = model.ModelID(pol.ModelDefault)
	}
	result := map[string]any{
		"role":       "assistant",
		"model":      modelName,
		"content":    map[string]any{"type": "text", "text": resp.Text},
		"stopReason": "endTurn",
	}
	// The answer is held to the same shape rules as a brokered one: a
	// local answer that is not a CreateMessageResult is refused upstream
	// as -32602 rather than relayed in a shape nobody sent.
	raw, err := json.Marshal(result)
	if err != nil {
		return nil, err
	}
	if err := elicit.ValidateSampleResult(raw, arrayOK, false); err != nil {
		return nil, &elicit.SamplingInvalidParams{Err: err}
	}
	return result, nil
}

// samplingLocalRequest converts a sampling CreateMessageRequest into a
// model.Request for the provider layer.
//
// The layer is text-only (model.Message carries a string), so the
// conversion is honest about what it cannot carry: a request with tools
// (whose answer would be tool_use blocks a text generation cannot
// produce) or with non-text content blocks fails as invalid params for
// this path rather than being silently flattened. Model preference
// hints are honoured the way the specification allows them to be used:
// the first hint names the model when one is given; otherwise
// model.default supplies it through Resolve/Generate as usual.
func samplingLocalRequest(params json.RawMessage, pol ConsumerPolicy) (model.Request, error) {
	var p struct {
		Messages []struct {
			Role    string          `json:"role"`
			Content json.RawMessage `json:"content"`
		} `json:"messages"`
		SystemPrompt     string `json:"systemPrompt"`
		MaxTokens        int    `json:"maxTokens"`
		ModelPreferences struct {
			Hints []struct {
				Name string `json:"name"`
			} `json:"hints"`
		} `json:"modelPreferences"`
		Tools json.RawMessage `json:"tools"`
	}
	if err := json.Unmarshal(params, &p); err != nil {
		return model.Request{}, fmt.Errorf("a CreateMessageRequest must be an object: %w", err)
	}
	if len(p.Tools) > 0 && string(p.Tools) != "null" && string(p.Tools) != "[]" {
		return model.Request{}, errors.New("the request carries tools, which a local text generation cannot answer")
	}

	req := model.Request{MaxTokens: modelMaxTokens(p.MaxTokens, pol.ModelBudget)}
	if p.SystemPrompt != "" {
		req.Messages = append(req.Messages, model.Message{Role: "system", Content: p.SystemPrompt})
	}
	for i, m := range p.Messages {
		text, err := samplingTextContent(m.Content)
		if err != nil {
			return model.Request{}, fmt.Errorf("message %d: %w", i, err)
		}
		role := m.Role
		if role == "" {
			role = "user"
		}
		req.Messages = append(req.Messages, model.Message{Role: role, Content: text})
	}
	if len(p.ModelPreferences.Hints) > 0 && p.ModelPreferences.Hints[0].Name != "" {
		req.Model = p.ModelPreferences.Hints[0].Name
	}
	return req, nil
}

// samplingTextContent extracts the text of one sampling message's
// content: a single block or a list of blocks (both eras' shapes are
// accepted here; the era's own rule was already enforced by
// ValidateSampleRequest). Every block must be a text block.
func samplingTextContent(raw json.RawMessage) (string, error) {
	b := strings.TrimSpace(string(raw))
	if b == "" || b == "null" {
		return "", errors.New("content is missing")
	}
	var blocks []json.RawMessage
	if b[0] == '[' {
		if err := json.Unmarshal(raw, &blocks); err != nil {
			return "", fmt.Errorf("content is not a list of content blocks: %w", err)
		}
	} else {
		blocks = []json.RawMessage{raw}
	}
	var texts []string
	for _, blk := range blocks {
		var block struct {
			Type string `json:"type"`
			Text string `json:"text"`
		}
		if err := json.Unmarshal(blk, &block); err != nil {
			return "", fmt.Errorf("content is not a content block: %w", err)
		}
		if block.Type != "text" {
			return "", fmt.Errorf("a %q content block cannot be answered by a local text generation", block.Type)
		}
		texts = append(texts, block.Text)
	}
	return strings.Join(texts, "\n"), nil
}
