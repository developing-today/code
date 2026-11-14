package mcpserver_test

import (
	"context"
	"encoding/json"
	"fmt"
	"testing"

	"github.com/developing-today/code/pkgs/mcpx/internal/mcpserver"
)

// #2435: an error mcpx mints itself carries the local-origin marker,
// merged into whatever data the error already has.

// errReadBackend fails every resource read, so the answer is mcpx's
// own notFound, whose data is the URI.
type errReadBackend struct {
	*fakeBackend
}

func (errReadBackend) ReadResource(context.Context, string) ([]mcpserver.ResourceContents, error) {
	return nil, fmt.Errorf("%w: no such resource", mcpserver.ErrResourceNotFound)
}

func TestALocallyMintedErrorCarriesTheLocalOriginMarker(t *testing.T) {
	s := mcpserver.New(&errReadBackend{fakeBackend: newBackend()}, "mcpx", "test")
	w := post(s, frame(7, "resources/read",
		modernWith(`{}`, map[string]any{"uri": "file:///no-such-resource"})),
		modernHeaders("resources/read", "file:///no-such-resource"))
	var resp struct {
		Error *struct {
			Code    int            `json:"code"`
			Message string         `json:"message"`
			Data    map[string]any `json:"data"`
		} `json:"error"`
	}
	if err := json.Unmarshal(w.Body.Bytes(), &resp); err != nil {
		t.Fatalf("response is not JSON: %v (%s)", err, w.Body.String())
	}
	if resp.Error == nil {
		t.Fatalf("expected an error response, got %s", w.Body.String())
	}
	if resp.Error.Data["uri"] != "file:///no-such-resource" {
		t.Fatalf("the error's own data was lost: %v", resp.Error.Data)
	}
	mcpx, ok := resp.Error.Data["mcpx"].(map[string]any)
	if !ok || mcpx["origin"] != "local" {
		t.Fatalf("locally minted error lacks the origin marker: data = %v", resp.Error.Data)
	}
}

// The marker is also stamped when the error carries no data at all: a
// prompts/get the backend refuses with ErrInvalidParams is classified
// and answered by mcpx itself.
func TestALocallyMintedErrorWithoutDataIsMarkedToo(t *testing.T) {
	s := mcpserver.New(newBackend(), "mcpx", "test")
	w := post(s, frame(8, "prompts/get",
		modernWith(`{}`, map[string]any{"name": "nosuch"})),
		modernHeaders("prompts/get", "nosuch"))
	var resp struct {
		Error *struct {
			Code int            `json:"code"`
			Data map[string]any `json:"data"`
		} `json:"error"`
	}
	if err := json.Unmarshal(w.Body.Bytes(), &resp); err != nil {
		t.Fatalf("response is not JSON: %v (%s)", err, w.Body.String())
	}
	if resp.Error == nil {
		t.Fatalf("expected an error response, got %s", w.Body.String())
	}
	mcpx, ok := resp.Error.Data["mcpx"].(map[string]any)
	if !ok || mcpx["origin"] != "local" {
		t.Fatalf("locally minted error lacks the origin marker: data = %v", resp.Error.Data)
	}
}

// #2435: a peer's error is relayed untouched -- code, message and data
// exactly as the upstream sent them, and never marked local.
func TestARelayedPeerErrorIsNeverMarkedLocal(t *testing.T) {
	b := newRelay()
	b.err = &mcpserver.UpstreamError{Code: -32602, Message: "no tool named \"greet\"",
		Data: json.RawMessage(`{"peer":"data"}`)}
	s := mcpserver.New(b, "mcpx", "test")
	s.Passthrough = []string{"up"}

	w := post(s, frame(9, "tools/call", modernWith(`{"elicitation":{}}`,
		map[string]any{"name": "greet", "arguments": map[string]any{}})), modernHeaders("tools/call", "greet"))
	var resp struct {
		Error *struct {
			Code    int             `json:"code"`
			Message string          `json:"message"`
			Data    json.RawMessage `json:"data"`
		} `json:"error"`
	}
	if err := json.Unmarshal(w.Body.Bytes(), &resp); err != nil {
		t.Fatalf("response is not JSON: %v (%s)", err, w.Body.String())
	}
	if resp.Error == nil {
		t.Fatalf("expected the relayed error, got %s", w.Body.String())
	}
	if resp.Error.Code != -32602 || resp.Error.Message != `no tool named "greet"` {
		t.Fatalf("relayed error altered: %+v", resp.Error)
	}
	if string(resp.Error.Data) != `{"peer":"data"}` {
		t.Fatalf("relayed data altered or marked: %s", resp.Error.Data)
	}
}
