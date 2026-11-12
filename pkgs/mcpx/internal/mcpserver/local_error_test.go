package mcpserver

import (
	"encoding/json"
	"strings"
	"testing"
)

// Oracle: the #2434 Phase 1 contract — an error mcpx mints itself is
// recognisable by {"mcpx":{"origin":"local"}} in its data; an error
// relayed from an upstream peer is answered verbatim, marker-free.

func errorData(t *testing.T, raw []byte) map[string]any {
	t.Helper()
	var resp struct {
		Error struct {
			Code    int            `json:"code"`
			Message string         `json:"message"`
			Data    map[string]any `json:"data"`
		} `json:"error"`
	}
	if err := json.Unmarshal(raw, &resp); err != nil {
		t.Fatalf("unmarshal %s: %v", raw, err)
	}
	return resp.Error.Data
}

func TestLocallyMintedErrorCarriesOriginMarker(t *testing.T) {
	resp := &response{JSONRPC: "2.0", ID: json.RawMessage(`1`),
		Error: mintLocalError(codeInternal, "boom", nil)}
	raw, err := json.Marshal(resp)
	if err != nil {
		t.Fatal(err)
	}
	data := errorData(t, raw)
	mcpx, ok := data["mcpx"].(map[string]any)
	if !ok || mcpx["origin"] != "local" {
		t.Fatalf("data = %v, want mcpx.origin=local (%s)", data, raw)
	}
}

func TestLocalOriginMarkerMergesIntoExistingData(t *testing.T) {
	resp := &response{JSONRPC: "2.0", ID: json.RawMessage(`1`),
		Error: &rpcError{Code: codeUnsupportedVersion, Message: "nope",
			Data: map[string]any{"supported": []string{"2026-07-28"},
				"mcpx": map[string]any{"surface": "gateway"}}}}
	raw, err := json.Marshal(resp)
	if err != nil {
		t.Fatal(err)
	}
	data := errorData(t, raw)
	if _, ok := data["supported"]; !ok {
		t.Fatalf("existing data lost: %v", data)
	}
	mcpx, ok := data["mcpx"].(map[string]any)
	if !ok || mcpx["origin"] != "local" || mcpx["surface"] != "gateway" {
		t.Fatalf("mcpx data = %v, want origin=local merged with surface=gateway", data["mcpx"])
	}
}

func TestRelayedUpstreamErrorCarriesNoOriginMarker(t *testing.T) {
	up := &UpstreamError{Code: -32021, Message: "missing",
		Data: json.RawMessage(`{"requiredCapabilities":{"sampling":{}}}`)}
	raw, err := json.Marshal(up.relay(json.RawMessage(`7`)))
	if err != nil {
		t.Fatal(err)
	}
	if strings.Contains(string(raw), `"mcpx"`) {
		t.Fatalf("relayed error gained a local marker: %s", raw)
	}
	data := errorData(t, raw)
	if _, ok := data["requiredCapabilities"]; !ok {
		t.Fatalf("relayed data lost: %s", raw)
	}
}
