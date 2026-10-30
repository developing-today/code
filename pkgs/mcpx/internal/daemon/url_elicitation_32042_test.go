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
	"github.com/dezren39/mcpx/internal/mcpclient"
)

// #2079 / ELI-09: broker rows are keyed by ID alone, and elicitationId
// is the upstream's own choice, so server-supplied IDs are stored
// server-qualified. Two upstreams choosing the same elicitationId hold
// distinct rows, and an answer settles only its own server's row.
func TestBrokerKeyingServerQualified(t *testing.T) {
	broker, err := elicit.Open(filepath.Join(t.TempDir(), "elicit.db"))
	if err != nil {
		t.Fatal(err)
	}
	defer broker.Close()
	r := &Registry{asks: newAskTable()}
	r.InstallHooks(events.New(0), broker, nil)

	params := json.RawMessage(`{"mode":"url","message":"sign in","url":"https://example.invalid/auth","elicitationId":"e1"}`)
	type outcome struct {
		out any
		err error
	}
	results := map[string]chan outcome{}
	for _, srv := range []string{"srvA", "srvB"} {
		ch := make(chan outcome, 1)
		results[srv] = ch
		go func(srv string, ch chan<- outcome) {
			out, err := r.elicitViaBroker(context.Background(), srv, "key", params)
			ch <- outcome{out, err}
		}(srv, ch)
	}

	waitRow := func(id string) {
		t.Helper()
		deadline := time.Now().Add(5 * time.Second)
		for time.Now().Before(deadline) {
			if _, ok, err := broker.Get(id); err == nil && ok {
				return
			}
			time.Sleep(10 * time.Millisecond)
		}
		t.Fatalf("broker row %q never appeared", id)
	}
	waitRow("srvA:e1")
	waitRow("srvB:e1")
	if _, ok, err := broker.Get("e1"); err != nil || ok {
		t.Fatalf("a bare e1 row exists (ok=%v err=%v): the collision is back", ok, err)
	}

	// Answering srvA's row settles only srvA's waiter.
	if err := broker.Respond(elicit.Answer{ID: "srvA:e1", Action: elicit.Accept, By: "test"}); err != nil {
		t.Fatal(err)
	}
	select {
	case o := <-results["srvA"]:
		if o.err != nil {
			t.Fatal(o.err)
		}
		if a, _ := o.out.(map[string]any)["action"].(string); a != "accept" {
			t.Fatalf("srvA outcome = %v, want accept", o.out)
		}
	case <-time.After(5 * time.Second):
		t.Fatal("srvA waiter never returned")
	}
	select {
	case o := <-results["srvB"]:
		t.Fatalf("srvB waiter returned early with %v: an answer crossed servers", o)
	case <-time.After(300 * time.Millisecond):
	}

	if err := broker.Respond(elicit.Answer{ID: "srvB:e1", Action: elicit.Decline, By: "test"}); err != nil {
		t.Fatal(err)
	}
	select {
	case o := <-results["srvB"]:
		if a, _ := o.out.(map[string]any)["action"].(string); a != "decline" {
			t.Fatalf("srvB outcome = %v, want decline", o.out)
		}
	case <-time.After(5 * time.Second):
		t.Fatal("srvB waiter never returned")
	}
}

// #2079: the -32042 handler runs each listed entry through the existing
// broker path in order; every entry must accept, and the first decline
// stops the flow with a named error, later entries unopened.
func TestURLElicitationsViaBroker(t *testing.T) {
	broker, err := elicit.Open(filepath.Join(t.TempDir(), "elicit.db"))
	if err != nil {
		t.Fatal(err)
	}
	defer broker.Close()
	r := &Registry{asks: newAskTable()}
	r.InstallHooks(events.New(0), broker, nil)

	els := []mcpclient.URLElicitation{
		{Mode: "url", URL: "https://example.invalid/a", ElicitationID: "e1", Message: "first"},
		{Mode: "url", URL: "https://example.invalid/b", ElicitationID: "e2", Message: "second"},
	}
	type outcome struct {
		ok  bool
		err error
	}
	done := make(chan outcome, 1)
	go func() {
		ok, err := r.urlElicitationsViaBroker(context.Background(), "srv", "key", els)
		done <- outcome{ok, err}
	}()

	answer := func(id string, action elicit.Action) {
		t.Helper()
		deadline := time.Now().Add(5 * time.Second)
		for time.Now().Before(deadline) {
			if _, ok, err := broker.Get(id); err == nil && ok {
				if err := broker.Respond(elicit.Answer{ID: id, Action: action, By: "test"}); err != nil {
					t.Fatal(err)
				}
				return
			}
			time.Sleep(10 * time.Millisecond)
		}
		t.Fatalf("broker row %q never appeared", id)
	}
	answer("srv:e1", elicit.Accept)
	answer("srv:e2", elicit.Decline)

	select {
	case o := <-done:
		if o.ok || o.err == nil || !strings.Contains(o.err.Error(), "e2") || !strings.Contains(o.err.Error(), "decline") {
			t.Fatalf("want (false, a named decline error for e2), got %v", o)
		}
	case <-time.After(5 * time.Second):
		t.Fatal("the handler never returned")
	}

	// First-entry decline: the second entry is never opened. (Fresh IDs:
	// broker IDs are primary keys, so a settled row cannot be reopened.)
	els2 := []mcpclient.URLElicitation{
		{Mode: "url", URL: "https://example.invalid/c", ElicitationID: "e3", Message: "third"},
		{Mode: "url", URL: "https://example.invalid/d", ElicitationID: "e4", Message: "fourth"},
	}
	done2 := make(chan outcome, 1)
	go func() {
		ok, err := r.urlElicitationsViaBroker(context.Background(), "srv", "key", els2)
		done2 <- outcome{ok, err}
	}()
	answer("srv:e3", elicit.Decline)
	select {
	case o := <-done2:
		if o.ok || o.err == nil {
			t.Fatalf("want (false, error) on first decline, got %v", o)
		}
	case <-time.After(5 * time.Second):
		t.Fatal("the handler never returned after the first decline")
	}
	if _, ok, err := broker.Get("srv:e4"); err != nil || ok {
		t.Fatalf("srv:e4 exists (ok=%v err=%v): a later entry was opened after a first-entry decline", ok, err)
	}
}
