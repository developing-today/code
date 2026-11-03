// Package model is the thin provider layer mcpx owns (#1925, approach C).
//
// mcpx has exactly one model-shaped need: a one-shot generation -- expand a
// query, write a script from an intent, draft a repair. It does not need an
// agent loop, and this package is deliberately not one: a Provider is a
// single function from a request to a response, with backends behind it.
//
// Backends in this slice: an OpenAI-compatible configured provider (any
// base URL, so OpenAI, LiteLLM, Ollama and LM Studio all work through it)
// and sampling through the broker (the pre-existing path, kept as a
// backend rather than replaced). The opencode-plugin backend is a later
// slice; it depends on #47.
//
// The scope guard from #1925 binds every caller: no mcpx operation may
// call a model on a path where a deterministic answer exists, and on a
// provider error the caller falls back to the deterministic result plus a
// note that the model was unavailable. Nothing in this package decides
// when a model is asked; that stays with the caller.
package model

import (
	"context"
	"encoding/json"
	"errors"
	"strings"
)

// Message is one turn of a generation request. Role is "system", "user" or
// "assistant", following the chat-completions convention every backend
// here maps onto.
type Message struct {
	Role    string `json:"role"`
	Content string `json:"content"`
}

// Request is one one-shot generation.
type Request struct {
	// Model is the model id, in the provider/model namespace (#1925):
	// "openai/gpt-5", "ollama/qwen3:8b". A bare id is read as belonging
	// to the provider the request was routed to.
	Model string
	// Messages is the conversation so far: at most a system prompt and a
	// user prompt for every current caller.
	Messages []Message
	// MaxTokens bounds the answer. Zero means the backend's own bound.
	MaxTokens int
	// Stop sequences, when the backend honours them.
	Stop []string
	// Schema, when set, is a JSON Schema the answer must conform to. It
	// is passed to backends that support structured output and ignored
	// by the rest.
	Schema json.RawMessage
}

// Response is what a provider answered.
type Response struct {
	Text string
	// Model is the model that actually answered, as the backend reports
	// it; it may differ from the requested id.
	Model string
	// Provider is the answering backend's name.
	Provider string
}

// Provider is the one function this layer exists to provide.
type Provider interface {
	Name() string
	Generate(ctx context.Context, req Request) (Response, error)
}

// ErrUnavailable means no provider could answer: none configured, the
// configured one unreachable, or the backend named not built yet. Callers
// treat it like any other provider error -- deterministic result, plus a
// note -- but tests and logs can tell it apart from a provider that was
// reached and failed.
var ErrUnavailable = errors.New("model: no provider available")

// SplitModelID splits the provider/model namespace: the provider is the
// segment before the first slash, the model is the rest. A bare id has an
// empty provider.
func SplitModelID(id string) (provider, model string) {
	if i := strings.Index(id, "/"); i > 0 {
		return id[:i], id[i+1:]
	}
	return "", id
}

// ModelID is the bare model id with any provider prefix removed.
func ModelID(id string) string {
	_, m := SplitModelID(id)
	return m
}
