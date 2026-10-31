package mcpclient

import (
	"encoding/base64"
	"encoding/json"
	"fmt"
)

// ValidateResourceResult checks an upstream resources/read result against
// the ReadResourceResult shape before mcpx passes it on (#2754). Until now
// ReadResource returned the upstream bytes untouched and the serve path's
// resourceContents failed open in every direction: null/missing contents
// became a successful empty read, an unparseable result was fabricated as
// a single application/json text entry carrying the raw bytes, and
// malformed entries passed through — so a malformed or hostile upstream
// response reached clients as if mcpx had produced it. An invalid result
// is an error, not a pass-through: the caller fails the resources/read
// with a clear reason. This mirrors ValidatePromptResult (#2193).
//
// The check is structural: the result must be an object carrying a
// contents list; every entry must be an object with a string uri and
// exactly one of text/blob, as a string; a blob must be valid base64.
// mimeType, when present, must be a string. Unknown extra fields pass —
// a revision mcpx does not know yet may add them — but a known field with
// the wrong JSON type fails: decoding into *string fields is the point.
func ValidateResourceResult(raw json.RawMessage) error {
	var probe map[string]json.RawMessage
	if err := json.Unmarshal(raw, &probe); err != nil {
		return fmt.Errorf("a resources/read result must be an object with a contents list: %w", err)
	}
	cv, ok := probe["contents"]
	if !ok || string(cv) == "null" {
		return fmt.Errorf("a resources/read result carries no contents")
	}
	var contents []json.RawMessage
	if err := json.Unmarshal(cv, &contents); err != nil {
		return fmt.Errorf("a resources/read result's contents must be a list: %w", err)
	}
	for i, eraw := range contents {
		if err := validateResourceEntry(eraw); err != nil {
			return fmt.Errorf("contents entry %d: %w", i, err)
		}
	}
	return nil
}

// validateResourceEntry checks one contents entry: an object with a
// string uri and exactly one of text/blob (a string; blob valid base64).
func validateResourceEntry(raw json.RawMessage) error {
	var e struct {
		URI      *string `json:"uri"`
		Text     *string `json:"text"`
		Blob     *string `json:"blob"`
		MimeType *string `json:"mimeType"`
	}
	if err := json.Unmarshal(raw, &e); err != nil {
		return fmt.Errorf("a resource contents entry must be an object with string uri/text/blob fields: %w", err)
	}
	if e.URI == nil || *e.URI == "" {
		return fmt.Errorf("a resource contents entry carries no uri")
	}
	if e.Text != nil && e.Blob != nil {
		return fmt.Errorf("a resource contents entry carries both text and blob; exactly one is required")
	}
	if e.Text == nil && e.Blob == nil {
		return fmt.Errorf("a resource contents entry carries neither text nor blob; exactly one is required")
	}
	if e.Blob != nil {
		if _, err := base64.StdEncoding.DecodeString(*e.Blob); err != nil {
			return fmt.Errorf("a resource contents entry's blob is not base64: %w", err)
		}
	}
	return nil
}
