package mcpclient

import (
	"encoding/json"
	"fmt"

	"github.com/dezren39/mcpx/internal/schemaval"
)

// This file wires internal/schemaval into elicitation answers (Phase 3
// of #1481, issue #1484). Client.answer is the single choke point every
// elicitation answer passes through — a server's own elicitation/create
// request and a 2026-07-28 input_required round alike — so the check
// lives there, after applyDefaults has filled omitted fields from the
// schema's defaults: an omitted field is valid only once its default
// is in.

// checkElicitationAnswer validates an accepted form answer's content
// against the requestedSchema the requesting server itself supplied.
//
//   - A validation failure is Invalid Params (-32602) to the requesting
//     server, naming the failing paths, plus a Warning; the answer is
//     NOT relayed — the unvalidated answer never reaches the upstream.
//   - A requestedSchema that does not compile is the server's own
//     defect, not the answerer's: warn and relay the answer
//     unvalidated rather than wedging the user's answer on a schema
//     they never wrote.
//   - Decline/cancel answers (no content to validate), URL mode, and
//     requests without a requestedSchema pass through untouched.
func (c *Client) checkElicitationAnswer(out any, params json.RawMessage) (any, *rpcError) {
	var p struct {
		Mode            string          `json:"mode"`
		RequestedSchema json.RawMessage `json:"requestedSchema"`
	}
	if json.Unmarshal(params, &p) != nil || len(p.RequestedSchema) == 0 {
		return out, nil
	}
	// Undeclared mode is form (the codebase already treats it so, and
	// elicitModeDeclared has already refused any other undeclared
	// mode); a declared non-form mode — URL — carries no form content
	// to validate.
	if p.Mode != "" && p.Mode != "form" {
		return out, nil
	}
	b, err := json.Marshal(out)
	if err != nil {
		return out, nil
	}
	var res map[string]json.RawMessage
	if json.Unmarshal(b, &res) != nil {
		return out, nil
	}
	var action string
	_ = json.Unmarshal(res["action"], &action)
	if action != "accept" {
		return out, nil
	}
	content, ok := res["content"]
	if !ok || string(content) == "null" {
		return out, nil
	}
	schema, err := schemaval.Compile(p.RequestedSchema)
	if err != nil {
		c.warn(Warning{Reason: fmt.Sprintf("elicitation answer relayed unvalidated: requestedSchema does not compile: %v", err)})
		return out, nil
	}
	if err := schema.ValidateJSON(content); err != nil {
		c.warn(Warning{Reason: fmt.Sprintf("elicitation answer rejected: content does not match requestedSchema: %v", err)})
		return nil, &rpcError{Code: codeInvalidParams,
			Message: fmt.Sprintf("elicitation answer does not match requestedSchema: %v", err)}
	}
	return out, nil
}
