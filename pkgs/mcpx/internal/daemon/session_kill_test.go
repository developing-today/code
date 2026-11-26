package daemon

import (
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
)

// Session kill (#1931 slice 2) handler contract: the address is the caller
// session handle, the same handle release takes. A known handle is detached
// and its owned instances are force-ended; an unknown handle is a typed
// refusal (404 naming the handle), not an empty success; a missing handle
// is a 400.

func postKill(t *testing.T, s *Server, body string) *httptest.ResponseRecorder {
	t.Helper()
	rec := httptest.NewRecorder()
	req := httptest.NewRequest(http.MethodPost, "/v1/session/kill", strings.NewReader(body))
	s.handleKill(rec, req)
	return rec
}

func TestHandleKillUnknownSessionIsATypedRefusal(t *testing.T) {
	s := &Server{reg: &Registry{leases: map[string]*leaseState{}}}
	rec := postKill(t, s, `{"session":"ghost"}`)
	if rec.Code != http.StatusNotFound {
		t.Fatalf("status = %d, want 404; body %s", rec.Code, rec.Body)
	}
	var doc map[string]string
	if err := json.Unmarshal(rec.Body.Bytes(), &doc); err != nil {
		t.Fatal(err)
	}
	if !strings.Contains(doc["error"], "ghost") {
		t.Fatalf("refusal must name the handle, got %s", rec.Body)
	}
}

func TestHandleKillKnownSessionDetachesAndReports(t *testing.T) {
	s := &Server{reg: &Registry{leases: map[string]*leaseState{
		"s1": {owned: map[string]string{}},
	}}}
	rec := postKill(t, s, `{"session":"s1"}`)
	if rec.Code != http.StatusOK {
		t.Fatalf("status = %d, want 200; body %s", rec.Code, rec.Body)
	}
	var doc map[string]int
	if err := json.Unmarshal(rec.Body.Bytes(), &doc); err != nil {
		t.Fatal(err)
	}
	if doc["killed"] != 0 {
		t.Fatalf("no pools own anything, killed = %d, want 0", doc["killed"])
	}
	// The handle is detached: a second kill is the unknown-handle refusal.
	rec = postKill(t, s, `{"session":"s1"}`)
	if rec.Code != http.StatusNotFound {
		t.Fatalf("second kill status = %d, want 404; body %s", rec.Code, rec.Body)
	}
}

func TestHandleKillRequiresASession(t *testing.T) {
	s := &Server{reg: &Registry{leases: map[string]*leaseState{}}}
	rec := postKill(t, s, `{}`)
	if rec.Code != http.StatusBadRequest {
		t.Fatalf("status = %d, want 400; body %s", rec.Code, rec.Body)
	}
}
