package cli

import (
	"context"
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"testing"

	"github.com/dezren39/mcpx/internal/daemon"
)

// #1961: Intent carries the caller's provider/model to /v1/intent when
// supplied, and omits them when not.
func TestIntentThreadsCallerProviderAndModel(t *testing.T) {
	bodies := make(chan map[string]any, 2)
	ts := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.URL.Path != "/v1/intent" {
			t.Errorf("path = %s", r.URL.Path)
		}
		var body map[string]any
		if err := json.NewDecoder(r.Body).Decode(&body); err != nil {
			t.Error(err)
		}
		bodies <- body
		_, _ = w.Write([]byte(`{"autonomy":"propose","model":"provider:claude/opus","source":"return 1;"}`))
	}))
	defer ts.Close()

	c := NewClientAt(daemon.Paths{}, "", ts.URL)
	res, err := c.Intent(context.Background(), "do a thing", "propose", nil, "sess", "claude", "opus")
	if err != nil {
		t.Fatalf("Intent: %v", err)
	}
	if res.Model != "provider:claude/opus" {
		t.Errorf("model = %q", res.Model)
	}
	body := <-bodies
	if body["provider"] != "claude" || body["model"] != "opus" {
		t.Errorf("body = %v, want provider/model threaded", body)
	}

	if _, err := c.Intent(context.Background(), "do a thing", "propose", nil, "sess", "", ""); err != nil {
		t.Fatalf("Intent: %v", err)
	}
	body = <-bodies
	if _, ok := body["provider"]; ok {
		t.Errorf("provider present in body when not supplied: %v", body)
	}
	if _, ok := body["model"]; ok {
		t.Errorf("model present in body when not supplied: %v", body)
	}
}
