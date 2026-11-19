package codegen_test

import (
	"context"
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"github.com/developing-today/code/pkgs/mcpx/internal/codegen"
)

func TestColdGeneratedClients1868(t *testing.T) {
	for _, language := range []string{"typescript", "python"} {
		t.Run(language, func(t *testing.T) {
			runtime := "deno"
			if language == "python" {
				runtime = "python3"
			}
			binary, err := exec.LookPath(runtime)
			if err != nil {
				t.Skip(runtime + " unavailable")
			}
			l, _ := codegen.Lookup(language)
			srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
				if r.URL.Path != "/v1/call" {
					t.Errorf("unexpected path %s", r.URL.Path)
					http.NotFound(w, r)
					return
				}
				var body map[string]any
				if err := json.NewDecoder(r.Body).Decode(&body); err != nil {
					t.Error(err)
				}
				w.Header().Set("Content-Type", "application/json")
				json.NewEncoder(w).Encode(map[string]any{"result": map[string]any{"structuredContent": body}})
			}))
			defer srv.Close()
			ns := []codegen.Namespace{
				{Name: "cold", Server: "cold-server", Cold: true},
				{Name: "rawselected", Server: "rawselected-server", Cold: true, LazyTools: []string{"foo-bar"}},
				{Name: "partial", Server: "partial-server", Cold: true, Tools: []codegen.Tool{{Name: "known-name", InputSchema: json.RawMessage(`{"type":"object","properties":{"x":{"type":"string"}},"required":["x"]}`)}}},
				{Name: "selected", Server: "selected-server", Cold: true, LazyTools: []string{"only"}},
				{Name: "warm", Server: "warm-server", Tools: []codegen.Tool{{Name: "known", InputSchema: json.RawMessage(`{"type":"object"}`)}}},
				{Name: "empty", Server: "empty-server"},
			}
			dir := t.TempDir()
			if err := os.WriteFile(filepath.Join(dir, l.ClientFile), []byte(l.EmitModule(ns, srv.URL, "isolated")), 0600); err != nil {
				t.Fatal(err)
			}
			var driver string
			if language == "python" {
				driver = `from mcpx_client import tools
r=tools.Cold.new_tool(args="value")
assert r["server"]=="cold-server" and r["tool"]=="new_tool" and r["lazyLanguage"]=="python",r
assert r["args"]=={"args":"value"},r
r=tools.Cold.__init___({"x":1})
assert r["tool"]=="__init___" and r["args"]=={"x":1},r
r=tools.known()
assert r["server"]=="warm-server" and "lazyLanguage" not in r,r
assert tools.Selected.only()["server"]=="selected-server"
r=tools.Rawselected.foo_bar()
assert r["lazySelector"]=="foo-bar",r
r=tools.Partial.known_name(x="typed")
assert r["lazyLanguage"]=="python" and r["args"]=={"x":"typed"},r
for obj in [tools.Selected,tools.Empty]:
 try: obj.missing()
 except AttributeError: pass
 else: raise AssertionError("missing member must not exist")
print("ok")
`
			} else {
				driver = `import {cold,selected,rawselected,partial,empty,tools} from "./mcpx-client.ts";
const r=await cold.new_tool({args:"value"});
if(r.server!=="cold-server"||r.tool!=="new_tool"||r.lazyLanguage!=="typescript"||r.args.args!=="value")throw Error(JSON.stringify(r));
const w=await tools.known();
if(w.server!=="warm-server"||w.lazyLanguage)throw Error(JSON.stringify(w));
if((await selected.only()).server!=="selected-server")throw Error("selected");
if((await rawselected.foo_bar()).lazySelector!=="foo-bar")throw Error("selector lost");
const p=await partial.known_name({x:"typed"});
if(p.lazyLanguage!=="typescript"||p.args.x!=="typed")throw Error("partial");
if(selected.missing!==undefined||empty.missing!==undefined||cold.then!==undefined)throw Error("unexpected member");
if(await Promise.resolve(cold)!==cold)throw Error("thenable");
console.log("ok");
`
			}
			driverPath := filepath.Join(dir, "driver"+l.ScriptExt)
			if err := os.WriteFile(driverPath, []byte(driver), 0600); err != nil {
				t.Fatal(err)
			}
			ctx, cancel := context.WithTimeout(context.Background(), 20*time.Second)
			defer cancel()
			args := []string{driverPath}
			if language == "typescript" {
				args = []string{"run", "--quiet", "--no-check", "--allow-env", "--allow-net=" + strings.TrimPrefix(srv.URL, "http://"), driverPath}
			}
			cmd := exec.CommandContext(ctx, binary, args...)
			cmd.Dir = dir
			for _, e := range os.Environ() {
				if !strings.HasPrefix(e, "MCPX_") && !strings.HasPrefix(e, "PYTHON") {
					cmd.Env = append(cmd.Env, e)
				}
			}
			cmd.Env = append(cmd.Env, "MCPX_ENDPOINT="+srv.URL, "MCPX_SOCKET=", "DENO_NO_UPDATE_CHECK=1")
			out, err := cmd.CombinedOutput()
			if err != nil {
				t.Fatalf("%v\n%s", err, out)
			}
			if strings.TrimSpace(string(out)) != "ok" {
				t.Fatalf("output %s", out)
			}
		})
	}
}

func TestColdGeneratedDeclarations1868(t *testing.T) {
	deno, err := exec.LookPath("deno")
	if err != nil {
		t.Skip("deno unavailable")
	}
	ns := []codegen.Namespace{{Name: "cold", Server: "cold", Cold: true, Tools: []codegen.Tool{
		{Name: "known-name", InputSchema: json.RawMessage(`{"type":"object","properties":{"x":{"type":"string"}},"required":["x"]}`)},
	}}}
	dir := t.TempDir()
	for name, contents := range map[string]string{
		"client.ts":  codegen.Module(ns, "http://127.0.0.1:1", ""),
		"types.d.ts": "type ToolResult = unknown;\n" + codegen.Declarations(ns),
		"driver.ts": `import {cold} from "./client.ts";
cold.unknown({arbitrary:true});
cold.known_name({x:"yes"});
// @ts-expect-error cached signature remains authoritative
cold.known_name({x:3});
`,
	} {
		if err := os.WriteFile(filepath.Join(dir, name), []byte(contents), 0600); err != nil {
			t.Fatal(err)
		}
	}
	ctx, cancel := context.WithTimeout(context.Background(), 20*time.Second)
	defer cancel()
	cmd := exec.CommandContext(ctx, deno, "check", "--quiet", "client.ts", "types.d.ts", "driver.ts")
	cmd.Dir = dir
	cmd.Env = append(os.Environ(), "DENO_NO_UPDATE_CHECK=1", "NO_COLOR=1")
	if out, err := cmd.CombinedOutput(); err != nil {
		t.Fatalf("%v\n%s", err, out)
	}
}
