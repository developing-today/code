package execsvc

import (
	"encoding/json"
	"sort"
)

// SyntheticResult builds the result a dry run returns for a call it
// intercepted (#1927): an MCP tool result whose structured content is
// synthesised from the tool's declared outputSchema, so a script
// written against the real shape keeps running and the caller can
// see what would have been called. A tool that declares no
// outputSchema gets a marked placeholder object rather than an
// invented shape: any value is valid where no schema constrains it,
// and the marker says plainly that the value is synthetic.
//
// Synthesis is deterministic: the same schema always produces the
// same value (zero values per type, the first enum member, object
// properties in sorted order), so two dry runs of one script are
// comparable byte for byte.
func SyntheticResult(outputSchema json.RawMessage) json.RawMessage {
	var structured any
	if len(outputSchema) > 0 {
		var schema map[string]any
		if json.Unmarshal(outputSchema, &schema) == nil {
			structured = synthValue(schema)
		}
	}
	if structured == nil {
		structured = map[string]any{"dryRun": true}
	}
	text, err := json.Marshal(structured)
	if err != nil {
		text = []byte(`{"dryRun":true}`)
	}
	out, err := json.Marshal(map[string]any{
		"content":           []any{map[string]any{"type": "text", "text": string(text)}},
		"structuredContent": structured,
		"isError":           false,
		"dryRun":            true,
	})
	if err != nil {
		return json.RawMessage(`{"content":[{"type":"text","text":"dry run"}],"isError":false,"dryRun":true}`)
	}
	return out
}

// synthValue is the deterministic zero value of one JSON Schema.
func synthValue(schema map[string]any) any {
	if schema == nil {
		return nil
	}
	if enum, ok := schema["enum"].([]any); ok && len(enum) > 0 {
		return enum[0]
	}
	for _, key := range []string{"anyOf", "oneOf"} {
		if branches, ok := schema[key].([]any); ok && len(branches) > 0 {
			if first, ok := branches[0].(map[string]any); ok {
				return synthValue(first)
			}
		}
	}
	typ, _ := schema["type"].(string)
	switch typ {
	case "object":
		props, _ := schema["properties"].(map[string]any)
		out := make(map[string]any, len(props))
		names := make([]string, 0, len(props))
		for name := range props {
			names = append(names, name)
		}
		sort.Strings(names)
		for _, name := range names {
			sub, _ := props[name].(map[string]any)
			out[name] = synthValue(sub)
		}
		return out
	case "array":
		return []any{}
	case "string":
		return ""
	case "integer", "number":
		return 0
	case "boolean":
		return false
	case "null":
		return nil
	}
	// No type: a schema with properties is an object in practice.
	if props, ok := schema["properties"].(map[string]any); ok && len(props) > 0 {
		return synthValue(map[string]any{"type": "object", "properties": props})
	}
	return map[string]any{"dryRun": true}
}
