package mcpclient

import (
	"encoding/json"
	"testing"
)

// A -32021 minted by the ask seam must reach an upstream as -32021 with
// its requiredCapabilities, not as a sentence, and not as -32603 (#2438,
// Phase 3 of #2434). rpcErrorFor is the one place a handler error becomes
// the JSON-RPC error an upstream reads.
func TestMissingCapabilityErrorKeepsItsData(t *testing.T) {
	err := MissingCapabilityError("Missing required client capability: no human answerer is present",
		map[string]any{"elicitation": map[string]any{}})
	r := rpcErrorFor(err)
	if r.Code != -32021 || r.Message != "Missing required client capability: no human answerer is present" {
		t.Fatalf("relayed %d/%q, want -32021 with the sentence", r.Code, r.Message)
	}
	var body struct {
		Required map[string]any `json:"requiredCapabilities"`
	}
	if err := json.Unmarshal(r.Data, &body); err != nil {
		t.Fatalf("data is not requiredCapabilities: %v (%s)", err, r.Data)
	}
	if _, ok := body.Required["elicitation"]; !ok {
		t.Fatalf("requiredCapabilities %v do not name elicitation", body.Required)
	}
}
