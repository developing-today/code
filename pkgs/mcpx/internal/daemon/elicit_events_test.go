package daemon

import (
	"context"
	"encoding/json"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"github.com/dezren39/mcpx/internal/elicit"
	"github.com/dezren39/mcpx/internal/events"
)

// #1677: elicit.opened / elicit.answered carry a summary only -- id,
// server, tool, mode, audience, state, expiry -- never a question's
// message, schema or url, and never an answer's content. Canary
// strings planted in every sensitive field must be absent from the
// published payloads, end to end through elicitViaBroker.
func TestElicitEventsAreSummaryOnly(t *testing.T) {
	bus := events.New(0)
	broker, err := elicit.Open(filepath.Join(t.TempDir(), "elicit.db"))
	if err != nil {
		t.Fatal(err)
	}
	defer broker.Close()
	r := &Registry{asks: newAskTable()}
	r.InstallHooks(bus, broker, nil)
	sub, _ := bus.Subscribe(events.Filter{Kinds: []string{"elicit"}}, 0)
	defer sub.Close()

	params := json.RawMessage(`{
		"mode": "url",
		"message": "CANARY-MESSAGE",
		"requestedSchema": {"type": "object", "properties": {"x": {"type": "string", "description": "CANARY-SCHEMA"}}},
		"url": "https://example.invalid/CANARY-URL"
	}`)
	done := make(chan error, 1)
	go func() {
		_, err := r.elicitViaBroker(context.Background(), "srv", "key", params)
		done <- err
	}()

	recv := func(kind events.Kind) events.Event {
		t.Helper()
		select {
		case e := <-sub.C:
			if e.Kind != kind {
				t.Fatalf("got event %s, want %s", e.Kind, kind)
			}
			return e
		case <-time.After(5 * time.Second):
			t.Fatalf("no %s event arrived", kind)
		}
		return events.Event{}
	}
	check := func(e events.Event, wantState string) map[string]any {
		t.Helper()
		data := string(e.Data)
		for _, canary := range []string{"CANARY-MESSAGE", "CANARY-SCHEMA", "CANARY-URL", "CANARY-CONTENT"} {
			if strings.Contains(data, canary) {
				t.Errorf("%s payload leaks %s: %s", e.Kind, canary, data)
			}
		}
		var fields map[string]any
		if err := json.Unmarshal(e.Data, &fields); err != nil {
			t.Fatalf("%s payload is not an object: %v", e.Kind, err)
		}
		for _, k := range []string{"message", "requestedSchema", "schema", "url", "content"} {
			if _, ok := fields[k]; ok {
				t.Errorf("%s payload carries forbidden key %q: %s", e.Kind, k, data)
			}
		}
		for _, k := range []string{"id", "server", "tool", "mode", "audience", "state", "expiry"} {
			if _, ok := fields[k]; !ok {
				t.Errorf("%s payload is missing summary key %q: %s", e.Kind, k, data)
			}
		}
		if fields["mode"] != "url" || fields["server"] != "srv" || fields["state"] != wantState {
			t.Errorf("%s summary = %v, want mode=url server=srv state=%s", e.Kind, fields, wantState)
		}
		return fields
	}

	opened := check(recv(events.ElicitOpened), "pending")
	id, _ := opened["id"].(string)
	if id == "" {
		t.Fatal("opened summary has no id")
	}
	if err := broker.Respond(elicit.Answer{
		ID: id, Action: elicit.Accept,
		Content: json.RawMessage(`{"token": "CANARY-CONTENT"}`),
	}); err != nil {
		t.Fatal(err)
	}
	check(recv(events.ElicitAnswered), "answered")
	if err := <-done; err != nil {
		t.Fatal(err)
	}
}
