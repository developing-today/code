package elicit

import (
	"bytes"
	"encoding/json"
	"fmt"
)

// Sampling shape checks (issue #1836, design #1834).
//
// mcpx relays sampling without a model of its own, but relaying is not the
// same as not looking: a request whose messages cannot decode as sampling
// messages, or an answer that is not a CreateMessageResult, is something the
// server must hear as -32602 rather than as a question for the agent to
// puzzle over. The rules are the specification's:
//
//   - SamplingMessage.content is a single block before 2025-11-25 and a
//     single block or an array from 2025-11-25 on.
//   - A user message carrying tool_result blocks MUST contain only tool
//     results, and each toolUseId MUST name a tool_use in an earlier
//     message (2025-11-25 sampling.mdx).
//   - CreateMessageResult carries role and model, and its content obeys the
//     same block rules as a message's.
//   - stopReason "toolUse" and tool_use content mean nothing when the
//     request carried no tools.
//
// A model hint is not checked: hints are expressly ignorable, so a mismatch
// is not a failure.

// sampleBlock is as much of a content block as these checks need.
type sampleBlock struct {
	Type      string `json:"type"`
	ID        string `json:"id"`
	ToolUseID string `json:"toolUseId"`
}

// sampleMessage is one message of a CreateMessageRequest.
type sampleMessage struct {
	Role    string          `json:"role"`
	Content json.RawMessage `json:"content"`
}

// decodeSampleContent reads a message's or result's content as blocks.
//
// arrayOK is false for a revision that allows only a single block; an array
// there is a shape the era does not have. Everything else about a block is
// left to the schema: this checks that content decodes as blocks at all, not
// that every block type is the one a particular revision introduced.
func decodeSampleContent(raw json.RawMessage, arrayOK bool) ([]sampleBlock, error) {
	b := bytes.TrimSpace(raw)
	if len(b) == 0 || string(b) == "null" {
		return nil, fmt.Errorf("content is missing")
	}
	if b[0] == '[' {
		if !arrayOK {
			return nil, fmt.Errorf("content is an array, which this revision does not allow")
		}
		var blocks []sampleBlock
		if err := json.Unmarshal(b, &blocks); err != nil {
			return nil, fmt.Errorf("content is not a list of content blocks: %w", err)
		}
		return blocks, nil
	}
	if b[0] != '{' {
		return nil, fmt.Errorf("content is neither a content block nor a list of them")
	}
	var one sampleBlock
	if err := json.Unmarshal(b, &one); err != nil {
		return nil, fmt.Errorf("content is not a content block: %w", err)
	}
	return []sampleBlock{one}, nil
}

// ValidateSampleRequest checks a CreateMessageRequest's messages against the
// era's shape. arrayOK says whether that era allows array content.
func ValidateSampleRequest(params json.RawMessage, arrayOK bool) error {
	var p struct {
		Messages []sampleMessage `json:"messages"`
	}
	if err := json.Unmarshal(params, &p); err != nil {
		return fmt.Errorf("a CreateMessageRequest must be an object with messages: %w", err)
	}
	// toolUses is every tool_use id seen in an earlier message; a tool
	// result may only name one of those.
	toolUses := map[string]bool{}
	for i, m := range p.Messages {
		if m.Role != "user" && m.Role != "assistant" {
			return fmt.Errorf("message %d has role %q, which sampling does not have", i, m.Role)
		}
		blocks, err := decodeSampleContent(m.Content, arrayOK)
		if err != nil {
			return fmt.Errorf("message %d: %w", i, err)
		}
		var results, others int
		for _, b := range blocks {
			switch b.Type {
			case "":
				return fmt.Errorf("message %d has a content block with no type", i)
			case "tool_result":
				results++
				if b.ToolUseID == "" {
					return fmt.Errorf("message %d has a tool_result with no toolUseId", i)
				}
				if !toolUses[b.ToolUseID] {
					return fmt.Errorf("message %d has a tool_result for %q, which no earlier message used", i, b.ToolUseID)
				}
			case "tool_use":
				others++
				if b.ID == "" {
					return fmt.Errorf("message %d has a tool_use with no id", i)
				}
				toolUses[b.ID] = true
			default:
				others++
			}
		}
		// "A user message containing tool results MUST contain ONLY tool
		// results" (2025-11-25 client/sampling.mdx).
		if results > 0 && others > 0 {
			return fmt.Errorf("message %d mixes tool results with other content", i)
		}
	}
	return nil
}

// ValidateSampleResult checks an answer against CreateMessageResult's shape.
//
// arrayOK is the era's content rule. tools says whether the request carried
// tools: without them a toolUse stop reason or tool_use content is an answer
// to something that was never asked.
func ValidateSampleResult(content json.RawMessage, arrayOK, tools bool) error {
	b := bytes.TrimSpace(content)
	if len(b) == 0 || string(b) == "null" {
		return fmt.Errorf("a sampling answer must be a CreateMessageResult, not null")
	}
	var r struct {
		Role       *string         `json:"role"`
		Model      *string         `json:"model"`
		Content    json.RawMessage `json:"content"`
		StopReason string          `json:"stopReason"`
	}
	if err := json.Unmarshal(b, &r); err != nil {
		return fmt.Errorf("a sampling answer must be a CreateMessageResult: %w", err)
	}
	if r.Role == nil {
		return fmt.Errorf("a sampling answer must carry a role")
	}
	if r.Model == nil || *r.Model == "" {
		return fmt.Errorf("a sampling answer must carry a model")
	}
	blocks, err := decodeSampleContent(r.Content, arrayOK)
	if err != nil {
		return fmt.Errorf("a sampling answer: %w", err)
	}
	if !tools {
		if r.StopReason == "toolUse" {
			return fmt.Errorf("a sampling answer stopped for tool use, but the request carried no tools")
		}
		for _, blk := range blocks {
			if blk.Type == "tool_use" {
				return fmt.Errorf("a sampling answer carries tool_use content, but the request carried no tools")
			}
		}
	}
	return nil
}

// SamplingInvalidParams is a sampling exchange whose shape the era does not
// allow. It carries JSON-RPC Invalid Params (-32602), the InvalidParamsError
// precedent: the caller sent something wrong, which is a protocol error.
type SamplingInvalidParams struct {
	Err error
}

func (e *SamplingInvalidParams) Error() string { return "Invalid params: " + e.Err.Error() }

func (e *SamplingInvalidParams) Unwrap() error { return e.Err }

// RPCCode is the JSON-RPC error code this failure stands for.
func (e *SamplingInvalidParams) RPCCode() int { return -32602 }
