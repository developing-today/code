package main

import (
	"encoding/json"
	"fmt"
	"os"
	"sync"
	"time"
)

// Scriptable behaviour (#1980 stage 4).
//
// Until now every behaviour a test needed from this server was compiled
// in: a new tool, failure mode, or start behaviour meant editing this
// binary, and every test using it shared the result. The FAKEMCP_* env
// knobs were the only per-test control, and each new knob was a new
// compiled-in branch.
//
// FAKEMCP_SCRIPT_FILE names a JSON file a test writes before starting
// the server; the file declares the behaviour that test needs:
//
//	{
//	  "failStart": true,
//	  "startDelayMs": 250,
//	  "tools": [
//	    {"name": "greet", "description": "Say hello",
//	     "inputSchema": {"type": "object", "properties": {}},
//	     "response": {"text": "hello"}},
//	    {"name": "flaky",
//	     "response": {"error": {"code": -32603, "message": "down"}}},
//	    {"name": "slowpoke", "response": {"delayMs": 100, "text": "done"}},
//	    {"name": "mirror", "response": {"echoArgs": true}}
//	  ]
//	}
//
// Scripted tools join tools/list; a scripted tool whose name matches a
// compiled-in tool replaces it, in the list and in dispatch —
// overriding a built-in for one test is the point of scripting. The
// script is re-read when it changes, like FAKEMCP_SCHEMA_FILE, so a
// test can change behaviour under a running server. A missing or
// malformed script file is not an error: the server behaves exactly as
// it did before scripts existed, so a bad script fails the test that
// reads the answer, not the harness. The env knobs keep working
// unchanged; the script's failStart and startDelayMs are additional
// ways to declare the same two start behaviours, for a test that
// already writes a script and need not also thread env knobs through
// the harness.
type fakeScript struct {
	FailStart    bool         `json:"failStart"`
	StartDelayMs int          `json:"startDelayMs"`
	Tools        []scriptTool `json:"tools"`
}

type scriptTool struct {
	Name        string          `json:"name"`
	Description string          `json:"description"`
	InputSchema json.RawMessage `json:"inputSchema"`
	Response    scriptResponse  `json:"response"`
}

type scriptResponse struct {
	Text     string       `json:"text"`
	DelayMs  int          `json:"delayMs"`
	EchoArgs bool         `json:"echoArgs"`
	Error    *scriptError `json:"error"`
	// IsError answers with an MCP tool-error result (isError: true)
	// carrying Text, instead of a successful result or a JSON-RPC
	// error: the shape a real server's failed tool call takes.
	IsError bool `json:"isError"`
	// IfEnv makes the response conditional on this process's
	// environment: when the named variable equals Equals this
	// response is used, otherwise Else is used (which may itself be
	// conditional). It is how a test makes a server's answer depend
	// on the credential it was spawned with.
	IfEnv *scriptEnvCond  `json:"ifEnv,omitempty"`
	Else  *scriptResponse `json:"else,omitempty"`
}

// scriptEnvCond is the condition on a scriptResponse's IfEnv branch.
type scriptEnvCond struct {
	Name   string `json:"name"`
	Equals string `json:"equals"`
}

type scriptError struct {
	Code    int    `json:"code"`
	Message string `json:"message"`
}

var (
	scriptMu     sync.Mutex
	scriptCached *fakeScript
	scriptStamp  string
)

// loadScript reads and parses the script file named by
// FAKEMCP_SCRIPT_FILE, caching by (path, mtime, size) so the
// per-request re-read stays cheap. It returns nil when scripting is
// off, or the file cannot be read or parsed.
func loadScript() *fakeScript {
	path := os.Getenv("FAKEMCP_SCRIPT_FILE")
	if path == "" {
		return nil
	}
	info, err := os.Stat(path)
	if err != nil {
		return nil
	}
	stamp := fmt.Sprintf("%s|%s|%d", path, info.ModTime(), info.Size())
	scriptMu.Lock()
	defer scriptMu.Unlock()
	if scriptCached != nil && scriptStamp == stamp {
		return scriptCached
	}
	b, err := os.ReadFile(path)
	if err != nil {
		return nil
	}
	var s fakeScript
	if err := json.Unmarshal(b, &s); err != nil {
		return nil
	}
	scriptCached = &s
	scriptStamp = stamp
	return scriptCached
}

// scriptedToolDefs returns the tool definitions the script declares,
// in tools/list shape.
func scriptedToolDefs(s *fakeScript) []map[string]any {
	var defs []map[string]any
	for _, t := range s.Tools {
		def := map[string]any{"name": t.Name, "description": t.Description}
		if len(t.InputSchema) > 0 {
			def["inputSchema"] = t.InputSchema
		} else {
			def["inputSchema"] = map[string]any{"type": "object", "properties": map[string]any{}}
		}
		defs = append(defs, def)
	}
	return defs
}

// scriptCall answers a tools/call for a scripted tool.
func scriptCall(id *int64, t scriptTool, args map[string]any) map[string]any {
	resp := t.Response
	for resp.IfEnv != nil {
		if os.Getenv(resp.IfEnv.Name) == resp.IfEnv.Equals {
			resp.IfEnv = nil
			break
		}
		if resp.Else == nil {
			return ok(id, textResult(""))
		}
		resp = *resp.Else
	}
	if resp.DelayMs > 0 {
		time.Sleep(time.Duration(resp.DelayMs) * time.Millisecond)
	}
	if resp.Error != nil {
		code := resp.Error.Code
		if code == 0 {
			code = -32603
		}
		return fail(id, code, resp.Error.Message)
	}
	if resp.EchoArgs {
		b, _ := json.Marshal(args)
		return ok(id, textResult(string(b)))
	}
	if resp.IsError {
		return ok(id, map[string]any{
			"isError": true,
			"content": []map[string]any{{"type": "text", "text": resp.Text}},
		})
	}
	return ok(id, textResult(resp.Text))
}
