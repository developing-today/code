package mcpclient

import (
	"encoding/json"
	"testing"
)

// TestValidateResourceResult covers the #2754 probe matrix: an upstream
// resources/read result is validated before it is passed on, and a
// malformed result is refused with a clear error instead of becoming an
// empty or fabricated successful read.
func TestValidateResourceResult(t *testing.T) {
	valid := []string{
		`{"contents":[{"uri":"file:///x","text":"body"}]}`,
		`{"contents":[{"uri":"file:///x","blob":"aGk=","mimeType":"application/octet-stream"}]}`,
		`{"contents":[{"uri":"mcpx://srv/file:///x","text":"a","mimeType":"text/plain","_meta":{"k":1}},{"uri":"file:///y","blob":"AAEC"}]}`,
		`{"contents":[]}`,
	}
	for _, raw := range valid {
		if err := ValidateResourceResult(json.RawMessage(raw)); err != nil {
			t.Errorf("valid result refused: %s: %v", raw, err)
		}
	}
	invalid := []string{
		`[]`,
		`"text"`,
		`{}`,
		`{"contents":null}`,
		`{"contents":{}}`,
		`{"contents":"x"}`,
		`{"contents":[42]}`,
		`{"contents":[{"text":"body"}]}`,
		`{"contents":[{"uri":42,"text":"body"}]}`,
		`{"contents":[{"uri":"","text":"body"}]}`,
		`{"contents":[{"uri":"file:///x"}]}`,
		`{"contents":[{"uri":"file:///x","text":"a","blob":"aGk="}]}`,
		`{"contents":[{"uri":"file:///x","text":42}]}`,
		`{"contents":[{"uri":"file:///x","blob":"!!not-base64!!"}]}`,
		`{"contents":[{"uri":"file:///x","text":"a","mimeType":42}]}`,
		`{"contents":[{"uri":null,"text":"a"}]}`,
		`{"contents":[{"uri":"file:///x","text":null}]}`,
		`{"contents":[{"uri":"file:///x","blob":null}]}`,
		`{"contents":[{"uri":"file:///x","text":"a","blob":null}]}`,
		`{"contents":[{"uri":"file:///x","text":null,"blob":"aGk="}]}`,
		`{"contents":[{"uri":"file:///x","text":"a","mimeType":null}]}`,
	}
	for _, raw := range invalid {
		err := ValidateResourceResult(json.RawMessage(raw))
		if err == nil {
			t.Errorf("invalid result passed through: %s", raw)
		}
	}
}
