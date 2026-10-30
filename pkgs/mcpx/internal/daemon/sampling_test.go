package daemon

import (
	"context"
	"encoding/json"
	"errors"
	"path/filepath"
	"testing"
	"time"

	"github.com/dezren39/mcpx/internal/config"
	"github.com/dezren39/mcpx/internal/elicit"
	"github.com/dezren39/mcpx/internal/events"
)

// sampleRig is a registry with a broker and no upstream connections: the
// sampling relay is what is under test, not the transport.
func sampleRig(t *testing.T) (*Registry, *elicit.Broker) {
	t.Helper()
	dir := t.TempDir()
	cfg := &config.Config{MCPServers: map[string]*config.Server{
		"a": {Command: filepath.Join(dir, "never-run")},
	}}
	r, err := NewRegistry(cfg, Paths{State: dir, Cache: dir}, nil)
	if err != nil {
		t.Fatal(err)
	}
	b, err := elicit.Open(filepath.Join(dir, "elicit.db"))
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { b.Close() })
	r.InstallHooks(events.New(0), b, nil)
	return r, b
}

func pending(t *testing.T, b *elicit.Broker) []elicit.Request {
	t.Helper()
	ps, err := b.Pending(elicit.Filter{})
	if err != nil {
		t.Fatal(err)
	}
	return ps
}

func TestSamplingRequestShapeRefusedWithoutAQuestion(t *testing.T) {
	// A tool_result naming no earlier tool_use is -32602, and nobody is
	// asked: the upstream's request was wrong, not the agent's answer.
	r, b := sampleRig(t)
	bad := json.RawMessage(`{"messages":[{"role":"user","content":[{"type":"tool_result","toolUseId":"nope"}]}],"maxTokens":8}`)
	_, err := r.sampleViaBroker(context.Background(), "a", "k", bad)
	var inv *elicit.SamplingInvalidParams
	if !errors.As(err, &inv) {
		t.Fatalf("err = %v, want SamplingInvalidParams", err)
	}
	if inv.RPCCode() != -32602 {
		t.Fatalf("RPCCode = %d, want -32602", inv.RPCCode())
	}
	if ps := pending(t, b); len(ps) != 0 {
		t.Fatalf("%d questions opened for a refused request", len(ps))
	}
}

func TestSamplingAnswerWithoutRoleIsRefusedAndStaysOpen(t *testing.T) {
	// The answer step refuses a result that is not a CreateMessageResult, so
	// the question stays open; nothing is filled in on the answerer's behalf.
	r, b := sampleRig(t)
	params := json.RawMessage(`{"messages":[{"role":"user","content":{"type":"text","text":"hi"}}],"maxTokens":8}`)
	type outcome struct {
		result any
		err    error
	}
	done := make(chan outcome, 1)
	go func() {
		result, err := r.sampleViaBroker(context.Background(), "a", "k", params)
		done <- outcome{result, err}
	}()

	var q elicit.Request
	for i := 0; i < 200 && q.ID == ""; i++ {
		if ps := pending(t, b); len(ps) > 0 {
			q = ps[0]
		} else {
			time.Sleep(5 * time.Millisecond)
		}
	}
	if q.ID == "" {
		t.Fatal("no sampling question was opened")
	}

	noRole := json.RawMessage(`{"model":"m","content":{"type":"text","text":"hi"}}`)
	if err := b.Respond(elicit.Answer{ID: q.ID, Action: elicit.Accept, Content: noRole}); err == nil {
		t.Fatal("an answer with no role was accepted")
	}
	if ps := pending(t, b); len(ps) != 1 {
		t.Fatalf("the question was closed by a refused answer: %d pending", len(ps))
	}

	good := json.RawMessage(`{"role":"assistant","model":"m","content":{"type":"text","text":"hi"},"stopReason":"endTurn"}`)
	if err := b.Respond(elicit.Answer{ID: q.ID, Action: elicit.Accept, Content: good}); err != nil {
		t.Fatal(err)
	}
	select {
	case o := <-done:
		if o.err != nil {
			t.Fatal(o.err)
		}
		m, _ := o.result.(map[string]any)
		if m["role"] != "assistant" || m["model"] != "m" {
			t.Fatalf("relayed %v, want the answer unchanged", o.result)
		}
	case <-time.After(2 * time.Second):
		t.Fatal("sampleViaBroker did not return")
	}
}
