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
