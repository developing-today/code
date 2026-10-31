package mcpserver_test

import (
	"context"
	"encoding/json"
	"testing"
	"time"

	"github.com/dezren39/mcpx/internal/mcpserver"
)

// #2080 (Phase 3 of #2077): server-side -32042 production. A 2025-11-25
// host that declared elicitation.url, whose call is blocked by a URL
// question it cannot be asked inline, is answered -32042 listing
// exactly the pending URL questions; every other configuration gets
// none.

type urlRequiredReply struct {
	Error *struct {
		Code    int    `json:"code"`
		Message string `json:"message"`
		Data    struct {
			Elicitations []struct {
				Mode          string `json:"mode"`
				URL           string `json:"url"`
				ElicitationID string `json:"elicitationId"`
				Message       string `json:"message"`
			} `json:"elicitations"`
		} `json:"data"`
	} `json:"error"`
	Result map[string]any `json:"result"`
}

func legacyInit(t *testing.T, s *mcpserver.Server, caps map[string]any) {
	t.Helper()
	resp := s.Handle(context.Background(), mcpserver.Request(0, "initialize", map[string]any{
		"protocolVersion": "2025-11-25",
		"capabilities":    caps,
		"clientInfo":      map[string]any{"name": "test", "version": "1"},
	}))
	if resp == nil || resp.Error != nil {
		t.Fatalf("initialize failed: %v", resp)
	}
}

func legacyCall(t *testing.T, s *mcpserver.Server) urlRequiredReply {
	t.Helper()
	resp := s.Handle(context.Background(), mcpserver.Request(1, "tools/call", map[string]any{
		"name":      "mcpx_call",
		"arguments": map[string]any{"namespace": "x", "tool": "y"},
	}))
	b, err := json.Marshal(resp)
	if err != nil {
		t.Fatal(err)
	}
	var out urlRequiredReply
	if err := json.Unmarshal(b, &out); err != nil {
		t.Fatalf("decode %s: %v", b, err)
	}
	return out
}

func urlQuestion(id string) mcpserver.Question {
	return mcpserver.Question{ID: id, Method: "elicitation/create", Mode: "url", Server: "up",
		Params: json.RawMessage(`{"mode":"url","message":"connect your account","url":"https://auth.example/connect?state=abc"}`)}
}

func TestURLRequiredWhenNoChannelToAskOn(t *testing.T) {
	asker := &scriptedAsker{questions: []mcpserver.Question{urlQuestion("u1")}, text: "done"}
	s := mcpserver.New(newBackend(), "mcpx", "test")
	s.Ask = asker
	// Declared form+url at 2025-11-25; the default connection has no
	// sender and no push, so nothing can be asked inline.
	legacyInit(t, s, map[string]any{"elicitation": map[string]any{"form": map[string]any{}, "url": map[string]any{}}})
	out := legacyCall(t, s)
	if out.Error == nil || out.Error.Code != -32042 {
		t.Fatalf("want -32042, got %+v", out)
	}
	els := out.Error.Data.Elicitations
	if len(els) != 1 {
		t.Fatalf("want exactly the one pending URL question, got %v", els)
	}
	if els[0].Mode != "url" || els[0].URL != "https://auth.example/connect?state=abc" {
		t.Errorf("elicitation not carried as given: %+v", els[0])
	}
	// The upstream sent no elicitationId; paramsFor synthesises mcpx-<id>.
	if els[0].ElicitationID != "mcpx-u1" {
		t.Errorf("elicitationId = %q, want mcpx-u1", els[0].ElicitationID)
	}
	if asker.abandoned != 0 {
		t.Errorf("the call must NOT be abandoned: the broker keeps the questions (abandoned %d)", asker.abandoned)
	}
}

func TestURLRequiredListsOnlyURLQuestions(t *testing.T) {
	form := mcpserver.Question{ID: "f1", Method: "elicitation/create", Mode: "form", Server: "up",
		Params: json.RawMessage(`{"message":"which repo?","requestedSchema":{"type":"object"}}`)}
	asker := &scriptedAsker{questions: []mcpserver.Question{form, urlQuestion("u2")}, text: "done"}
	s := mcpserver.New(newBackend(), "mcpx", "test")
	s.Ask = asker
	legacyInit(t, s, map[string]any{"elicitation": map[string]any{"form": map[string]any{}, "url": map[string]any{}}})
	out := legacyCall(t, s)
	if out.Error == nil || out.Error.Code != -32042 {
		t.Fatalf("want -32042, got %+v", out)
	}
	els := out.Error.Data.Elicitations
	if len(els) != 1 || els[0].ElicitationID != "mcpx-u2" {
		t.Fatalf("want only the URL question listed, got %v", els)
	}
}

func TestURLRequiredWhenPushFailsInLoop(t *testing.T) {
	asker := &scriptedAsker{questions: []mcpserver.Question{urlQuestion("u3")}, text: "done"}
	s := mcpserver.New(newBackend(), "mcpx", "test")
	s.Ask = asker
	legacyInit(t, s, map[string]any{"elicitation": map[string]any{"form": map[string]any{}, "url": map[string]any{}}})
	// canPush is now true, so the entry trigger does not fire; but the
	// connection still has no sender, so askClient returns ErrNoPush
	// and the in-loop trigger answers -32042.
	s.SetPush(func(string, any) {})
	out := legacyCall(t, s)
	if out.Error == nil || out.Error.Code != -32042 {
		t.Fatalf("want -32042 from the ErrNoPush trigger, got %+v", out)
	}
	if asker.abandoned != 0 {
		t.Errorf("abandoned %d, want 0", asker.abandoned)
	}
}

func TestNoURLRequiredForFormOnlyQuestion(t *testing.T) {
	asker := &scriptedAsker{questions: oneQuestion(), text: "done"}
	s := mcpserver.New(newBackend(), "mcpx", "test")
	s.Ask = asker
	s.Timing = mcpserver.Timing{AskTimeout: 300 * time.Millisecond, AskPoll: 20 * time.Millisecond}
	legacyInit(t, s, map[string]any{"elicitation": map[string]any{"form": map[string]any{}, "url": map[string]any{}}})
	out := legacyCall(t, s)
	if out.Error != nil && out.Error.Code == -32042 {
		t.Fatalf("a form question must never produce -32042: %+v", out)
	}
	if asker.abandoned != 1 {
		t.Errorf("a form-only call still times out the old way (abandoned %d)", asker.abandoned)
	}
}

func TestNoURLRequiredWhenURLNotDeclared(t *testing.T) {
	asker := &scriptedAsker{questions: []mcpserver.Question{urlQuestion("u4")}, text: "done"}
	s := mcpserver.New(newBackend(), "mcpx", "test")
	s.Ask = asker
	legacyInit(t, s, map[string]any{"elicitation": map[string]any{"form": map[string]any{}}})
	out := legacyCall(t, s)
	if out.Error != nil && out.Error.Code == -32042 {
		t.Fatalf("an undeclared host must never receive -32042: %+v", out)
	}
	if asker.began != 0 {
		t.Errorf("a form-only host keeps the ordinary path (asker began %d)", asker.began)
	}
}

func TestNoURLRequiredForModernPeer(t *testing.T) {
	asker := &scriptedAsker{questions: []mcpserver.Question{urlQuestion("u5")}, text: "done"}
	s := mcpserver.New(newBackend(), "mcpx", "test")
	s.Ask = asker
	params := map[string]any{
		"name":      "mcpx_call",
		"arguments": map[string]any{"namespace": "x", "tool": "y"},
		"_meta": map[string]any{
			mcpserver.MetaProtocolVersion:    "2026-07-28",
			mcpserver.MetaClientCapabilities: map[string]any{"elicitation": map[string]any{"form": map[string]any{}, "url": map[string]any{}}},
		},
	}
	resp := s.Handle(context.Background(), mcpserver.Request(1, "tools/call", params))
	b, _ := json.Marshal(resp)
	var out urlRequiredReply
	_ = json.Unmarshal(b, &out)
	if out.Error != nil && out.Error.Code == -32042 {
		t.Fatalf("2026 must never receive -32042: %s", b)
	}
	if out.Result == nil {
		t.Fatalf("a modern peer is handed the question in an input_required result: %s", b)
	}
}

// upstream32042Asker finishes at once with an upstream -32042 error.
type upstream32042Asker struct{ abandoned int }

func (a *upstream32042Asker) Begin(context.Context, string, json.RawMessage) (string, error) {
	return "call-1", nil
}

func (a *upstream32042Asker) Poll(context.Context, string, time.Duration) (mcpserver.Outcome, error) {
	return mcpserver.Outcome{Done: true, Err: &mcpserver.UpstreamError{
		Code:    -32042,
		Message: "url elicitation required",
		Data:    json.RawMessage(`{"elicitations":[{"mode":"url","url":"https://auth.example/x","elicitationId":"up-9","message":"connect"}]}`),
	}}, nil
}

func (a *upstream32042Asker) Reply(context.Context, string, map[string]json.RawMessage) error {
	return nil
}

func (a *upstream32042Asker) Abandon(string) { a.abandoned++ }

func TestUpstreamURLRequiredRelayedOnlyToDeclaredHosts(t *testing.T) {
	t.Run("2025-11-25 url-declared host gets the relay", func(t *testing.T) {
		s := mcpserver.New(newBackend(), "mcpx", "test")
		s.Ask = &upstream32042Asker{}
		legacyInit(t, s, map[string]any{"elicitation": map[string]any{"form": map[string]any{}, "url": map[string]any{}}})
		out := legacyCall(t, s)
		if out.Error == nil || out.Error.Code != -32042 {
			t.Fatalf("want the upstream -32042 relayed, got %+v", out)
		}
		if len(out.Error.Data.Elicitations) != 1 || out.Error.Data.Elicitations[0].ElicitationID != "up-9" {
			t.Errorf("relayed data changed: %+v", out.Error.Data)
		}
	})
	t.Run("2026 host gets a generic error, never -32042", func(t *testing.T) {
		s := mcpserver.New(newBackend(), "mcpx", "test")
		s.Ask = &upstream32042Asker{}
		params := map[string]any{
			"name":      "mcpx_call",
			"arguments": map[string]any{"namespace": "x", "tool": "y"},
			"_meta": map[string]any{
				mcpserver.MetaProtocolVersion:    "2026-07-28",
				mcpserver.MetaClientCapabilities: map[string]any{"elicitation": map[string]any{"form": map[string]any{}, "url": map[string]any{}}},
			},
		}
		resp := s.Handle(context.Background(), mcpserver.Request(1, "tools/call", params))
		if resp == nil || resp.Error == nil {
			t.Fatalf("want a generic error, got %v", resp)
		}
		b, _ := json.Marshal(resp)
		var out urlRequiredReply
		_ = json.Unmarshal(b, &out)
		if out.Error.Code == -32042 {
			t.Fatalf("2026 must never receive -32042: %s", b)
		}
	})
}

// A host declaring ONLY elicitation.url answers nothing inline, so
// AnswersInline is false; it must still be admitted, or the -32042
// path is unreachable for exactly the hosts it exists for (Codex
// finding on PR #2886).
func TestURLRequiredForURLOnlyHost(t *testing.T) {
	asker := &scriptedAsker{questions: []mcpserver.Question{urlQuestion("u9")}, text: "done"}
	s := mcpserver.New(newBackend(), "mcpx", "test")
	s.Ask = asker
	legacyInit(t, s, map[string]any{"elicitation": map[string]any{"url": map[string]any{}}})
	out := legacyCall(t, s)
	if out.Error == nil || out.Error.Code != -32042 {
		t.Fatalf("url-only host: want -32042, got %+v", out)
	}
	if len(out.Error.Data.Elicitations) != 1 || out.Error.Data.Elicitations[0].ElicitationID != "mcpx-u9" {
		t.Fatalf("url-only host: want the one URL question listed, got %v", out.Error.Data.Elicitations)
	}
	if asker.abandoned != 0 {
		t.Errorf("abandoned %d, want 0", asker.abandoned)
	}
}
