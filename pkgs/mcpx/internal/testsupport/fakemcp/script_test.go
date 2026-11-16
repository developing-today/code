package main

import (
	"encoding/json"
	"os"
	"path/filepath"
	"testing"
	"time"
)

// Oracle for these tests: the script JSON written in each test is the
// behaviour the server must show — a scripted tool answers exactly
// what its response declares, a scripted name replaces the compiled-in
// tool of the same name, and no script (or a broken one) leaves the
// compiled-in behaviour untouched (#1980 stage 4).

const testScript = `{
  "failStart": true,
  "startDelayMs": 250,
  "tools": [
    {"name": "greet", "description": "Say hello",
     "inputSchema": {"type": "object", "properties": {"who": {"type": "string"}}},
     "response": {"text": "hello"}},
    {"name": "flaky", "response": {"error": {"code": -32603, "message": "down"}}},
    {"name": "slowpoke", "response": {"delayMs": 60, "text": "done"}},
    {"name": "mirror", "response": {"echoArgs": true}},
    {"name": "echo", "response": {"text": "scripted echo"}}
  ]
}`

func writeScript(t *testing.T, body string) {
	t.Helper()
	path := filepath.Join(t.TempDir(), "script.json")
	if err := os.WriteFile(path, []byte(body), 0o644); err != nil {
		t.Fatal(err)
	}
	t.Setenv("FAKEMCP_SCRIPT_FILE", path)
	scriptMu.Lock()
	scriptCached, scriptStamp = nil, ""
	scriptMu.Unlock()
	t.Cleanup(func() {
		scriptMu.Lock()
		scriptCached, scriptStamp = nil, ""
		scriptMu.Unlock()
	})
}

func call(t *testing.T, name string, args map[string]any) map[string]any {
	t.Helper()
	id := int64(7)
	params, _ := json.Marshal(map[string]any{"name": name, "arguments": args})
	resp := callTool(req{ID: &id, Method: "tools/call", Params: params})
	if resp == nil {
		t.Fatalf("call %s: no response", name)
	}
	return resp
}

func resultText(t *testing.T, resp map[string]any) string {
	t.Helper()
	result, ok := resp["result"].(map[string]any)
	if !ok {
		t.Fatalf("response has no result: %v", resp)
	}
	content, ok := result["content"].([]map[string]any)
	if !ok || len(content) == 0 {
		t.Fatalf("result has no content: %v", result)
	}
	text, _ := content[0]["text"].(string)
	return text
}

func TestLoadScriptParsesStartBehaviourAndTools(t *testing.T) {
	writeScript(t, testScript)
	s := loadScript()
	if s == nil {
		t.Fatal("loadScript returned nil for a valid script")
	}
	if !s.FailStart || s.StartDelayMs != 250 {
		t.Fatalf("start behaviour = failStart %v delay %d, want true/250", s.FailStart, s.StartDelayMs)
	}
	if len(s.Tools) != 5 {
		t.Fatalf("script has %d tools, want 5", len(s.Tools))
	}
}

func TestLoadScriptOffAndBroken(t *testing.T) {
	t.Setenv("FAKEMCP_SCRIPT_FILE", "")
	if s := loadScript(); s != nil {
		t.Fatalf("unset script file: got %v, want nil", s)
	}
	writeScript(t, "{not json")
	if s := loadScript(); s != nil {
		t.Fatalf("malformed script: got %v, want nil", s)
	}
	t.Setenv("FAKEMCP_SCRIPT_FILE", filepath.Join(t.TempDir(), "missing.json"))
	scriptMu.Lock()
	scriptCached, scriptStamp = nil, ""
	scriptMu.Unlock()
	if s := loadScript(); s != nil {
		t.Fatalf("missing script file: got %v, want nil", s)
	}
}

func TestScriptedToolsJoinAndReplaceInList(t *testing.T) {
	writeScript(t, testScript)
	var greet, echo map[string]any
	for _, d := range toolDefs() {
		switch d["name"] {
		case "greet":
			greet = d
		case "echo":
			echo = d
		}
	}
	if greet == nil {
		t.Fatal("scripted tool greet missing from tools/list")
	}
	if greet["description"] != "Say hello" {
		t.Fatalf("greet description = %v", greet["description"])
	}
	raw, _ := json.Marshal(greet["inputSchema"])
	var schema struct {
		Properties map[string]struct {
			Type string `json:"type"`
		} `json:"properties"`
	}
	if err := json.Unmarshal(raw, &schema); err != nil {
		t.Fatalf("greet inputSchema does not round-trip: %v (%s)", err, raw)
	}
	if schema.Properties["who"].Type != "string" {
		t.Fatalf("greet inputSchema = %s, want a string property 'who'", raw)
	}
	if echo == nil || echo["description"] != "" {
		// The scripted echo declares no description; the compiled-in
		// one ("Echo a message back...") must not survive the replace.
		t.Fatalf("compiled-in echo not replaced: %v", echo)
	}
}

func TestScriptedCallAnswers(t *testing.T) {
	writeScript(t, testScript)
	if got := resultText(t, call(t, "greet", map[string]any{"who": "x"})); got != "hello" {
		t.Fatalf("greet answered %q, want hello", got)
	}
	if got := resultText(t, call(t, "mirror", map[string]any{"a": "b"})); got != `{"a":"b"}` {
		t.Fatalf("mirror answered %q, want the arguments as JSON", got)
	}
	resp := call(t, "flaky", map[string]any{})
	errObj, ok := resp["error"].(map[string]any)
	if !ok {
		t.Fatalf("flaky response has no error: %v", resp)
	}
	if errObj["message"] != "down" {
		t.Fatalf("flaky error = %v", errObj)
	}
}

func TestScriptedCallOverridesBuiltIn(t *testing.T) {
	writeScript(t, testScript)
	if got := resultText(t, call(t, "echo", map[string]any{"message": "hi"})); got != "scripted echo" {
		t.Fatalf("echo answered %q, want the scripted override", got)
	}
}

func TestScriptedDelayIsHonoured(t *testing.T) {
	writeScript(t, testScript)
	start := time.Now()
	if got := resultText(t, call(t, "slowpoke", map[string]any{})); got != "done" {
		t.Fatalf("slowpoke answered %q, want done", got)
	}
	if elapsed := time.Since(start); elapsed < 60*time.Millisecond {
		t.Fatalf("slowpoke returned after %v, want at least its 60ms delay", elapsed)
	}
}

func TestNoScriptKeepsBuiltInBehaviour(t *testing.T) {
	t.Setenv("FAKEMCP_SCRIPT_FILE", "")
	if got := resultText(t, call(t, "echo", map[string]any{"message": "hi"})); got != "hi" {
		t.Fatalf("built-in echo answered %q, want hi", got)
	}
	for _, d := range toolDefs() {
		if d["name"] == "greet" {
			t.Fatal("greet listed with no script set")
		}
	}
}
