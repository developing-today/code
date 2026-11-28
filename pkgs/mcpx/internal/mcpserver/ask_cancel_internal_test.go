package mcpserver

import (
	"encoding/json"
	"testing"
)

// TestCancelledForKeepsIDTypesDistinct pins the provenance Codex
// flagged on the ask-path cancellation routing: JSON-RPC treats a
// string id and a numeric id as different requests, so the record a
// notifications/cancelled leaves must keep them apart in both
// directions. Collapsing them (recording "7" for both) would let one
// request's cancellation be read as another's when the second
// request's context later ends for an unrelated reason.
func TestCancelledForKeepsIDTypesDistinct(t *testing.T) {
	s := New(nil, "mcpx", "test")

	numeric := s.newConn("", nil)
	numeric.cancel(json.RawMessage(`{"requestId":7,"reason":"user"}`))
	if _, ok := numeric.cancelledFor(json.RawMessage(`7`)); !ok {
		t.Error("the numeric request's cancellation should be recorded for the numeric id")
	}
	if _, ok := numeric.cancelledFor(json.RawMessage(`"7"`)); ok {
		t.Error("the numeric request's cancellation must not be visible to the string-id request")
	}

	str := s.newConn("", nil)
	str.cancel(json.RawMessage(`{"requestId":"7","reason":"user"}`))
	if _, ok := str.cancelledFor(json.RawMessage(`"7"`)); !ok {
		t.Error("the string request's cancellation should be recorded for the string id")
	}
	if _, ok := str.cancelledFor(json.RawMessage(`7`)); ok {
		t.Error("the string request's cancellation must not be visible to the numeric-id request")
	}
}
