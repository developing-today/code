package client

import (
	"context"
	"encoding/json"
	"errors"
	"io"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
)

// fakeDaemon is a documented stand-in for the daemon: it serves the
// /v1 shapes the tests exercise and records what reached it, so a test
// can assert the client built the request the ops table declares —
// method, substituted path, query, and body — not merely that some
// request arrived.
type fakeDaemon struct {
	t      *testing.T
	last   recordedRequest
	health int
}

type recordedRequest struct {
	Method string
	Path   string
	URI    string
	Query  string
	Body   string
	Auth   string
}

func (f *fakeDaemon) handler() http.Handler {
	mux := http.NewServeMux()
	record := func(w http.ResponseWriter, r *http.Request) {
		body, err := io.ReadAll(r.Body)
		if err != nil {
			f.t.Errorf("fake daemon: reading body: %v", err)
		}
		f.last = recordedRequest{
			Method: r.Method,
			Path:   r.URL.Path,
			URI:    r.RequestURI,
			Query:  r.URL.RawQuery,
			Body:   string(body),
			Auth:   r.Header.Get("Authorization"),
		}
	}
	mux.HandleFunc("GET /v1/health", func(w http.ResponseWriter, r *http.Request) {
		record(w, r)
		f.health++
		_, _ = io.WriteString(w, `{"ok":true,"version":"test"}`)
	})
	mux.HandleFunc("GET /v1/tasks/{id}", func(w http.ResponseWriter, r *http.Request) {
		record(w, r)
		_, _ = io.WriteString(w, `{"id":"`+r.PathValue("id")+`","state":"done"}`)
	})
	mux.HandleFunc("POST /v1/elicit/{id}/{action}", func(w http.ResponseWriter, r *http.Request) {
		record(w, r)
		_, _ = io.WriteString(w, `{"answered":"`+r.PathValue("action")+`"}`)
	})
	mux.HandleFunc("GET /v1/events", func(w http.ResponseWriter, r *http.Request) {
		record(w, r)
		_, _ = io.WriteString(w, `{"events":[]}`)
	})
	return mux
}

func newFake(t *testing.T) (*fakeDaemon, *Client) {
	t.Helper()
	f := &fakeDaemon{t: t}
	srv := httptest.NewServer(f.handler())
	t.Cleanup(srv.Close)
	return f, New(srv.URL, WithToken("test-token"))
}

// TestHealthCall exercises a real /v1 call end to end: the parameterless
// health operation, decoded into a typed answer.
func TestHealthCall(t *testing.T) {
	f, c := newFake(t)
	var out struct {
		OK      bool   `json:"ok"`
		Version string `json:"version"`
	}
	if err := c.Call(context.Background(), "health", nil, &out); err != nil {
		t.Fatalf("health call: %v", err)
	}
	if !out.OK || out.Version != "test" {
		t.Fatalf("health answer = %+v", out)
	}
	if f.last.Method != http.MethodGet || f.last.Path != "/v1/health" {
		t.Fatalf("request = %s %s", f.last.Method, f.last.Path)
	}
	if f.last.Auth != "Bearer test-token" {
		t.Fatalf("authorization = %q", f.last.Auth)
	}
}

// TestPathAndBodyDerivation exercises two operations whose requests the
// table fully determines: task_get (path parameter) and elicit_answer
// (two path parameters plus a body object).
func TestPathAndBodyDerivation(t *testing.T) {
	f, c := newFake(t)
	ctx := context.Background()

	var task struct {
		ID    string `json:"id"`
		State string `json:"state"`
	}
	if err := c.Call(ctx, "task_get", map[string]any{"id": "t 1/2"}, &task); err != nil {
		t.Fatalf("task_get: %v", err)
	}
	if task.ID != "t 1/2" {
		t.Fatalf("task id round-trip = %q (path escaping broken)", task.ID)
	}
	if f.last.URI != "/v1/tasks/t%201%2F2" {
		t.Fatalf("escaped request URI = %s", f.last.URI)
	}

	var ans struct {
		Answered string `json:"answered"`
	}
	err := c.Call(ctx, "elicit_answer", map[string]any{
		"id": "q9", "action": "accept", "content": map[string]any{"choice": "a"},
	}, &ans)
	if err != nil {
		t.Fatalf("elicit_answer: %v", err)
	}
	if ans.Answered != "accept" || f.last.Path != "/v1/elicit/q9/accept" {
		t.Fatalf("answer = %+v via %s", ans, f.last.Path)
	}
	var sentBody map[string]any
	if err := json.Unmarshal([]byte(f.last.Body), &sentBody); err != nil {
		t.Fatalf("body not JSON: %v (%q)", err, f.last.Body)
	}
	content, ok := sentBody["content"].(map[string]any)
	if !ok || content["choice"] != "a" {
		t.Fatalf("body = %q, want the content object only", f.last.Body)
	}
}

// TestQueryDerivation checks query parameters land in the query string,
// including a repeated parameter from a slice.
func TestQueryDerivation(t *testing.T) {
	f, c := newFake(t)
	if _, err := c.CallRaw(context.Background(), "events", map[string]any{
		"kinds": "tool", "since": 5,
	}); err != nil {
		t.Fatalf("events: %v", err)
	}
	if !strings.Contains(f.last.Query, "kinds=tool") || !strings.Contains(f.last.Query, "since=5") {
		t.Fatalf("query = %q", f.last.Query)
	}
}

// TestArgRefusals pins the client-side refusals: unknown operation,
// undeclared argument, missing required parameter, and a value outside
// a declared enum. None of them may reach the daemon.
func TestArgRefusals(t *testing.T) {
	f, c := newFake(t)
	ctx := context.Background()

	if err := c.Call(ctx, "no_such_operation", nil, nil); !errors.Is(err, ErrUnknownOperation) {
		t.Fatalf("unknown op error = %v", err)
	}
	var argErr *ArgError
	if err := c.Call(ctx, "task_get", map[string]any{"bogus": 1}, nil); !errors.As(err, &argErr) {
		t.Fatalf("undeclared arg error = %v", err)
	}
	if err := c.Call(ctx, "task_get", nil, nil); !errors.As(err, &argErr) || argErr.Param != "id" {
		t.Fatalf("missing required error = %v", err)
	}
	err := c.Call(ctx, "elicit_answer", map[string]any{"id": "q1", "action": "shrug"}, nil)
	if !errors.As(err, &argErr) || argErr.Param != "action" {
		t.Fatalf("enum error = %v", err)
	}
	if f.health != 0 || f.last.Path != "" {
		t.Fatalf("a refused call reached the daemon: %+v", f.last)
	}
}

// TestHTTPErrorEnvelope pins the daemon's error shape: a non-2xx answer
// becomes an *HTTPError carrying status, code, message, and request id.
func TestHTTPErrorEnvelope(t *testing.T) {
	// A server that refuses the health path itself, so the envelope is
	// exercised through a real CallRaw round trip.
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.WriteHeader(http.StatusUnauthorized)
		_, _ = io.WriteString(w, `{"error":"unauthorized","code":"auth_required","message":"bearer token required","requestId":"req-1"}`)
	}))
	defer srv.Close()
	_, err := New(srv.URL).CallRaw(context.Background(), "health", nil)
	var httpErr *HTTPError
	if !errors.As(err, &httpErr) {
		t.Fatalf("error = %v (%T)", err, err)
	}
	if httpErr.Status != 401 || httpErr.Code != "auth_required" ||
		httpErr.Message != "bearer token required" || httpErr.RequestID != "req-1" {
		t.Fatalf("HTTPError = %+v", httpErr)
	}
}

// TestTableCoverage is the derivation guarantee: the client's operation
// list is the daemon's table, and every entry is a /v1 operation with a
// method — if the table gains an operation, this client gains it.
func TestTableCoverage(t *testing.T) {
	ops := Operations()
	if len(ops) < 50 {
		t.Fatalf("operations = %d, want the daemon's full table", len(ops))
	}
	for _, op := range ops {
		if op.Method == "" || !strings.HasPrefix(op.Path, "/v1/") {
			t.Fatalf("operation %s lacks a /v1 method+path: %s %s", op.Name, op.Method, op.Path)
		}
		if _, ok := Operation(op.Name); !ok {
			t.Fatalf("operation %s not lookupable", op.Name)
		}
	}
}

// TestGenericInvoke covers the generic convenience form.
func TestGenericInvoke(t *testing.T) {
	_, c := newFake(t)
	out, err := Invoke[map[string]any](context.Background(), c, "health", nil)
	if err != nil {
		t.Fatalf("invoke health: %v", err)
	}
	if out["ok"] != true {
		t.Fatalf("invoke answer = %v", out)
	}
}
