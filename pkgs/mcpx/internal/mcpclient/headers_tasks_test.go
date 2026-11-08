package mcpclient

import "testing"

// TestStandardHeadersTaskID: the tasks extension routes on Mcp-Name
// carrying the taskId (SEP-2663), mirroring mcpserver.nameOf.
func TestStandardHeadersTaskID(t *testing.T) {
	for _, method := range []string{"tasks/get", "tasks/update", "tasks/cancel"} {
		msg := []byte(`{"jsonrpc":"2.0","id":1,"method":"` + method + `","params":{"taskId":"t-9"}}`)
		h := standardHeaders(msg)
		if h["Mcp-Method"] != method || h["Mcp-Name"] != "t-9" {
			t.Fatalf("%s headers = %v, want Mcp-Name t-9", method, h)
		}
	}
	// A task method without a taskId carries no Mcp-Name.
	h := standardHeaders([]byte(`{"jsonrpc":"2.0","id":1,"method":"tasks/get","params":{}}`))
	if _, ok := h["Mcp-Name"]; ok {
		t.Fatalf("headers = %v, want no Mcp-Name", h)
	}
}
