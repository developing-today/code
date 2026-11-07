package scripting

import (
	"context"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
	"time"
)

func TestProbeHugeErrorBody(t *testing.T) {
	big := strings.Repeat("x", 32<<20)
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.WriteHeader(http.StatusInternalServerError)
		_, _ = w.Write([]byte(big))
	}))
	defer srv.Close()
	p := NewOpenAICompatibleProvider("probe", srv.URL, "", "m", 30*time.Second)
	_, err := p.Complete(context.Background(), CompletionRequest{UserPrompt: "hi"})
	if err == nil {
		t.Fatal("expected error")
	}
	t.Logf("error length = %d bytes (body was %d)", len(err.Error()), len(big))
	if len(err.Error()) < len(big) {
		t.Fatalf("error shorter than body: %d", len(err.Error()))
	}
}
