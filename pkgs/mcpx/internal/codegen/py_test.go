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
	for _, want := range []string{"from mcpx_client import tools, call, ToolError", "class Demo:", "def fancy_name(self, *, x: str) -> Any: ..."} {
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

// TestPythonCodexRound2 covers the four Codex round-2 findings on the
// Python emitter: runtime bindings reserved in class allocation,
// parameters that would shadow runtime helpers, the call(server, tool)
// overload with omitted args, and literal keys in result TypedDicts.
func TestPythonCodexRound2(t *testing.T) {
	l := pythonLang(t)
	ns := codegen.Namespace{
		Name:   "tool_error",
		Server: "err-server",
		Tools: []codegen.Tool{{
			Name:         "probe",
			InputSchema:  json.RawMessage(`{"type":"object","properties":{"_MISSING":{"type":"string"},"_unwrap":{"type":"string"}}}`),
			OutputSchema: json.RawMessage(`{"type":"object","properties":{"class":{"type":"string"},"next-page":{"type":"string"},"plain":{"type":"integer"}}}`),
		}},
	}
	out := l.EmitModule([]codegen.Namespace{ns}, "http://127.0.0.1:1234", "sess-2")
	for _, want := range []string{
		// The namespace class must not replace the runtime ToolError.
		"class ToolError2:",
		// Parameters are renamed around the runtime helpers; the
		// request keys keep the original property names.
		"_MISSING_: Optional[str] = _MISSING",
		"if _MISSING_ is not _MISSING:",
		`payload["_MISSING"] = _MISSING_`,
		"_unwrap_: Optional[str] = _MISSING",
		// call("files", "read") keeps the tool name.
		"isinstance(tool_or_args, str)",
		// Result TypedDict keeps the server's literal keys.
		`TypedDict("ToolError2ProbeResult", {"class": str, "next-page": str, "plain": int}, total=False)`,
	} {
		if !strings.Contains(out, want) {
			t.Errorf("python module missing %q:\n%s", want, out)
		}
	}
	if strings.Contains(out, "class ToolError:\n") && !strings.Contains(out, "class ToolError(RuntimeError)") {
		t.Errorf("namespace class replaced runtime ToolError:\n%s", out)
	}
}

func TestPythonCodexRound3(t *testing.T) {
	l := pythonLang(t)
	ns := codegen.Namespace{
		Name:   "demo",
		Server: "demo-server",
		Tools: []codegen.Tool{
			{
				Name:        "__init__",
				InputSchema: json.RawMessage(`{"type":"object","properties":{"x":{"type":"string"}},"required":["x"]}`),
			},
			{
				Name: "shapes",
				InputSchema: json.RawMessage(`{"type":"object","properties":{
					"obj_enum":{"enum":[{"x":true}]},
					"arr_enum":{"enum":[[1,2]]},
					"mixed_enum":{"enum":[{"x":true},[1]]},
					"obj_const":{"const":{"a":1}},
					"scalar_enum":{"enum":["a","b"]}
				}}`),
			},
			{
				Name:        "greet",
				InputSchema: json.RawMessage(`{"type":"object","properties":{"name":{"type":"string"},"count":{"type":"integer"}},"required":["name"]}`),
			},
		},
	}
	out := l.EmitModule([]codegen.Namespace{ns}, "http://127.0.0.1:1234", "sess-3")
	for _, want := range []string{
		// A tool named __init__ is renamed as a binding; the request
		// keeps the original tool name.
		"def __init___(self, *, x: str)",
		`_call_raw("demo-server", "__init__"`,
		// Compound enum/const values fall back to container types.
		"obj_enum: Dict[str, Any]",
		"arr_enum: List[Any]",
		"mixed_enum: Any",
		"obj_const: Dict[str, Any]",
		// Scalar enums still use Literal.
		`scalar_enum: Literal["a", "b"]`,
	} {
		if !strings.Contains(out, want) {
			t.Errorf("python module missing %q:\n%s", want, out)
		}
	}
	for _, bad := range []string{
		"def __init__(self, *, x: str)",
		`Literal[{"x":true}]`,
		"Literal[[1, 2]]",
		"Literal[[1,2]]",
	} {
		if strings.Contains(out, bad) {
			t.Errorf("python module contains %q:\n%s", bad, out)
		}
	}
	decl := l.EmitDeclarations([]codegen.Namespace{ns})
	// Declarations advertise the module's keyword-only convention:
	// required arguments carry no default, optional ones do.
	if want := "def greet(self, *, name: str, count: Optional[int] = ...) -> Any: ..."; !strings.Contains(decl, want) {
		t.Errorf("python declarations missing %q:\n%s", want, decl)
	}
}

// TestPythonCodexRound4 covers the three Codex round-4 findings on the
// current head: a property named "self" colliding with the method's own
// first parameter, "$" surviving into Python identifiers, and TCP
// header values being encoded as Latin-1 instead of UTF-8.
func TestPythonCodexRound4(t *testing.T) {
	l := pythonLang(t)
	ns := codegen.Namespace{
		Name:   "demo",
		Server: "demo-server",
		Tools: []codegen.Tool{{
			Name: "ping",
			InputSchema: json.RawMessage(`{"type":"object","properties":{
				"self":{"type":"string"},
				"$id":{"type":"string"}
			}}`),
		}},
	}
	out := l.EmitModule([]codegen.Namespace{ns}, "http://127.0.0.1:1234", "sess-4")
	for _, want := range []string{
		// "self" is allocated around the method's own parameter.
		"self_: Optional[str] = _MISSING",
		// "$" is renamed out of Python identifiers.
		"_id: ",
	} {
		if !strings.Contains(out, want) {
			t.Errorf("python module missing %q:\n%s", want, out)
		}
	}
	if strings.Contains(out, "$id:") || strings.Contains(out, "*, self:") {
		t.Errorf("python module keeps an invalid identifier:\n%s", out)
	}

	// Drive the client over TCP with a non-Latin-1 MCPX_CWD: the call
	// must go out (no UnicodeEncodeError) and the daemon must see the
	// cwd header as the UTF-8 path, as on the socket transport.
	python, err := exec.LookPath("python3")
	if err != nil {
		t.Skip("python3 not available")
	}
	cwd := make(chan string, 1)
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		cwd <- r.Header.Get("X-Mcpx-Cwd")
		w.Header().Set("content-type", "application/json")
		_, _ = w.Write([]byte(`{"result":{"structuredContent":{"ok":true}}}`))
	}))
	defer srv.Close()
	dir := t.TempDir()
	module := l.EmitModule([]codegen.Namespace{ns}, srv.URL, "sess-4")
	if err := os.WriteFile(filepath.Join(dir, "mcpx_client.py"), []byte(module), 0o644); err != nil {
		t.Fatal(err)
	}
	driver := "import sys\n" +
		"sys.path.insert(0, " + pyStrLit(dir) + ")\n" +
		"from mcpx_client import tools\n" +
		"print(tools.Demo.ping(self_=\"a\", _id=\"b\"))\n"
	if err := os.WriteFile(filepath.Join(dir, "driver.py"), []byte(driver), 0o644); err != nil {
		t.Fatal(err)
	}
	cmd := exec.Command(python, filepath.Join(dir, "driver.py"))
	cmd.Env = append(os.Environ(),
		"MCPX_ENDPOINT="+srv.URL, "MCPX_SESSION=sess-4", "MCPX_SOCKET=",
		"MCPX_CWD=/tmp/项目")
	got, err := cmd.CombinedOutput()
	if err != nil {
		t.Fatalf("generated python client failed over TCP: %v\n%s\n--- module ---\n%s", err, got, module)
	}
	if gotCwd, want := <-cwd, "/tmp/项目"; gotCwd != want {
		t.Fatalf("fixture daemon got x-mcpx-cwd = %q, want %q", gotCwd, want)
	}
}

// TestPythonCodexRound5 covers the three Codex round-5 findings on
// deffb123: scalar const annotations emitted bare (a string const
// becomes a TypedDict forward reference, a SyntaxError at import),
// Python keyword literals (None/True/False) not reserved when
// allocating namespace class names, and arguments defined only inside
// composed (allOf/anyOf) input schemas being dropped by schemaFields.
func TestPythonCodexRound5(t *testing.T) {
	l := pythonLang(t)
	none := codegen.Namespace{
		Name:   "none",
		Server: "none-server",
		Tools: []codegen.Tool{{
			Name:        "ping",
			InputSchema: json.RawMessage(`{"type":"object","properties":{"mode":{"const":"hello world"},"empty":{"const":""}}}`),
		}},
	}
	demo := codegen.Namespace{
		Name:   "demo",
		Server: "demo-server",
		Tools: []codegen.Tool{
			{
				Name:        "composed",
				InputSchema: json.RawMessage(`{"type":"object","allOf":[{"properties":{"x":{"type":"string"}},"required":["x"]},{"properties":{"y":{"type":"integer"}}}]}`),
			},
			{
				Name:        "branched",
				InputSchema: json.RawMessage(`{"type":"object","anyOf":[{"properties":{"a":{"type":"string"}},"required":["a"]},{"properties":{"a":{"type":"string"},"b":{"type":"string"}},"required":["a","b"]}]}`),
			},
		},
	}
	out := l.EmitModule([]codegen.Namespace{none, demo}, "http://127.0.0.1:1234", "sess-5")
	for _, want := range []string{
		// Scalar consts render as Literal[...], like enum values.
		`mode: Literal["hello world"]`,
		`empty: Literal[""]`,
		// A namespace named "none" must not emit `class None:`.
		"class None2:",
		// allOf arguments appear with their real types; the first
		// branch's required field stays required.
		"def composed(self, *, x: str, y: Optional[int] = _MISSING)",
		// anyOf: "a" is required by every branch, "b" only by one.
		"def branched(self, *, a: str, b: Optional[str] = _MISSING)",
	} {
		if !strings.Contains(out, want) {
			t.Errorf("python module missing %q:\n%s", want, out)
		}
	}
	for _, bad := range []string{
		"class None:",
		`mode: "hello world"`,
	} {
		if strings.Contains(out, bad) {
			t.Errorf("python module contains %q:\n%s", bad, out)
		}
	}

	// Drive the composed-schema tool against a fixture daemon: the
	// generated method must accept the allOf arguments and send them.
	python, err := exec.LookPath("python3")
	if err != nil {
		t.Skip("python3 not available")
	}
	type callBody struct {
		Args json.RawMessage `json:"args"`
	}
	seen := make(chan callBody, 1)
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		var body callBody
		if err := json.NewDecoder(r.Body).Decode(&body); err != nil {
			t.Errorf("fixture daemon: decode call: %v", err)
		}
		seen <- body
		w.Header().Set("content-type", "application/json")
		_, _ = w.Write([]byte(`{"result":{"structuredContent":{"ok":true}}}`))
	}))
	defer srv.Close()
	dir := t.TempDir()
	module := l.EmitModule([]codegen.Namespace{demo}, srv.URL, "sess-5")
	if err := os.WriteFile(filepath.Join(dir, "mcpx_client.py"), []byte(module), 0o644); err != nil {
		t.Fatal(err)
	}
	driver := "import sys\n" +
		"sys.path.insert(0, " + pyStrLit(dir) + ")\n" +
		"from mcpx_client import tools\n" +
		"print(tools.Demo.composed(x=\"hi\", y=3))\n"
	if err := os.WriteFile(filepath.Join(dir, "driver.py"), []byte(driver), 0o644); err != nil {
		t.Fatal(err)
	}
	cmd := exec.Command(python, filepath.Join(dir, "driver.py"))
	cmd.Env = append(os.Environ(), "MCPX_ENDPOINT="+srv.URL, "MCPX_SESSION=sess-5", "MCPX_SOCKET=")
	got, err := cmd.CombinedOutput()
	if err != nil {
		t.Fatalf("generated python client failed for composed schema: %v\n%s\n--- module ---\n%s", err, got, module)
	}
	body := <-seen
	var args map[string]any
	if err := json.Unmarshal(body.Args, &args); err != nil {
		t.Fatalf("fixture daemon: unmarshal args: %v", err)
	}
	if args["x"] != "hi" || args["y"] != float64(3) {
		t.Fatalf("fixture daemon got args = %s, want x=hi, y=3", body.Args)
	}

	// The "none"-namespace module must import cleanly (no `class None`,
	// no bare-string const annotation).
	dir2 := t.TempDir()
	if err := os.WriteFile(filepath.Join(dir2, "mcpx_client.py"), []byte(out), 0o644); err != nil {
		t.Fatal(err)
	}
	imp := exec.Command(python, "-c", "import sys; sys.path.insert(0, "+pyStrLit(dir2)+"); import mcpx_client")
	if got, err := imp.CombinedOutput(); err != nil {
		t.Fatalf("generated python module with none namespace / const fields does not import: %v\n%s\n--- module ---\n%s", err, got, out)
	}
}

// TestPythonCodexRound6 covers the two Codex round-6 findings (14:20Z re-review
// of head 4c988840) on the Python emitter: leading-double-underscore names
// must not mangle inside the generated classes, and open input schemas (no
// named properties, additional arguments allowed) need an extra-mapping
// fallback instead of a `def put(self)` that can only ever send {}.
func TestPythonCodexRound6(t *testing.T) {
	l := pythonLang(t)
	demo := codegen.Namespace{
		Name:   "demo",
		Server: "demo-server",
		Tools: []codegen.Tool{
			{
				Name:        "__hidden",
				InputSchema: json.RawMessage(`{"type":"object","properties":{"__value":{"type":"string"}},"required":["__value"]}`),
			},
			{
				Name:        "put",
				InputSchema: json.RawMessage(`{"type":"object","additionalProperties":{"type":"string"},"minProperties":1}`),
			},
			{
				Name:        "sealed",
				InputSchema: json.RawMessage(`{"type":"object","additionalProperties":false}`),
			},
		},
	}
	out := l.EmitModule([]codegen.Namespace{demo}, "http://127.0.0.1:1234", "sess-6")
	for _, want := range []string{
		// The dunder tool binds to a non-private method; the request
		// keeps the tool's original name.
		"def hidden(self, *, value: str)",
		`_call_raw("demo-server", "__hidden", payload)`,
		// The dunder property binds to a non-private parameter; the
		// request keeps the original JSON key.
		`payload["__value"] = value`,
		// Open schemas take an explicit extra mapping merged into the
		// request; the declaration advertises the same convention.
		"def put(self, *, extra: Optional[Dict[str, Any]] = None)",
		"payload.update(extra)",
		// A sealed schema still sends exactly {}.
		"def sealed(self)",
	} {
		if !strings.Contains(out, want) {
			t.Errorf("python module missing %q:\n%s", want, out)
		}
	}
	decl := l.EmitDeclarations([]codegen.Namespace{demo})
	for _, want := range []string{
		"def hidden(self, *, value: str) -> Any: ...",
		"def put(self, *, extra: Optional[Dict[str, Any]] = ...) -> Any: ...",
		"def sealed(self) -> Any: ...",
	} {
		if !strings.Contains(decl, want) {
			t.Errorf("python declarations missing %q:\n%s", want, decl)
		}
	}
	for _, bad := range []string{
		"def __hidden(",
		"def __value(",
		"__value: str",
		"def put(self)",
	} {
		if strings.Contains(out, bad) {
			t.Errorf("python module contains %q:\n%s", bad, out)
		}
	}

	python, err := exec.LookPath("python3")
	if err != nil {
		t.Skip("python3 not available")
	}
	type callBody struct {
		Server string          `json:"server"`
		Tool   string          `json:"tool"`
		Args   json.RawMessage `json:"args"`
	}
	seen := make(chan callBody, 8)
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		var body callBody
		if err := json.NewDecoder(r.Body).Decode(&body); err != nil {
			t.Errorf("fixture daemon: decode call: %v", err)
		}
		seen <- body
		w.Header().Set("content-type", "application/json")
		_, _ = w.Write([]byte(`{"result":{"structuredContent":{"ok":true}}}`))
	}))
	defer srv.Close()
	dir := t.TempDir()
	module := l.EmitModule([]codegen.Namespace{demo}, srv.URL, "sess-6")
	if err := os.WriteFile(filepath.Join(dir, "mcpx_client.py"), []byte(module), 0o644); err != nil {
		t.Fatal(err)
	}
	driver := "import sys\n" +
		"sys.path.insert(0, " + pyStrLit(dir) + ")\n" +
		"from mcpx_client import tools\n" +
		"tools.Demo.hidden(value=\"x\")\n" +
		"tools.Demo.put(extra={\"a\": \"1\"})\n" +
		"tools.Demo.put()\n" +
		"tools.Demo.sealed()\n" +
		"print('round6-ok')\n"
	if err := os.WriteFile(filepath.Join(dir, "driver.py"), []byte(driver), 0o644); err != nil {
		t.Fatal(err)
	}
	cmd := exec.Command(python, filepath.Join(dir, "driver.py"))
	cmd.Env = append(os.Environ(), "MCPX_ENDPOINT="+srv.URL, "MCPX_SESSION=sess-6", "MCPX_SOCKET=")
	got, err := cmd.CombinedOutput()
	if err != nil {
		t.Fatalf("generated python client failed round-6 cases: %v\n%s\n--- module ---\n%s", err, got, module)
	}
	if !strings.Contains(string(got), "round6-ok") {
		t.Fatalf("driver did not finish:\n%s\n--- module ---\n%s", got, module)
	}
	wantCalls := []struct{ server, tool, args string }{
		{"demo-server", "__hidden", `{"__value": "x"}`},
		{"demo-server", "put", `{"a": "1"}`},
		{"demo-server", "put", `{}`},
		{"demo-server", "sealed", `{}`},
	}
	for i, want := range wantCalls {
		body := <-seen
		if body.Server != want.server || body.Tool != want.tool {
			t.Errorf("call %d = %s/%s, want %s/%s", i, body.Server, body.Tool, want.server, want.tool)
		}
		if args := strings.TrimSpace(string(body.Args)); args != want.args {
			t.Errorf("call %d args = %s, want %s", i, args, want.args)
		}
	}
}
