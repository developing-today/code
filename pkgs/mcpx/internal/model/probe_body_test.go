package model

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
	p := NewOpenAIProvider("probe", srv.URL, "", 30*time.Second)
	_, err := p.Generate(context.Background(), Request{Model: "m", Messages: []Message{{Content: "hi"}}})
	if err == nil {
		t.Fatal("expected error")
	}
	t.Logf("error length = %d bytes (body was %d)", len(err.Error()), len(big))
	if len(err.Error()) < len(big) {
		t.Fatalf("error shorter than body: %d", len(err.Error()))
	}
}

func TestProbeHugeOKBodyBuffered(t *testing.T) {
	big := strings.Repeat("y", 32<<20)
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.WriteHeader(http.StatusOK)
		_, _ = w.Write([]byte(big)) // not JSON at all
	}))
	defer srv.Close()
	p := NewOpenAIProvider("probe", srv.URL, "", 30*time.Second)
	_, err := p.Generate(context.Background(), Request{Model: "m", Messages: []Message{{Content: "hi"}}})
	if err == nil {
		t.Fatal("expected invalid JSON error")
	}
	t.Logf("200 garbage body fully read before JSON error: %q", err.Error()[:80])
}
