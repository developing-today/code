package mcpclient

import (
	"context"
	"errors"
	"fmt"
	"net/http"
	"net/http/httptest"
	"net/url"
	"strings"
	"sync/atomic"
	"testing"
)

const redirectTestAPIKey = "redirect-test-credential"

func newRedirectTarget(t *testing.T) (*httptest.Server, <-chan string) {
	t.Helper()
	seen := make(chan string, 1)
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		seen <- r.Header.Get("X-API-Key")
		w.WriteHeader(http.StatusNoContent)
	}))
	t.Cleanup(srv.Close)
	return srv, seen
}

func assertRedirectTargetNotReached(t *testing.T, seen <-chan string) {
	t.Helper()
	select {
	case got := <-seen:
		t.Fatalf("cross-origin redirect reached target (X-API-Key=%q)", got)
	default:
	}
}

func assertCrossOriginRedirectReturned(t *testing.T, err error, wantStatus int) {
	t.Helper()
	var statusErr *HTTPStatusError
	if !errors.As(err, &statusErr) || statusErr.Status != wantStatus {
		t.Fatalf("request error = %v, want HTTPStatusError %d", err, wantStatus)
	}
}

func TestHTTPTransportRefusesCrossOriginRedirect(t *testing.T) {
	target, seen := newRedirectTarget(t)
	source := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		http.Redirect(w, r, target.URL, http.StatusTemporaryRedirect)
	}))
	defer source.Close()

	tr, err := NewHTTP(HTTPOptions{URL: source.URL, Headers: map[string]string{"X-API-Key": redirectTestAPIKey}})
	if err != nil {
		t.Fatal(err)
	}
	defer tr.Abandon()

	err = tr.Send(context.Background(), []byte(`{"jsonrpc":"2.0","id":1,"method":"tools/list"}`))
	assertCrossOriginRedirectReturned(t, err, http.StatusTemporaryRedirect)
	assertRedirectTargetNotReached(t, seen)
}

func TestHTTPTransportFollowsSameOriginRedirectWithHeaders(t *testing.T) {
	seen := make(chan string, 1)
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.URL.Path == "/start" {
			http.Redirect(w, r, "/target", http.StatusTemporaryRedirect)
			return
		}
		seen <- r.Header.Get("X-API-Key")
		w.WriteHeader(http.StatusNoContent)
	}))
	defer srv.Close()

	tr, err := NewHTTP(HTTPOptions{URL: srv.URL + "/start", Headers: map[string]string{"X-API-Key": redirectTestAPIKey}})
	if err != nil {
		t.Fatal(err)
	}
	defer tr.Abandon()
	if err := tr.Send(context.Background(), []byte(`{"jsonrpc":"2.0","id":1,"method":"tools/list"}`)); err != nil {
		t.Fatal(err)
	}
	if got := <-seen; got != redirectTestAPIKey {
		t.Fatalf("same-origin redirect X-API-Key = %q, want configured header", got)
	}
}

func TestHTTPStreamClientRefusesCrossOriginRedirect(t *testing.T) {
	target, seen := newRedirectTarget(t)
	source := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		http.Redirect(w, r, target.URL, http.StatusFound)
	}))
	defer source.Close()
	tr, err := NewHTTP(HTTPOptions{URL: source.URL, Headers: map[string]string{"X-API-Key": redirectTestAPIKey}})
	if err != nil {
		t.Fatal(err)
	}
	defer tr.Abandon()

	body, err := tr.get(context.Background(), "")
	if body != nil {
		body.Close()
	}
	assertCrossOriginRedirectReturned(t, err, http.StatusFound)
	assertRedirectTargetNotReached(t, seen)
}

func TestLegacySSEInitialGETRefusesCrossOriginRedirect(t *testing.T) {
	target, seen := newRedirectTarget(t)
	source := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		http.Redirect(w, r, target.URL, http.StatusFound)
	}))
	defer source.Close()

	tr, err := NewLegacySSE(context.Background(), HTTPOptions{URL: source.URL, Headers: map[string]string{"X-API-Key": redirectTestAPIKey}})
	if tr != nil {
		tr.Close()
	}
	assertCrossOriginRedirectReturned(t, err, http.StatusFound)
	assertRedirectTargetNotReached(t, seen)
}

func TestLegacySSEPOSTRefusesCrossOriginRedirect(t *testing.T) {
	target, seen := newRedirectTarget(t)
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		switch r.URL.Path {
		case "/sse":
			w.Header().Set("Content-Type", "text/event-stream")
			fmt.Fprint(w, "event: endpoint\ndata: /post\n\n")
			w.(http.Flusher).Flush()
			<-r.Context().Done()
		case "/post":
			http.Redirect(w, r, target.URL, http.StatusTemporaryRedirect)
		default:
			w.WriteHeader(http.StatusNotFound)
		}
	}))
	defer srv.Close()

	tr, err := NewLegacySSE(context.Background(), HTTPOptions{URL: srv.URL + "/sse", Headers: map[string]string{"X-API-Key": redirectTestAPIKey}})
	if err != nil {
		t.Fatal(err)
	}
	defer tr.Close()

	err = tr.Send(context.Background(), []byte(`{"jsonrpc":"2.0","id":1,"method":"tools/list"}`))
	assertCrossOriginRedirectReturned(t, err, http.StatusTemporaryRedirect)
	assertRedirectTargetNotReached(t, seen)
}

func TestSameHTTPOriginNormalizesSchemeHostAndDefaultPort(t *testing.T) {
	cases := []struct {
		left, right string
		want        bool
	}{
		{"http://example.test", "http://EXAMPLE.test:80", true},
		{"https://example.test", "https://example.test:443", true},
		{"http://example.test", "https://example.test", false},
		{"http://example.test:8080", "http://example.test:80", false},
	}
	for _, tc := range cases {
		t.Run(tc.left+" -> "+tc.right, func(t *testing.T) {
			left, err := url.Parse(tc.left)
			if err != nil {
				t.Fatal(err)
			}
			right, err := url.Parse(tc.right)
			if err != nil {
				t.Fatal(err)
			}
			if got := sameHTTPOrigin(left, right); got != tc.want {
				t.Fatalf("sameHTTPOrigin(%q, %q) = %v, want %v", tc.left, tc.right, got, tc.want)
			}
		})
	}
}

func TestHTTPTransportSameOriginRedirectLoopIsBounded(t *testing.T) {
	var hits atomic.Int32
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		hits.Add(1)
		http.Redirect(w, r, "/loop", http.StatusFound)
	}))
	defer srv.Close()

	tr, err := NewHTTP(HTTPOptions{URL: srv.URL + "/loop", Headers: map[string]string{"X-API-Key": redirectTestAPIKey}})
	if err != nil {
		t.Fatal(err)
	}
	defer tr.Abandon()

	err = tr.Send(context.Background(), []byte(`{"jsonrpc":"2.0","id":1,"method":"tools/list"}`))
	if err == nil || !strings.Contains(err.Error(), "stopped after 10 redirects") {
		t.Fatalf("request error = %v, want the redirect-count bound", err)
	}
	if got := hits.Load(); got < 2 || got > maxRedirects+1 {
		t.Fatalf("redirect loop reached the server %d times, want between 2 and %d", got, maxRedirects+1)
	}
}

func TestDeleteSessionReportsRefusedCrossOriginRedirect(t *testing.T) {
	target, seen := newRedirectTarget(t)
	source := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		http.Redirect(w, r, target.URL, http.StatusTemporaryRedirect)
	}))
	defer source.Close()

	tr, err := NewHTTP(HTTPOptions{URL: source.URL, Headers: map[string]string{"X-API-Key": redirectTestAPIKey}})
	if err != nil {
		t.Fatal(err)
	}
	tr.Resume(HTTPHandoff{SessionID: "sess-redirect"})

	err = tr.closeSession()
	if err == nil || !strings.Contains(err.Error(), "http 307") {
		t.Fatalf("closeSession = %v, want the unfollowed 307 reported as a termination failure", err)
	}
	assertRedirectTargetNotReached(t, seen)
}
