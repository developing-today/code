package runner

// Python as a harnessed language (#1917 Phase 2): a Python script is run by
// the machine's Python with the generated Python client installed beside it,
// the counterpart of what a TypeScript run has always had. These tests prove
// the client is installed, importable, and able to call a tool end to end,
// and that a TypeScript run is untouched.

import (
	"bytes"
	"context"
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"testing"

	"github.com/developing-today/code/pkgs/mcpx/internal/codegen"
)

// pythonFixture renders the Python client and stub for one demo namespace and
// returns a daemon fixture that answers the one call the scripts make.
func pythonFixture(t *testing.T, seen chan<- map[string]any) (module, globals string) {
	t.Helper()
	nss := []codegen.Namespace{{
		Name:   "demo",
		Server: "demo-server",
		Tools: []codegen.Tool{{
			Name:        "fancy-name",
			Description: "does a thing",
			InputSchema: json.RawMessage(`{"type":"object","properties":{"x":{"type":"string"}},"required":["x"]}`),
		}},
	}}
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.URL.Path != "/v1/call" {
			http.NotFound(w, r)
			return
		}
		var body map[string]any
		if err := json.NewDecoder(r.Body).Decode(&body); err != nil {
			t.Errorf("fixture daemon: decode call: %v", err)
		}
		if seen != nil {
			seen <- body
		}
		w.Header().Set("content-type", "application/json")
		_, _ = w.Write([]byte(`{"result":{"structuredContent":{"greeting":"hello, world"}}}`))
	}))
	t.Cleanup(srv.Close)
	lang, ok := codegen.Lookup(codegen.Python.Name)
	if !ok {
		t.Fatal("codegen must register python (#1917 Phase 1)")
	}
	return lang.EmitModule(nss, srv.URL, ""), lang.EmitGlobals(nss)
}

func pythonClients(module, globals string) map[string]Client {
	return map[string]Client{codegen.Python.Name: {Module: module, Globals: globals}}
}

func requirePython(t *testing.T) {
	t.Helper()
	if _, err := exec.LookPath("python3"); err != nil {
		if _, uerr := exec.LookPath("python"); uerr != nil {
			if _, verr := exec.LookPath("uv"); verr != nil {
				t.Skip("no python3, python or uv on PATH")
			}
		}
	}
}

// TestPythonFileRunInstallsThePythonClient is the acceptance case: a .py file
// through the runner imports the generated client and calls a tool, and the
// client and stub are on disk next to the script rather than a TypeScript
// module the script could never import.
func TestPythonFileRunInstallsThePythonClient(t *testing.T) {
	requirePython(t)
	seen := make(chan map[string]any, 1)
	module, globals := pythonFixture(t, seen)

	dir := t.TempDir()
	script := filepath.Join(dir, "report.py")
	body := "import json\n" +
		"from mcpx_client import tools\n" +
		"print(json.dumps(tools.Demo.fancy_name(x=\"world\")))\n"
	if err := os.WriteFile(script, []byte(body), 0o644); err != nil {
		t.Fatal(err)
	}

	var stdout, stderr bytes.Buffer
	res, err := Run(context.Background(), Options{
		File:    script,
		Clients: pythonClients(module, globals),
		Env:     map[string]string{"MCPX_SESSION": "sess-py"},
		Stdout:  &stdout,
		Stderr:  &stderr,
	})
	if err != nil {
		t.Fatalf("python run failed: %v\nstderr:\n%s", err, stderr.String())
	}
	if got := strings.TrimSpace(stdout.String()); got != `{"greeting": "hello, world"}` {
		t.Fatalf("stdout = %q, want the unwrapped structuredContent\nstderr:\n%s", got, stderr.String())
	}
	if res.Notice != "" {
		t.Errorf("a harnessed Python run has a client, so it must not print the no-client notice: %q", res.Notice)
	}
	if !strings.Contains(res.Runtime, "python") {
		t.Errorf("Result.Runtime = %q, want the Python interpreter", res.Runtime)
	}

	client := filepath.Join(dir, "mcpx_client.py")
	if res.Client != client {
		t.Errorf("Result.Client = %q, want %q", res.Client, client)
	}
	for _, name := range []string{"mcpx_client.py", "mcpx_globals.pyi"} {
		if _, err := os.Stat(filepath.Join(dir, name)); err != nil {
			t.Errorf("%s must be installed beside the script: %v", name, err)
		}
	}
	if _, err := os.Stat(filepath.Join(dir, clientFileName)); err == nil {
		t.Errorf("%s must not be installed for a Python run", clientFileName)
	}

	call := <-seen
	if call["server"] != "demo-server" || call["tool"] != "fancy-name" {
		t.Errorf("daemon got server/tool = %v/%v, want demo-server/fancy-name", call["server"], call["tool"])
	}
}

// TestPythonShebangSnippetGetsThePythonClient covers the other way a run
// names Python: a snippet whose shebang is python3. Dispatch rank 1 still
// chooses the interpreter, and the client follows the language rather than
// the rank.
func TestPythonShebangSnippetGetsThePythonClient(t *testing.T) {
	requirePython(t)
	module, globals := pythonFixture(t, nil)

	work := t.TempDir()
	src := "#!/usr/bin/env python3\n" +
		"import json\n" +
		"from mcpx_client import tools\n" +
		"print(json.dumps(tools.Demo.fancy_name(x=\"snip\")))\n"
	var stdout, stderr bytes.Buffer
	res, err := Run(context.Background(), Options{
		Source:  src,
		WorkDir: work,
		Clients: pythonClients(module, globals),
		Stdout:  &stdout,
		Stderr:  &stderr,
	})
	if err != nil {
		t.Fatalf("python snippet failed: %v\nstderr:\n%s", err, stderr.String())
	}
	if !strings.HasSuffix(res.Script, ".py") {
		t.Errorf("a Python snippet's file should carry .py, got %q", res.Script)
	}
	if filepath.Dir(res.Script) != work {
		t.Errorf("snippet script %q should live in the workdir %q", res.Script, work)
	}
	if _, err := os.Stat(filepath.Join(work, "mcpx_client.py")); err != nil {
		t.Errorf("the Python client must be installed in the snippet's directory: %v", err)
	}
	if got := strings.TrimSpace(stdout.String()); got != `{"greeting": "hello, world"}` {
		t.Fatalf("stdout = %q, want the unwrapped result", got)
	}
}

// TestPythonRunWithoutAClientSaysSo keeps the honest fallback: when a caller
// supplies no Python client (an older daemon, a raw runner embedding), the
// run still executes and states once that no client is available, exactly as
// a raw interpreter run does.
func TestPythonRunWithoutAClientSaysSo(t *testing.T) {
	requirePython(t)
	dir := t.TempDir()
	script := filepath.Join(dir, "plain.py")
	if err := os.WriteFile(script, []byte("print('hi')\n"), 0o644); err != nil {
		t.Fatal(err)
	}
	var stdout, stderr bytes.Buffer
	res, err := Run(context.Background(), Options{
		File:   script,
		Stdout: &stdout,
		Stderr: &stderr,
	})
	if err != nil {
		t.Fatalf("run failed: %v", err)
	}
	if got := strings.TrimSpace(stdout.String()); got != "hi" {
		t.Fatalf("stdout = %q, want hi", got)
	}
	if res.Notice == "" || !strings.Contains(stderr.String(), "no generated mcpx client") {
		t.Errorf("a Python run with no client must say so; notice=%q stderr=%q", res.Notice, stderr.String())
	}
	if _, err := os.Stat(filepath.Join(dir, "mcpx_client.py")); err == nil {
		t.Error("no client source was supplied, so none should be written")
	}
}

// TestScriptLanguage is the shared rule callers and the runner both use.
func TestScriptLanguage(t *testing.T) {
	cases := []struct {
		file, source, want string
	}{
		{"report.py", "", codegen.Python.Name},
		{"report.ts", "", codegen.TypeScript.Name},
		{"", "#!/usr/bin/env python3\nprint(1)\n", codegen.Python.Name},
		{"", "#!/usr/bin/env -S python3 -u\nprint(1)\n", codegen.Python.Name},
		{"", "export default () => 1\n", ""},
		{"", "#!/bin/sh\necho hi\n", ""},
		{"", "export default () => 1\n", ""},
	}
	for _, c := range cases {
		if got := ScriptLanguage(c.file, c.source); got != c.want {
			t.Errorf("ScriptLanguage(%q, %q) = %q, want %q", c.file, c.source, got, c.want)
		}
	}
}
