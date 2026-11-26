package mcpclient

import (
	"context"
	"encoding/json"
	"net"
	"net/http"
	"os"
	"path/filepath"
	"testing"
)

func TestUnixHTTPTransportInitializeListAndCall(t *testing.T) {
	socket := filepath.Join(t.TempDir(), "mcp.sock")
	ln, err := net.Listen("unix", socket)
	if err != nil {
		t.Fatal(err)
	}
	defer ln.Close()
	server := &http.Server{Handler: http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		var req struct {
			ID     json.RawMessage `json:"id"`
			Method string          `json:"method"`
		}
		if err := json.NewDecoder(r.Body).Decode(&req); err != nil {
			http.Error(w, err.Error(), http.StatusBadRequest)
			return
		}
		var result any
		switch req.Method {
		case "notifications/initialized":
			return
		case "initialize":
			result = map[string]any{"protocolVersion": "2025-03-26", "capabilities": map[string]any{"tools": map[string]any{}}, "serverInfo": map[string]any{"name": "mcpx", "version": "test", "instance": "unix-fixture"}}
		case "tools/list":
			result = map[string]any{"tools": []any{map[string]any{"name": "echo", "inputSchema": map[string]any{"type": "object"}}}}
		case "tools/call":
			result = map[string]any{"content": []any{map[string]any{"type": "text", "text": "called"}}}
		default:
			http.Error(w, "unexpected method", http.StatusBadRequest)
			return
		}
		w.Header().Set("Content-Type", "application/json")
		_ = json.NewEncoder(w).Encode(map[string]any{"jsonrpc": "2.0", "id": req.ID, "result": result})
	})}
	go func() { _ = server.Serve(ln) }()
	t.Cleanup(func() { _ = server.Close() })

	tr, err := NewHTTP(HTTPOptions{URL: "http://mcpx-unix.invalid/mcp", UnixSocket: socket})
	if err != nil {
		t.Fatal(err)
	}
	client, err := NewWithPreference(context.Background(), tr, "test", "1", ForceLegacy)
	if err != nil {
		t.Fatal(err)
	}
	defer client.Close()
	if got := client.ServerInfo.Instance; got != "unix-fixture" {
		t.Fatalf("initialize instance = %q, want unix-fixture", got)
	}
	tools, err := client.ListTools(context.Background())
	if err != nil {
		t.Fatal(err)
	}
	if len(tools) != 1 || tools[0].Name != "echo" {
		t.Fatalf("tools/list = %#v, want echo", tools)
	}
	result, err := client.CallTool(context.Background(), "echo", map[string]any{})
	if err != nil {
		t.Fatal(err)
	}
	var call struct {
		Content []struct {
			Text string `json:"text"`
		} `json:"content"`
	}
	if err := json.Unmarshal(result, &call); err != nil {
		t.Fatal(err)
	}
	if len(call.Content) != 1 || call.Content[0].Text != "called" {
		t.Fatalf("tools/call result = %s", result)
	}
	if _, err := os.Stat(socket); err != nil {
		t.Fatalf("socket was not created: %v", err)
	}
}
