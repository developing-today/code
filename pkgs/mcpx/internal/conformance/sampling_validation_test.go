package conformance_test

import (
	"encoding/json"
	"testing"

	"github.com/dezren39/mcpx/internal/elicit"
)

// TestSamplingValidation covers sampling-both-validate-message-content on
// both sides: mcpx validates a sampling request's messages on the way in and
// the result's shape on the way out, at one choke point (#1836, #1834).
func TestSamplingValidation(t *testing.T) {
	// Before 2025-11-25 content is a single block; from 2025-11-25 it may be
	// an array (SMP-05). Both sides of the exchange follow the same rule.
	revs := []struct {
		rev     string
		arrayOK bool
	}{
		{"2024-11-05", false},
		{"2025-03-26", false},
		{"2025-06-18", false},
		{"2025-11-25", true},
		{"2026-07-28", true},
	}
	for _, c := range revs {
		t.Run(c.rev+"/sampling/sampling-both-validate-message-content", func(t *testing.T) {
			// Client side: what a server sent us must decode as sampling
			// messages for the revision in use.
			array := json.RawMessage(`{"messages":[{"role":"user","content":[{"type":"text","text":"hi"}]}]}`)
			err := elicit.ValidateSampleRequest(array, c.arrayOK)
			if c.arrayOK && err != nil {
				t.Errorf("an array was refused from 2025-11-25, which allows one: %v", err)
			}
			if !c.arrayOK && err == nil {
				t.Error("an array was accepted before 2025-11-25, which allows only a single block")
			}
			unpaired := json.RawMessage(`{"messages":[{"role":"user","content":[{"type":"tool_result","toolUseId":"x"}]}]}`)
			if err := elicit.ValidateSampleRequest(unpaired, true); err == nil {
				t.Error("a tool_result naming no earlier tool_use was accepted")
			}

			// Server side: what we hand back must be a CreateMessageResult,
			// and tool use only answers a request that carried tools.
			noModel := json.RawMessage(`{"role":"assistant","content":{"type":"text","text":"hi"}}`)
			if err := elicit.ValidateSampleResult(noModel, c.arrayOK, false); err == nil {
				t.Error("a result with no model was accepted")
			}
			toolUse := json.RawMessage(`{"role":"assistant","model":"m","content":[{"type":"tool_use","id":"t1","name":"x","input":{}}],"stopReason":"toolUse"}`)
			if c.arrayOK {
				// Tool use is a 2025-11-25 shape, array content and all.
				if err := elicit.ValidateSampleResult(toolUse, c.arrayOK, false); err == nil {
					t.Error("tool use was accepted against a request that carried no tools")
				}
				if err := elicit.ValidateSampleResult(toolUse, c.arrayOK, true); err != nil {
					t.Errorf("tool use was refused against a request that carried tools: %v", err)
				}
			} else if err := elicit.ValidateSampleResult(toolUse, c.arrayOK, true); err == nil {
				t.Error("tool use was accepted in a revision that has no tool use")
			}
		})
	}
}
