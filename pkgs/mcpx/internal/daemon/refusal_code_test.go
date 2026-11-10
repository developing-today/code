package daemon

import (
	"context"
	"encoding/json"
	"errors"
	"path/filepath"
	"testing"
	"time"

	"github.com/developing-today/code/pkgs/mcpx/internal/config"
	"github.com/developing-today/code/pkgs/mcpx/internal/elicit"
	"github.com/developing-today/code/pkgs/mcpx/internal/mcpclient"
	"github.com/developing-today/code/pkgs/mcpx/internal/pool"
)

// #1927 item 6: every refusal on the mcpx call path carries the
// structured reason code defined in internal/elicit, alongside the
// human-readable error, and the daemon's error body surfaces it so
// exec/diagnose callers can branch on it without parsing prose.

func refusalRegistry(t *testing.T, askTimeout time.Duration) (*Registry, *elicit.Broker) {
	t.Helper()
	broker, err := elicit.Open(filepath.Join(t.TempDir(), "elicit.db"))
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { broker.Close() })
	r := &Registry{
		consumer: ConsumerPolicy{
			WritePolicy:        "allow",
			ConfirmDestructive: true,
			AskTimeout:         askTimeout,
		},
		broker: broker,
	}
	return r, broker
}

func refusalPool() *pool.Pool {
	p := pool.New(&config.Resolved{
		Server:    &config.Server{Name: "demo", Command: "demo"},
		Namespace: "demo",
	})
	p.SetSchemas(
		[]mcpclient.Tool{{Name: "wipe"}},
		nil, nil, "", time.Now(), time.Time{}, false)
	return p
}

func refusalCodeOf(t *testing.T, err error) elicit.RefusalCode {
	t.Helper()
	var notConfirmed ErrNotConfirmed
	if !errors.As(err, &notConfirmed) {
		t.Fatalf("error = %T %v, want ErrNotConfirmed", err, err)
	}
	return notConfirmed.Code
}

// A destructive confirmation nobody answers refuses with
// confirmation_unanswered.
func TestConfirmDestructiveUnansweredCarriesRefusalCode(t *testing.T) {
	r, _ := refusalRegistry(t, 300*time.Millisecond)
	err := r.confirmDestructive(context.Background(), refusalPool(), "demo", "wipe", config.CallContext{})
	if got := refusalCodeOf(t, err); got != elicit.RefusalUnanswered {
		t.Errorf("unanswered confirmation code = %q, want %q", got, elicit.RefusalUnanswered)
	}
	if body := callErrorBody(err); body.RefusalCode != string(elicit.RefusalUnanswered) {
		t.Errorf("callErrorBody.RefusalCode = %q, want %q", body.RefusalCode, elicit.RefusalUnanswered)
	}
}

// An elicitation the answerer declines refuses with
// elicitation_declined.
func TestConfirmDestructiveDeclinedCarriesRefusalCode(t *testing.T) {
	r, broker := refusalRegistry(t, 10*time.Second)
	done := make(chan error, 1)
	go func() {
		done <- r.confirmDestructive(context.Background(), refusalPool(), "demo", "wipe", config.CallContext{})
	}()
	deadline := time.Now().Add(5 * time.Second)
	for {
		pending, err := broker.Pending(elicit.Filter{})
		if err != nil {
			t.Fatal(err)
		}
		if len(pending) > 0 {
			if err := broker.Respond(elicit.Answer{ID: pending[0].ID, Action: elicit.Decline, By: "test"}); err != nil {
				t.Fatal(err)
			}
			break
		}
		if time.Now().After(deadline) {
			t.Fatal("the confirmation question never reached the broker")
		}
		time.Sleep(10 * time.Millisecond)
	}
	if got := refusalCodeOf(t, <-done); got != elicit.RefusalDeclined {
		t.Errorf("declined confirmation code = %q, want %q", got, elicit.RefusalDeclined)
	}
}

// A confirmed call proceeds and carries no refusal code at all.
func TestConfirmedCallCarriesNoRefusalCode(t *testing.T) {
	r, broker := refusalRegistry(t, 10*time.Second)
	done := make(chan error, 1)
	go func() {
		done <- r.confirmDestructive(context.Background(), refusalPool(), "demo", "wipe", config.CallContext{})
	}()
	deadline := time.Now().Add(5 * time.Second)
	for {
		pending, err := broker.Pending(elicit.Filter{})
		if err != nil {
			t.Fatal(err)
		}
		if len(pending) > 0 {
			content, _ := json.Marshal(map[string]any{elicit.FieldConfirm: true})
			if err := broker.Respond(elicit.Answer{ID: pending[0].ID, Action: elicit.Accept, Content: content, By: "test"}); err != nil {
				t.Fatal(err)
			}
			break
		}
		if time.Now().After(deadline) {
			t.Fatal("the confirmation question never reached the broker")
		}
		time.Sleep(10 * time.Millisecond)
	}
	if err := <-done; err != nil {
		t.Fatalf("confirmed call was refused: %v", err)
	}
}

// Ask with no broker refuses with confirmation_unavailable.
func TestWritePolicyAskWithNoBrokerCarriesRefusalCode(t *testing.T) {
	r := writePolicyRegistry("ask")
	p := writePolicyPool(t, "", ``)
	_, err := r.applyWritePolicy(context.Background(), p, "demo", "wipe", config.CallContext{}, nil)
	if got := refusalCodeOf(t, err); got != elicit.RefusalUnavailable {
		t.Errorf("ask-with-no-broker code = %q, want %q", got, elicit.RefusalUnavailable)
	}
	if body := callErrorBody(err); body.Code != "confirmation_required" || body.RefusalCode != string(elicit.RefusalUnavailable) {
		t.Errorf("callErrorBody = code %q refusalCode %q, want confirmation_required / %q",
			body.Code, body.RefusalCode, elicit.RefusalUnavailable)
	}
}
