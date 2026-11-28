package mcpclient

import (
	"encoding/base64"
	"encoding/json"
	"fmt"
)

// ValidatePromptResult checks an upstream prompts/get result against the
// GetPromptResult shape before mcpx passes it on (#2193, conformance row
// prompts-validate-io). Until now GetPrompt returned the upstream bytes
// untouched, so a malformed or hostile upstream response reached clients
// as if mcpx had produced it. An invalid result is an error, not a
// pass-through: the caller fails the prompts/get with a clear reason.
//
// The check is structural, mirroring elicit.ValidateSampleResult: the
// result must be an object carrying a messages list; every message must
// carry a user/assistant role and content (one block, or a list of blocks
// in the revisions that allow it); every block must carry the fields its
// type requires. Unknown block types pass — a revision mcpx does not know
// yet may add blocks, and refusing them would break valid traffic — but a
// block with no type, or a known type missing its required fields, fails.
func ValidatePromptResult(raw json.RawMessage) error {
	var res struct {
		Description *string           `json:"description"`
		Messages    []json.RawMessage `json:"messages"`
	}
	if err := json.Unmarshal(raw, &res); err != nil {
		return fmt.Errorf("a prompts/get result must be an object with a messages list: %w", err)
	}
	var probe map[string]json.RawMessage
	if err := json.Unmarshal(raw, &probe); err != nil {
		return fmt.Errorf("a prompts/get result must be an object: %w", err)
	}
	mv, ok := probe["messages"]
	if !ok || string(mv) == "null" {
		return fmt.Errorf("a prompts/get result carries no messages")
	}
	for i, mraw := range res.Messages {
		if err := validatePromptMessage(mraw); err != nil {
			return fmt.Errorf("message %d: %w", i, err)
		}
	}
	return nil
}

// validatePromptMessage checks one PromptMessage: a role sampling and
// prompts share (user or assistant) and content in spec shape.
func validatePromptMessage(raw json.RawMessage) error {
	var m struct {
		Role    *string         `json:"role"`
		Content json.RawMessage `json:"content"`
	}
	if err := json.Unmarshal(raw, &m); err != nil {
		return fmt.Errorf("a prompt message must be an object: %w", err)
	}
	if m.Role == nil || (*m.Role != "user" && *m.Role != "assistant") {
		return fmt.Errorf("a prompt message must carry role \"user\" or \"assistant\"")
	}
	if len(m.Content) == 0 {
		return fmt.Errorf("a prompt message carries no content")
	}
	blocks, err := promptContentBlocks(m.Content)
	if err != nil {
		return err
	}
	if len(blocks) == 0 {
		return fmt.Errorf("a prompt message carries no content blocks")
	}
	for j, b := range blocks {
		if err := validatePromptBlock(b); err != nil {
			return fmt.Errorf("content block %d: %w", j, err)
		}
	}
	return nil
}

// promptContentBlocks accepts either a single block object or a list of
// blocks; revisions differ on which shape a prompt message uses.
func promptContentBlocks(raw json.RawMessage) ([]json.RawMessage, error) {
	trimmed := raw
	for len(trimmed) > 0 && (trimmed[0] == ' ' || trimmed[0] == '\n' || trimmed[0] == '\t' || trimmed[0] == '\r') {
		trimmed = trimmed[1:]
	}
	if len(trimmed) == 0 {
		return nil, fmt.Errorf("content is empty")
	}
	if trimmed[0] == '[' {
		var blocks []json.RawMessage
		if err := json.Unmarshal(raw, &blocks); err != nil {
			return nil, fmt.Errorf("content list is malformed: %w", err)
		}
		return blocks, nil
	}
	if trimmed[0] != '{' {
		return nil, fmt.Errorf("content must be a block or a list of blocks")
	}
	return []json.RawMessage{raw}, nil
}

// validatePromptBlock checks the fields a block's type requires: a text
// block's text, an image/audio block's base64 data and mimeType, an
// embedded resource's resource object (uri plus text or blob), and a
// resource_link's uri and name. Decoding into *string fields is the
// point: a field present with the wrong JSON type fails.
func validatePromptBlock(raw json.RawMessage) error {
	var head struct {
		Type *string `json:"type"`
	}
	if err := json.Unmarshal(raw, &head); err != nil {
		return fmt.Errorf("a content block must be an object: %w", err)
	}
	if head.Type == nil || *head.Type == "" {
		return fmt.Errorf("a content block carries no type")
	}
	switch *head.Type {
	case "text":
		var b struct {
			Text *string `json:"text"`
		}
		if err := json.Unmarshal(raw, &b); err != nil {
			return fmt.Errorf("a text block's text is not a string: %w", err)
		}
		if b.Text == nil {
			return fmt.Errorf("a text block carries no text")
		}
	case "image", "audio":
		var b struct {
			Data     *string `json:"data"`
			MimeType *string `json:"mimeType"`
		}
		if err := json.Unmarshal(raw, &b); err != nil {
			return fmt.Errorf("an %s block's data or mimeType is not a string: %w", *head.Type, err)
		}
		if b.Data == nil || b.MimeType == nil {
			return fmt.Errorf("an %s block must carry data and mimeType", *head.Type)
		}
		if _, err := base64.StdEncoding.DecodeString(*b.Data); err != nil {
			return fmt.Errorf("an %s block's data is not base64: %w", *head.Type, err)
		}
	case "resource":
		var b struct {
			Resource struct {
				URI      *string `json:"uri"`
				Text     *string `json:"text"`
				Blob     *string `json:"blob"`
				MimeType *string `json:"mimeType"`
			} `json:"resource"`
		}
		if err := json.Unmarshal(raw, &b); err != nil {
			return fmt.Errorf("an embedded resource block is malformed: %w", err)
		}
		if b.Resource.URI == nil || *b.Resource.URI == "" {
			return fmt.Errorf("an embedded resource carries no uri")
		}
		if b.Resource.Text == nil && b.Resource.Blob == nil {
			return fmt.Errorf("an embedded resource must carry text or blob")
		}
		if b.Resource.Blob != nil {
			if _, err := base64.StdEncoding.DecodeString(*b.Resource.Blob); err != nil {
				return fmt.Errorf("an embedded resource's blob is not base64: %w", err)
			}
		}
	case "resource_link":
		var b struct {
			URI  *string `json:"uri"`
			Name *string `json:"name"`
		}
		if err := json.Unmarshal(raw, &b); err != nil {
			return fmt.Errorf("a resource_link block's uri or name is not a string: %w", err)
		}
		if b.URI == nil || *b.URI == "" || b.Name == nil {
			return fmt.Errorf("a resource_link block must carry uri and name")
		}
	}
	return nil
}
