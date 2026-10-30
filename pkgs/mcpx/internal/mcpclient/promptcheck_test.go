package mcpclient

import (
	"encoding/json"
	"testing"
)

// TestValidatePromptResult is the conformance cover for prompts-validate-io:
// an upstream prompts/get result is validated before it is passed on, and a
// malformed result is refused with a clear error instead of flowing through.
func TestValidatePromptResult(t *testing.T) {
	valid := []string{
		`{"messages":[{"role":"user","content":{"type":"text","text":"hi"}}]}`,
		`{"description":"d","messages":[{"role":"assistant","content":[{"type":"text","text":"a"},{"type":"image","data":"aGk=","mimeType":"image/png"}]}]}`,
		`{"messages":[{"role":"user","content":{"type":"resource","resource":{"uri":"file:///x","text":"body"}}}]}`,
		`{"messages":[{"role":"user","content":{"type":"resource","resource":{"uri":"file:///x","blob":"aGk=","mimeType":"application/octet-stream"}}}]}`,
		`{"messages":[{"role":"user","content":{"type":"resource_link","uri":"file:///x","name":"x"}}]}`,
		`{"messages":[]}`,
	}
	for _, raw := range valid {
		if err := ValidatePromptResult(json.RawMessage(raw)); err != nil {
			t.Errorf("valid result refused: %s: %v", raw, err)
		}
	}
	invalid := []string{
		`[]`,
		`{"description":"d"}`,
		`{"messages":null}`,
		`{"messages":[{"content":{"type":"text","text":"hi"}}]}`,
		`{"messages":[{"role":"system","content":{"type":"text","text":"hi"}}]}`,
		`{"messages":[{"role":"user"}]}`,
		`{"messages":[{"role":"user","content":"hi"}]}`,
		`{"messages":[{"role":"user","content":{"text":"hi"}}]}`,
		`{"messages":[{"role":"user","content":{"type":"text"}}]}`,
		`{"messages":[{"role":"user","content":{"type":"text","text":42}}]}`,
		`{"messages":[{"role":"user","content":{"type":"image","data":"aGk="}}]}`,
		`{"messages":[{"role":"user","content":{"type":"image","data":"!!not-base64!!","mimeType":"image/png"}}]}`,
		`{"messages":[{"role":"user","content":{"type":"resource","resource":{"text":"body"}}}]}`,
		`{"messages":[{"role":"user","content":{"type":"resource","resource":{"uri":"file:///x"}}}]}`,
		`{"messages":[{"role":"user","content":{"type":"resource_link","uri":"file:///x"}}]}`,
	}
	for _, raw := range invalid {
		err := ValidatePromptResult(json.RawMessage(raw))
		if err == nil {
			t.Errorf("invalid result passed through: %s", raw)
		}
	}
}
