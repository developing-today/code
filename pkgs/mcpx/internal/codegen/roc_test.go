package codegen_test

import (
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"testing"

	"github.com/developing-today/code/pkgs/mcpx/internal/codegen"
)

func TestRocLanguageEntryAndClient(t *testing.T) {
	l, ok := codegen.Lookup("roc")
	if !ok || l.ClientFile != "Mcpx.roc" || l.ScriptExt != ".roc" || l.ContentType != "text/x-roc" {
		t.Fatalf("Roc language entry = %+v, found=%v", l, ok)
	}
	nss := []codegen.Namespace{{Name: "demo", Server: "demo-server", Tools: []codegen.Tool{{
		Name: "say-hi", InputSchema: []byte(`{"type":"object","properties":{"name":{"type":"string"},"count":{"type":"integer"}}}`),
	}}}}
	out := l.EmitModule(nss, "http://127.0.0.1:1", "test-session")
	for _, want := range []string{"import pf.Http", "import pf.Env", "import http.Request", "TransportError(Str)", "StatusError(U16)", "Try(Response.Response, ToolError)", "name : Str", "count : I64", "Http.send!(request, {})", "x-mcpx-session", "demo_say_hi! = |args|"} {
		if !strings.Contains(out, want) {
			t.Errorf("Roc client missing %q:\n%s", want, out)
		}
	}
	if got := l.EmitGlobals(nss); got != "" {
		t.Errorf("Roc has no ambient globals, got %q", got)
	}
}

func TestRocNamesAvoidReservedWordsAndSanitizationCollisions(t *testing.T) {
	nss := []codegen.Namespace{{Name: "demo", Server: "demo", Tools: []codegen.Tool{
		{Name: "say-hi", InputSchema: []byte(`{"type":"object","properties":{"if":{"type":"string"},"a-b":{"type":"string"},"a_b":{"type":"string"}}}`)},
		{Name: "say_hi", InputSchema: []byte(`{"type":"object"}`)},
	}}}
	out := codegen.RocModule(nss, "", "")
	for _, want := range []string{"demo_say_hi! :", "demo_say_hi_2! :", "if_ : Str", "a_b : Str", "a_b_2 : Str"} {
		if !strings.Contains(out, want) {
			t.Errorf("generated Roc names missing %q:\n%s", want, out)
		}
	}
	if got := strings.Count(out, "demo_say_hi!"); got != 2 {
		t.Errorf("first tool declaration/reference count = %d, want 2", got)
	}
}

func TestRocStringUsesRocEscapes(t *testing.T) {
	out := codegen.RocModule(nil, "quote\" slash\\ line\n", "")
	if !strings.Contains(out, `default_endpoint = "quote\" slash\\ line\n"`) {
		t.Errorf("Roc endpoint did not use Roc string escapes:\n%s", out)
	}
}

func TestRocNamesAndStringEscapingAreStable(t *testing.T) {
	nss := []codegen.Namespace{
		{Name: "demo", Server: "one", Tools: []codegen.Tool{{Name: "say-hi"}}},
		{Name: "demo_say", Server: "two", Tools: []codegen.Tool{{Name: "hi"}}},
		{Name: "schema", Server: "three", Tools: []codegen.Tool{{
			Name: "fields", InputSchema: []byte(`{"type":"object","properties":{"if":{"type":"string"},"if_":{"type":"string"}}}`),
		}}},
	}
	out := codegen.RocModule(nss, "https://example.test/\"path\n", "session\tvalue")
	for _, want := range []string{
		`default_endpoint = "https://example.test/\"path\n"`,
		`default_session = "session\tvalue"`,
		`demo_say_hi! : { _unit : {} }`,
		`demo_say_hi_2! : { _unit : {} }`,
		`schema_fields! : { if_ : Str, if__2 : Str }`,
	} {
		if !strings.Contains(out, want) {
			t.Errorf("Roc output missing %q:\n%s", want, out)
		}
	}
}

// The installed nightly is the syntax oracle for the generated Roc module.
// The application platform provides the Http API; Approach C (a custom mcpx
// platform) is intentionally not part of this test or implementation.
func TestRocModulePassesCompiler(t *testing.T) {
	roc, err := exec.LookPath("roc")
	if err != nil {
		t.Skip("roc compiler is not installed")
	}
	dir := t.TempDir()
	nss := []codegen.Namespace{{Name: "demo", Server: "demo-server", Tools: []codegen.Tool{{
		Name: "say-hi", InputSchema: []byte(`{"type":"object","properties":{"name":{"type":"string"},"count":{"type":"integer"}}}`),
	}}}}
	client := codegen.RocModule(nss, "http://127.0.0.1:1", "test-session")
	main := "app [main!] { pf: platform \"https://github.com/roc-lang/basic-cli/releases/download/0.28.1/7brgwR8YUqfeGrkdfF1V4GPPN7DnP7ftqub9KcrTEZtr.tar.zst\", http: \"https://github.com/roc-lang/http/releases/download/1.0.0/6ZUwqYhCS8PU9Mo6MF7oV82ET2o7KYb57CLKDq4cq4sS.tar.zst\" }\n\nimport Mcpx\n\nmain! = |_args| {\n    _ = Mcpx.demo_say_hi!({ name: \"hello\", count: 1 })\n    Ok({})\n}\n"
	if err := os.WriteFile(filepath.Join(dir, "Mcpx.roc"), []byte(client), 0o600); err != nil {
		t.Fatal(err)
	}
	app := filepath.Join(dir, "main.roc")
	if err := os.WriteFile(app, []byte(main), 0o600); err != nil {
		t.Fatal(err)
	}
	platformURL := "https://github.com/roc-lang/basic-cli/releases/download/0.28.1/7brgwR8YUqfeGrkdfF1V4GPPN7DnP7ftqub9KcrTEZtr.tar.zst"
	cmd := exec.Command(roc, "check", app)
	cache, _ := os.UserCacheDir()
	localPlatform := filepath.Join(cache, "roc", "packages", "github.com", "roc-lang", "basic-cli", "releases", "download", "0.28.1", "7brgwR8YUqfeGrkdfF1V4GPPN7DnP7ftqub9KcrTEZtr", "main.roc")
	if _, err := os.Stat(localPlatform); err == nil {
		cmd = exec.Command(roc, "check", "--replace-dep", platformURL, localPlatform, app)
	}
	if out, err := cmd.CombinedOutput(); err != nil {
		if strings.Contains(string(out), "package download failed") || strings.Contains(string(out), "invalid package dependency") {
			t.Skipf("Roc basic-cli platform is unavailable/incompatible with this compiler: %s", out)
		}
		t.Fatalf("roc check rejected generated client: %v\n%s\n--- client ---\n%s", err, out, client)
	}
}
