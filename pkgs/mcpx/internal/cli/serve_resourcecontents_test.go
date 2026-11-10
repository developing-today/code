package cli

import (
	"encoding/json"
	"testing"
)

// TestResourceContentsValidation is the serve-path half of #2754: a
// malformed upstream resources/read reply is an error on the read, never
// an empty or fabricated successful read; valid text/blob entries pass
// through with their URIs namespaced.
func TestResourceContentsValidation(t *testing.T) {
	a := &App{}
	out, err := a.resourceContents(json.RawMessage(
		`{"contents":[{"uri":"file:///x","text":"body","mimeType":"text/plain"},{"uri":"file:///y","blob":"aGk="}]}`), "srv")
	if err != nil {
		t.Fatalf("valid reply refused: %v", err)
	}
	if len(out) != 2 {
		t.Fatalf("want 2 entries, got %d", len(out))
	}
	if out[0].URI != "mcpx://srv/file:///x" || out[0].Text != "body" || out[0].MimeType != "text/plain" {
		t.Errorf("text entry mangled: %+v", out[0])
	}
	if out[1].URI != "mcpx://srv/file:///y" || out[1].Blob != "aGk=" || out[1].Text != "" {
		t.Errorf("blob entry mangled: %+v", out[1])
	}
	invalid := []string{
		`{"contents":null}`,
		`{}`,
		`not json`,
		`{"contents":[{"uri":"file:///x"}]}`,
		`{"contents":[{"uri":"file:///x","text":"a","blob":"aGk="}]}`,
		`{"contents":[{"uri":"file:///x","blob":"!!not-base64!!"}]}`,
	}
	for _, raw := range invalid {
		if _, err := a.resourceContents(json.RawMessage(raw), "srv"); err == nil {
			t.Errorf("invalid reply passed through: %s", raw)
		}
	}
}
