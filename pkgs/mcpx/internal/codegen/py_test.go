package codegen_test

import (
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"testing"

	"github.com/dezren39/mcpx/internal/codegen"
)

func pythonLang(t *testing.T) codegen.Language {
	t.Helper()
	l, ok := codegen.Lookup("python")
	if !ok {
		t.Fatal("python must be registered in codegen.Languages (#1917 Phase 1)")
	}
	return l
}

func TestPythonLanguageFiles(t *testing.T) {
	l := pythonLang(t)
	if l.ClientFile != "mcpx_client.py" || l.GlobalsFile != "mcpx_globals.pyi" || l.ScriptExt != ".py" {
		t.Fatalf("python entry = %+v, want mcpx_client.py / mcpx_globals.pyi / .py", l)
	}
	if codegen.Default().Name != "typescript" {
		t.Fatal("registering python must not change the default target")
	}
}

func TestPythonModuleShape(t *testing.T) {
	l := pythonLang(t)
	out := l.EmitModule([]codegen.Namespace{sampleNamespace()}, "http://127.0.0.1:1234", "sess-1")
	for _, want := range []string{
		`DEFAULT_ENDPOINT = "http://127.0.0.1:1234"`,
		`DEFAULT_SESSION = "sess-1"`,
		"import http.client",
		"class DemoFancyNameArgs",
		"class Demo:",
		"def fancy_name(self, *, x: str)",
		`_call_raw("demo-server", "fancy-name", payload)`,
		"MCPX_ENDPOINT",
		"MCPX_SOCKET",
		"MCPX_SESSION",
		"tools = _Tools()",
	} {
		if !strings.Contains(out, want) {
			t.Errorf("python module missing %q:\n%s", want, out)
		}
	}
	// Stdlib only: no third-party imports anywhere in the module.
	for _, bad := range []string{"import requests", "import httpx", "import aiohttp"} {
		if strings.Contains(out, bad) {
			t.Errorf("python module must be stdlib-only, found %q", bad)
		}
	}
}

func TestPythonGlobalsAndDeclarations(t *testing.T) {
	l := pythonLang(t)
	globals := l.EmitGlobals([]codegen.Namespace{sampleNamespace()})
	for _, want := range []string{"from mcpx_client import", "ToolError", "LAST_RESULT"} {
		if !strings.Contains(globals, want) {
			t.Errorf("python globals stub missing %q:\n%s", want, globals)
		}
	}
	decl := l.EmitDeclarations([]codegen.Namespace{sampleNamespace()})
	for _, want := range []string{"from mcpx_client import tools, call, ToolError", "class Demo:", "def fancy_name(self, x: str = ...) -> Any: ..."} {
		if !strings.Contains(decl, want) {
			t.Errorf("python declarations missing %q:\n%s", want, decl)
		}
	}
}

// TestPythonClientRoundTrip drives the generated client against a fixture
// daemon: a typed method call must POST /v1/call with the server name (not
// the namespace), the tool name and the typed arguments, and return the
// daemon's structured result unwrapped.
func TestPythonClientRoundTrip(t *testing.T) {
	python, err := exec.LookPath("python3")
	if err != nil {
		t.Skip("python3 not available")
	}
	type callBody struct {
		Server string          `json:"server"`
		Tool   string          `json:"tool"`
		Args   json.RawMessage `json:"args"`
	}
	seen := make(chan callBody, 1)
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.URL.Path != "/v1/call" {
			http.NotFound(w, r)
			return
		}
		var body callBody
		if err := json.NewDecoder(r.Body).Decode(&body); err != nil {
			t.Errorf("fixture daemon: decode call: %v", err)
		}
		seen <- body
		w.Header().Set("content-type", "application/json")
		_, _ = w.Write([]byte(`{"result":{"structuredContent":{"greeting":"hello, world"}}}`))
	}))
	defer srv.Close()

	l := pythonLang(t)
	dir := t.TempDir()
	module := l.EmitModule([]codegen.Namespace{sampleNamespace()}, srv.URL, "sess-rt")
	if err := os.WriteFile(filepath.Join(dir, "mcpx_client.py"), []byte(module), 0o644); err != nil {
		t.Fatal(err)
	}
	driver := "import json, sys\n" +
		"sys.path.insert(0, " + pyStrLit(dir) + ")\n" +
		"from mcpx_client import tools\n" +
		"print(json.dumps(tools.Demo.fancy_name(x=\"world\")))\n"
	if err := os.WriteFile(filepath.Join(dir, "driver.py"), []byte(driver), 0o644); err != nil {
		t.Fatal(err)
	}
	cmd := exec.Command(python, filepath.Join(dir, "driver.py"))
	cmd.Env = append(os.Environ(),
		"MCPX_ENDPOINT="+srv.URL, "MCPX_SESSION=sess-rt", "MCPX_SOCKET=")
	out, err := cmd.CombinedOutput()
	if err != nil {
		t.Fatalf("generated python client failed: %v\n%s\n--- module ---\n%s", err, out, module)
	}
	if got := strings.TrimSpace(string(out)); got != `{"greeting": "hello, world"}` {
		t.Fatalf("round-trip result = %s, want the unwrapped structuredContent", got)
	}
	body := <-seen
	if body.Server != "demo-server" || body.Tool != "fancy-name" {
		t.Fatalf("fixture daemon got server/tool = %q/%q, want demo-server/fancy-name", body.Server, body.Tool)
	}
	if got := strings.TrimSpace(string(body.Args)); got != `{"x": "world"}` {
		t.Fatalf("fixture daemon got args = %s, want {\"x\": \"world\"}", got)
	}
}

func pyStrLit(s string) string {
	b, _ := json.Marshal(s)
	return string(b)
}

// TestPythonClientEdgeCases drives a generated client built from hostile
// schemas through a fixture daemon. Every case here is a regression from
// the Codex review round on the Phase 1 PR: each one previously made the
// generated module unimportable or silently wrong.
func TestPythonClientEdgeCases(t *testing.T) {
	python, err := exec.LookPath("python3")
	if err != nil {
		t.Skip("python3 not available")
	}
	l := pythonLang(t)

	nss := []codegen.Namespace{
		{
			Name:        "demo",
			Server:      "demo-server",
			Description: "Use Python \"\"\"triple quotes\"\"\" for strings.",
			Tools: []codegen.Tool{
				{Name: "fancy-name", InputSchema: json.RawMessage(`{"type":"object","properties":{"x":{"type":"string"}},"required":["x"]}`)},
				{Name: "ping", InputSchema: json.RawMessage(`{"type":"object","properties":{"n":{"allOf":[{"type":"integer"},{"type":"number"}]},"q":{"type":["string","null"]}}}`)},
				{Name: "send", InputSchema: json.RawMessage(`{"type":"object","properties":{"payload":{"type":"string"}},"required":["payload"]}`)},
				{Name: "is", InputSchema: json.RawMessage(`{"type":"object","properties":{}}`)},
				{Name: "is_", InputSchema: json.RawMessage(`{"type":"object","properties":{}}`)},
			},
		},
		{Name: "fooBar", Server: "server-b", Tools: []codegen.Tool{
			{Name: "ping", InputSchema: json.RawMessage(`{"type":"object","properties":{}}`)},
		}},
		{Name: "foo_bar", Server: "server-a", Tools: []codegen.Tool{
			{Name: "ping", InputSchema: json.RawMessage(`{"type":"object","properties":{}}`)},
		}},
	}
	module := l.EmitModule(nss, "http://127.0.0.1:1234", "sess-1")
	for _, bad := range []string{`"/v1/read"`, "int & float", "def ping(self, *q"} {
		if strings.Contains(module, bad) {
			t.Errorf("python module must not contain %q:\n%s", bad, module)
		}
	}
	for _, want := range []string{`"/v1/resource"`, "def ping(self, *, ", " = _MISSING", "class FooBar2:"} {
		if !strings.Contains(module, want) {
			t.Errorf("python module missing %q:\n%s", want, module)
		}
	}

	type recorded struct {
		Path string
		Body map[string]any
	}
	recCh := make(chan recorded, 32)
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		var body map[string]any
		if err := json.NewDecoder(r.Body).Decode(&body); err != nil {
			t.Errorf("fixture daemon: decode: %v", err)
		}
		recCh <- recorded{r.URL.Path, body}
		w.Header().Set("content-type", "application/json")
		if r.URL.Path == "/v1/resource" {
			_, _ = w.Write([]byte(`{"result":{"uri":"mcpx://thing","text":"data"}}`))
			return
		}
		_, _ = w.Write([]byte(`{"result":{"structuredContent":{"greeting":"hello, world"}}}`))
	}))
	defer srv.Close()

	dir := t.TempDir()
	if err := os.WriteFile(filepath.Join(dir, "mcpx_client.py"), []byte(l.EmitModule(nss, srv.URL, "sess-edge")), 0o644); err != nil {
		t.Fatal(err)
	}
	driver := "import json, sys\n" +
		"sys.path.insert(0, " + pyStrLit(dir) + ")\n" +
		"from mcpx_client import tools, read_resource\n" +
		"out = {}\n" +
		"out['bare'] = tools.fancy_name(x=\"world\")\n" +
		"tools.Demo.ping()\n" +
		"tools.Demo.ping(q=None)\n" +
		"tools.Demo.send(payload=\"hi\")\n" +
		"try:\n" +
		"    tools.Demo.is_()\n" +
		"    out['collision'] = 'no error'\n" +
		"except Exception as exc:\n" +
		"    out['collision'] = str(exc)\n" +
		"tools.FooBar.ping()\n" +
		"tools.FooBar2.ping()\n" +
		"out['resource'] = read_resource(\"demo-server\", \"mcpx://thing\")\n" +
		"print(json.dumps(out))\n"
	if err := os.WriteFile(filepath.Join(dir, "driver.py"), []byte(driver), 0o644); err != nil {
		t.Fatal(err)
	}
	cmd := exec.Command(python, filepath.Join(dir, "driver.py"))
	cmd.Env = append(os.Environ(), "MCPX_ENDPOINT="+srv.URL, "MCPX_SESSION=sess-edge", "MCPX_SOCKET=")
	out, err := cmd.CombinedOutput()
	if err != nil {
		t.Fatalf("generated python client failed on edge cases: %v\n%s\n--- module ---\n%s", err, out, module)
	}
	var result map[string]any
	if err := json.Unmarshal([]byte(strings.TrimSpace(string(out))), &result); err != nil {
		t.Fatalf("driver output is not json: %v\n%s", err, out)
	}
	if got := result["bare"]; got == nil {
		t.Errorf("bare-name call result missing: %v", result)
	}
	if c, _ := result["collision"].(string); !strings.Contains(c, `"is"`) || !strings.Contains(c, "use call") {
		t.Errorf("collision error = %q, want it to name both tools and the call() escape hatch", c)
	}
	if r, _ := result["resource"].(map[string]any); r["text"] != "data" {
		t.Errorf("read_resource result = %v, want the daemon's resource result", result["resource"])
	}

	wantCalls := []struct {
		path, server, tool, args string
	}{
		{"/v1/call", "demo-server", "fancy-name", `{"x":"world"}`},
		{"/v1/call", "demo-server", "ping", `{}`},
		{"/v1/call", "demo-server", "ping", `{"q":null}`},
		{"/v1/call", "demo-server", "send", `{"payload":"hi"}`},
		{"/v1/call", "server-b", "ping", `{}`},
		{"/v1/call", "server-a", "ping", `{}`},
		{"/v1/resource", "demo-server", "", ``},
	}
	for i, want := range wantCalls {
		rec := <-recCh
		if rec.Path != want.path {
			t.Fatalf("call %d path = %s, want %s (body %v)", i, rec.Path, want.path, rec.Body)
		}
		if want.tool != "" && rec.Body["tool"] != want.tool {
			t.Errorf("call %d tool = %v, want %s", i, rec.Body["tool"], want.tool)
		}
		if rec.Body["server"] != want.server {
			t.Errorf("call %d server = %v, want %s", i, rec.Body["server"], want.server)
		}
		if want.args != "" {
			args, _ := json.Marshal(rec.Body["args"])
			if string(args) != want.args {
				t.Errorf("call %d args = %s, want %s", i, args, want.args)
			}
		}
		if want.path == "/v1/resource" && rec.Body["uri"] != "mcpx://thing" {
			t.Errorf("resource call uri = %v, want mcpx://thing", rec.Body["uri"])
		}
	}

	// An empty namespace selection must still import: _Tools.__init__
	// needs a body even when there is nothing to initialise.
	dir2 := t.TempDir()
	if err := os.WriteFile(filepath.Join(dir2, "mcpx_client.py"), []byte(l.EmitModule(nil, srv.URL, "sess-empty")), 0o644); err != nil {
		t.Fatal(err)
	}
	driver2 := "import sys\n" +
		"sys.path.insert(0, " + pyStrLit(dir2) + ")\n" +
		"import mcpx_client\n" +
		"print('empty-ok', mcpx_client.tools is not None)\n"
	if err := os.WriteFile(filepath.Join(dir2, "driver.py"), []byte(driver2), 0o644); err != nil {
		t.Fatal(err)
	}
	cmd2 := exec.Command(python, filepath.Join(dir2, "driver.py"))
	cmd2.Env = append(os.Environ(), "MCPX_ENDPOINT="+srv.URL, "MCPX_SESSION=sess-empty", "MCPX_SOCKET=")
	out2, err := cmd2.CombinedOutput()
	if err != nil || !strings.Contains(string(out2), "empty-ok True") {
		t.Fatalf("empty-selection module must import: %v\n%s", err, out2)
	}
}
