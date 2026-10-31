package config

import (
	"encoding/json"
	"testing"
)

// A tool view pin is forwarded verbatim on every call and the caller
// cannot override it, so an integer pin past the exact float64 range
// must survive config decoding exactly (Codex P2 on PR #2976:
// 9007199254740993 arrived as 9007199254740992).
func TestParsePreservesPinNumberPrecision(t *testing.T) {
	c, err := parse([]byte(`{
		"mcpServers": {
			"base": {"command": "x"},
			"view": {
				"aliasOf": "base",
				"mcpx": {"toolViews": {
					"getThing": {"from": "get_thing", "pin": {"id": 9007199254740993, "ratio": 1.5, "name": "n"}}
				}}
			}
		}
	}`))
	if err != nil {
		t.Fatalf("parse: %v", err)
	}
	pin := c.MCPServers["view"].Mcpx.ToolViews["getThing"].Pin
	got, err := json.Marshal(pin["id"])
	if err != nil {
		t.Fatalf("marshal pin id: %v", err)
	}
	if string(got) != "9007199254740993" {
		t.Errorf("pin id must decode exactly, got %s", got)
	}
	if _, isNumber := pin["id"].(json.Number); !isNumber {
		t.Errorf("pin id should decode as json.Number, got %T", pin["id"])
	}
	got, _ = json.Marshal(pin["ratio"])
	if string(got) != "1.5" {
		t.Errorf("fractional pin must survive too, got %s", got)
	}
	if pin["name"] != "n" {
		t.Errorf("string pin changed: %v", pin["name"])
	}
}
