package daemon

import (
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"testing"

	"github.com/dezren39/mcpx/internal/pool"
)

// The session inventory (#1931 slice 1): an empty inventory is an empty
// list, and a populated one carries the pools' InstanceStatus values
// unchanged — no parallel struct, no dropped fields.

func TestSessionInventoryEmptyIsAnEmptyList(t *testing.T) {
	got := sessionInventory(nil)
	if got == nil {
		t.Fatal("empty inventory must be an empty list, not nil (which JSON renders as null)")
	}
	if len(got) != 0 {
		t.Fatalf("expected no sessions, got %+v", got)
	}
	got = sessionInventory([]pool.Status{{Name: "a"}, {Name: "b"}})
	if len(got) != 0 {
		t.Fatalf("pools with no instances contribute none, got %+v", got)
	}
}

func TestSessionInventoryFlattensEveryPool(t *testing.T) {
	statuses := []pool.Status{
		{Name: "a", Instances: []pool.InstanceStatus{
			{ID: "i1", PID: 111, Holders: 2, Key: "k1", Calls: 5, UptimeSec: 60, IdleSec: 3},
		}},
		{Name: "b", Instances: []pool.InstanceStatus{
			{ID: "i2", PID: 222, Holders: 1, Key: "k2", Calls: 9, UptimeSec: 30, IdleSec: 0},
			{ID: "i3", PID: 333, Holders: 0, Key: "k3", Calls: 1, UptimeSec: 10, IdleSec: 8},
		}},
	}
	got := sessionInventory(statuses)
	if len(got) != 3 {
		t.Fatalf("expected 3 sessions across 2 pools, got %+v", got)
	}
	if got[0].ID != "i1" || got[1].ID != "i2" || got[2].ID != "i3" {
		t.Fatalf("order or content changed: %+v", got)
	}
	if got[0].PID != 111 || got[0].Holders != 2 || got[0].Key != "k1" ||
		got[0].Calls != 5 || got[0].UptimeSec != 60 || got[0].IdleSec != 3 {
		t.Fatalf("InstanceStatus fields must pass through unchanged: %+v", got[0])
	}
}

func TestHandleSessionsEmptyRegistryRendersEmptyList(t *testing.T) {
	s := &Server{reg: &Registry{}}
	rec := httptest.NewRecorder()
	s.handleSessions(rec, httptest.NewRequest(http.MethodGet, "/v1/sessions", nil))
	if rec.Code != http.StatusOK {
		t.Fatalf("status = %d, want 200; body %s", rec.Code, rec.Body)
	}
	var body struct {
		Sessions []pool.InstanceStatus `json:"sessions"`
	}
	if err := json.Unmarshal(rec.Body.Bytes(), &body); err != nil {
		t.Fatal(err)
	}
	if body.Sessions == nil || len(body.Sessions) != 0 {
		t.Fatalf("empty inventory must render as [], got %s", rec.Body)
	}
}
