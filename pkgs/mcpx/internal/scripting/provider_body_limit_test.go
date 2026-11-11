package scripting

import (
	"context"
	"fmt"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
	"time"

	"github.com/developing-today/code/pkgs/mcpx/internal/defaults"
)

// Regression tests for #4354: see the model package twin. Oracle: the
// produced error's length and text.

func bodyLimitServer(status int, body string) *httptest.Server {
	return httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.WriteHeader(status)
		_, _ = fmt.Fprint(w, body)
	}))
}

func TestCompleteBoundsOversizedErrorBody(t *testing.T) {
	srv := bodyLimitServer(500, strings.Repeat("x", 1<<20))
	defer srv.Close()
	p := NewOpenAICompatibleProvider("testprov", srv.URL, "", "m", time.Second)
	_, err := p.Complete(context.Background(), CompletionRequest{UserPrompt: "hi"})
	if err == nil {
		t.Fatal("expected an error for HTTP 500")
	}
	if int64(len(err.Error())) > defaults.HTTPErrorBodyLimit+256 {
		t.Fatalf("error length %d exceeds the error-body cap %d plus overhead", len(err.Error()), defaults.HTTPErrorBodyLimit)
	}
	if !strings.Contains(err.Error(), "HTTP 500") {
		t.Fatalf("error should name the status: %v", err)
	}
}

func TestCompleteRefusesOversizedSuccessBody(t *testing.T) {
	srv := bodyLimitServer(200, strings.Repeat("x", int(defaults.ProviderResponseBodyLimit)+1))
	defer srv.Close()
	p := NewOpenAICompatibleProvider("testprov", srv.URL, "", "m", time.Second)
	_, err := p.Complete(context.Background(), CompletionRequest{UserPrompt: "hi"})
	if err == nil {
		t.Fatal("expected an error for an over-cap success body")
	}
	if !strings.Contains(err.Error(), "exceeds") || !strings.Contains(err.Error(), "testprov") {
		t.Fatalf("error should name the provider and the cap refusal: %v", err)
	}
	if strings.Contains(err.Error(), "invalid json") {
		t.Fatalf("over-cap body must not surface as a JSON parse failure: %v", err)
	}
}

func TestCompleteSmallErrorBodyUnchanged(t *testing.T) {
	srv := bodyLimitServer(500, "boom: bad things")
	defer srv.Close()
	p := NewOpenAICompatibleProvider("testprov", srv.URL, "", "m", time.Second)
	_, err := p.Complete(context.Background(), CompletionRequest{UserPrompt: "hi"})
	if err == nil || !strings.Contains(err.Error(), "boom: bad things") {
		t.Fatalf("small error bodies pass through as before: %v", err)
	}
}
