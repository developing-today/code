package catalog

// lane-scratch: code-w /tmp/lane-w-20261110-1514 task 13050

import (
	"context"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"

	"github.com/developing-today/code/pkgs/mcpx/internal/defaults"
)

// Oracle: response-read contract decision 1 (design #4375, closing
// audit #4376) — a non-200 embeddings body is read as an excerpt
// bounded at defaults.HTTPErrorBodyLimit, never as a whole read. A
// server returning far more than the limit must produce an error
// whose excerpt is exactly the limit, byte for byte.
func TestRemoteEmbedderErrorBodyBounded(t *testing.T) {
	oversized := strings.Repeat("x", int(defaults.HTTPErrorBodyLimit)+10000)
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.WriteHeader(http.StatusInternalServerError)
		_, _ = w.Write([]byte(oversized))
	}))
	defer srv.Close()

	rem := NewRemoteEmbedder(srv.URL, "test-key", "test-model")
	_, err := rem.Embed(context.Background(), "hello")
	if err == nil {
		t.Fatal("expected an error from the 500 response")
	}
	const prefix = "remote embeddings error 500: "
	if !strings.HasPrefix(err.Error(), prefix) {
		t.Fatalf("unexpected error shape: %.60q", err.Error())
	}
	excerpt := strings.TrimPrefix(err.Error(), prefix)
	if int64(len(excerpt)) != defaults.HTTPErrorBodyLimit {
		t.Fatalf("error excerpt is %d bytes, want exactly the %d-byte HTTPErrorBodyLimit", len(excerpt), defaults.HTTPErrorBodyLimit)
	}
}
