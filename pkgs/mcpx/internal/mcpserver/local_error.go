package mcpserver

import "encoding/json"

// Error honesty (#2434 Phase 1): a client must be able to tell an error
// mcpx minted itself from an error mcpx is relaying for an upstream
// peer. Every locally minted error on the MCP surface therefore carries
// {"mcpx": {"origin": "local"}} in its data, merged into any data the
// error already has; a relayed peer error carries no marker, so the
// absence of the marker means the error came from somewhere else.

// mintLocalError is the minting helper for mcpx's own errors on the MCP
// surface. The origin marker itself is stamped at marshal time (see
// MarshalJSON below), so errors minted as bare rpcError literals are
// stamped too; the helper exists so new mint sites have one obvious
// constructor to reach for.
func mintLocalError(code int, message string, data any) *rpcError {
	return &rpcError{Code: code, Message: message, Data: data}
}

// withLocalOrigin returns data with the local origin marker merged in.
// Existing keys survive, including any other keys under "mcpx"; a
// non-object payload is carried under "data" so the marker still has
// an object to live in.
func withLocalOrigin(data any) any {
	mark := func(m map[string]any) map[string]any {
		if inner, ok := m["mcpx"].(map[string]any); ok {
			merged := make(map[string]any, len(inner)+1)
			for k, v := range inner {
				merged[k] = v
			}
			merged["origin"] = "local"
			m["mcpx"] = merged
		} else {
			m["mcpx"] = map[string]any{"origin": "local"}
		}
		return m
	}
	switch d := data.(type) {
	case nil:
		return mark(map[string]any{})
	case map[string]any:
		out := make(map[string]any, len(d)+1)
		for k, v := range d {
			out[k] = v
		}
		return mark(out)
	case json.RawMessage:
		if len(d) == 0 {
			return mark(map[string]any{})
		}
		var v any
		if err := json.Unmarshal(d, &v); err != nil {
			return data
		}
		return withLocalOrigin(v)
	default:
		raw, err := json.Marshal(d)
		if err != nil {
			return data
		}
		var v any
		if err := json.Unmarshal(raw, &v); err != nil {
			return data
		}
		if _, isObj := v.(map[string]any); isObj {
			return withLocalOrigin(v)
		}
		return mark(map[string]any{"data": v})
	}
}

// MarshalJSON stamps the local origin marker into a locally minted
// error's data. An error marked relayed -- one that arrived from an
// upstream peer and is answered verbatim, see UpstreamError.relay --
// is marshalled untouched.
func (e *rpcError) MarshalJSON() ([]byte, error) {
	type wire struct {
		Code    int    `json:"code"`
		Message string `json:"message"`
		Data    any    `json:"data,omitempty"`
	}
	data := e.Data
	if !e.relayed {
		data = withLocalOrigin(data)
	}
	return json.Marshal(wire{Code: e.Code, Message: e.Message, Data: data})
}
