package mcpclient

import (
	"bytes"
	"context"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"fmt"

	"github.com/dezren39/mcpx/internal/schemaval"
)

// This file wires internal/schemaval into the client (Phase 2 of #1481,
// issue #1483). ListTools compiles every tool's input/output schema; a
// tool whose schema does not compile is excluded with a Warning, the
// x-mcp-header precedent. CallTool then enforces the compiled input
// validator before the transport is touched, and checks a result's
// structuredContent against the output validator warn-only.

// toolSchemas is one tool's compiled validators. A nil validator means
// the tool declared no schema of that kind.
type toolSchemas struct {
	hash   string
	input  *schemaval.Schema
	output *schemaval.Schema
	// required names the inputSchema's required top-level properties,
	// parsed from the raw schema at compile time. validateToolArgs uses
	// it to tell an omitted optional property apart from a required one
	// sent as explicit null.
	required map[string]bool
}

// InvalidParamsError is a CallTool argument set that failed validation
// against the tool's inputSchema. It carries the JSON-RPC Invalid Params
// code (-32602): the caller sent something wrong, which is a protocol
// error, never a tool failure — surfaces that translate generic call
// errors into isError results must pass it through (see RPCCode).
type InvalidParamsError struct {
	Tool string
	Err  error
}

func (e *InvalidParamsError) Error() string {
	return fmt.Sprintf("Invalid params: tool %q: %v", e.Tool, e.Err)
}

func (e *InvalidParamsError) Unwrap() error { return e.Err }

// RPCCode is the JSON-RPC error code this failure stands for. The
// gateway's invoke path checks for it (an interface assertion, so no
// import cycle) and relays the error as a protocol error.
func (e *InvalidParamsError) RPCCode() int { return codeInvalidParams }

// schemaHash keys the compiled-validator cache: a re-list recompiles
// only a tool whose schema bytes changed.
func schemaHash(t Tool) string {
	h := sha256.New()
	h.Write(t.InputSchema)
	h.Write([]byte{0})
	h.Write(t.OutputSchema)
	return hex.EncodeToString(h.Sum(nil))
}

// checkToolSchemas compiles each tool's schemas and drops the tools
// whose schemas do not compile, warning about each. It runs on every
// connection (unlike checkToolHeaders, which is modern-HTTP only):
// argument enforcement is a property of the choke point, not the era.
func (c *Client) checkToolSchemas(tools []Tool) []Tool {
	c.mu.Lock()
	prev := c.toolValidators
	c.mu.Unlock()
	next := make(map[string]*toolSchemas, len(tools))
	kept := tools[:0:0]
	for _, t := range tools {
		if len(t.InputSchema) == 0 && len(t.OutputSchema) == 0 {
			next[t.Name] = &toolSchemas{hash: schemaHash(t)}
			kept = append(kept, t)
			continue
		}
		hash := schemaHash(t)
		if old, ok := prev[t.Name]; ok && old.hash == hash && (old.input != nil || len(t.InputSchema) == 0) && (old.output != nil || len(t.OutputSchema) == 0) {
			next[t.Name] = old
			kept = append(kept, t)
			continue
		}
		ts := &toolSchemas{hash: hash}
		bad := ""
		if len(t.InputSchema) > 0 {
			s, err := schemaval.Compile(t.InputSchema)
			if err != nil {
				bad = fmt.Sprintf("inputSchema: %v", err)
			} else {
				ts.input = s
				ts.required = requiredProps(t.InputSchema)
			}
		}
		if bad == "" && len(t.OutputSchema) > 0 {
			s, err := schemaval.Compile(t.OutputSchema)
			if err != nil {
				bad = fmt.Sprintf("outputSchema: %v", err)
			} else {
				ts.output = s
			}
		}
		if bad != "" {
			c.warn(Warning{Tool: t.Name, Reason: "excluded: schema does not compile: " + bad})
			continue
		}
		next[t.Name] = ts
		kept = append(kept, t)
	}
	c.mu.Lock()
	c.toolValidators = next
	c.mu.Unlock()
	return kept
}

// validatorsFor returns the tool's compiled validators. Validation
// applies only once a list has happened on this connection: the pool
// lists a server's tools before it ever calls them, so the choke
// point is covered in production, while a connection that has never
// listed behaves exactly as before validation existed (no surprise
// tools/list is issued on a call's behalf). ok is false when the tool
// is not in the list — such a call is the upstream's to answer for.
func (c *Client) validatorsFor(name string) (*toolSchemas, bool) {
	c.mu.Lock()
	defer c.mu.Unlock()
	ts, ok := c.toolValidators[name]
	return ts, ok
}

// validateToolArgs enforces the tool's compiled input validator against
// the marshalled arguments, before any transport use.
func (c *Client) validateToolArgs(ctx context.Context, name string, argsJSON json.RawMessage) error {
	ts, ok := c.validatorsFor(name)
	if !ok || ts.input == nil {
		return nil
	}
	if err := ts.input.ValidateJSON(validationCopy(ts, argsJSON)); err != nil {
		return &InvalidParamsError{Tool: name, Err: err}
	}
	return nil
}

// requiredProps extracts the top-level required property names from a
// raw inputSchema. A schema whose required list does not parse simply
// yields none; Compile has already judged the schema itself.
func requiredProps(raw json.RawMessage) map[string]bool {
	var doc struct {
		Required []string `json:"required"`
	}
	if err := json.Unmarshal(raw, &doc); err != nil || len(doc.Required) == 0 {
		return nil
	}
	out := make(map[string]bool, len(doc.Required))
	for _, name := range doc.Required {
		out[name] = true
	}
	return out
}

// validationCopy returns the argument bytes to validate. An explicit
// JSON null for a property the schema does not require is an omission,
// not a value: SEP-2243 treats a null parameter as not provided (its
// header must be omitted), and the conformance suite calls tools that
// way -- a boolean-typed optional parameter sent as null. The copy
// drops those keys; the caller's original arguments still go on the
// wire unchanged. A null for a required property stays in the copy and
// passes or fails exactly as the schema says.
func validationCopy(ts *toolSchemas, argsJSON json.RawMessage) json.RawMessage {
	var obj map[string]json.RawMessage
	if err := json.Unmarshal(argsJSON, &obj); err != nil {
		return argsJSON
	}
	changed := false
	for key, value := range obj {
		if string(bytes.TrimSpace(value)) == "null" && !ts.required[key] {
			delete(obj, key)
			changed = true
		}
	}
	if !changed {
		return argsJSON
	}
	out, err := json.Marshal(obj)
	if err != nil {
		return argsJSON
	}
	return out
}

// checkToolOutput validates a result's structuredContent against the
// tool's output validator, warn-only: the result is delivered unchanged
// either way (the enforcement flip is an open question on #1481).
func (c *Client) checkToolOutput(ctx context.Context, name string, raw json.RawMessage) {
	ts, ok := c.validatorsFor(name)
	if !ok || ts.output == nil {
		return
	}
	var res struct {
		StructuredContent json.RawMessage `json:"structuredContent"`
	}
	if err := json.Unmarshal(raw, &res); err != nil || len(res.StructuredContent) == 0 {
		return
	}
	if err := ts.output.ValidateJSON(res.StructuredContent); err != nil {
		c.warn(Warning{Tool: name, Reason: "structuredContent violates outputSchema (delivered unchanged): " + err.Error()})
	}
}
