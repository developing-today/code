package runner

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

func TestRocLanguageDetectionAndSelection(t *testing.T) {
	cases := []struct{ file, source, explicit, want string }{
		{"report.roc", "", "", codegen.Roc.Name},
		{"", "#!/usr/bin/env roc\napp [main] {}\n", "", codegen.Roc.Name},
		{"report.ts", "", "roc", codegen.Roc.Name},
		{"report.roc", "", "typescript", codegen.TypeScript.Name},
	}
	for _, tc := range cases {
		if got := EffectiveScriptLanguage(tc.file, tc.source, tc.explicit); got != tc.want {
			t.Errorf("EffectiveScriptLanguage(%q, %q, %q) = %q, want %q", tc.file, tc.source, tc.explicit, got, tc.want)
		}
	}
	if got := ClientScriptLanguage("", "#!/usr/bin/env roc\n", "typescript"); got != codegen.Roc.Name {
		t.Errorf("Roc shebang client language = %q, want roc", got)
	}
	if got, err := NormalizeLanguage(" Roc "); err != nil || got != codegen.Roc.Name {
		t.Errorf("NormalizeLanguage(roc) = %q, %v", got, err)
	}
}

func TestRocFileRunCallsMockDaemon(t *testing.T) {
	if _, err := exec.LookPath("roc"); err != nil {
		t.Skip("roc compiler is not installed")
	}
	seen := make(chan map[string]any, 1)
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.URL.Path != "/v1/call" {
			http.NotFound(w, r)
			return
		}
		var body map[string]any
		if err := json.NewDecoder(r.Body).Decode(&body); err != nil {
			t.Errorf("mock daemon: decode call: %v", err)
		}
		seen <- body
		w.Header().Set("content-type", "application/json")
		_, _ = w.Write([]byte(`{"result":{"structuredContent":{"ok":true}}}`))
	}))
	defer srv.Close()

	nss := []codegen.Namespace{{Name: "demo", Server: "demo-server", Tools: []codegen.Tool{{
		Name: "fancy-name", InputSchema: []byte(`{"type":"object","properties":{"x":{"type":"string"}},"required":["x"]}`),
	}}}}
	dir := t.TempDir()
	script := filepath.Join(dir, "report.roc")
	main := "app [main!] { pf: platform \"https://github.com/roc-lang/basic-cli/releases/download/0.28.1/7brgwR8YUqfeGrkdfF1V4GPPN7DnP7ftqub9KcrTEZtr.tar.zst\", http: \"https://github.com/roc-lang/http/releases/download/1.0.0/6ZUwqYhCS8PU9Mo6MF7oV82ET2o7KYb57CLKDq4cq4sS.tar.zst\" }\n\nimport Mcpx\n\nmain! = |_args| {\n    _ = Mcpx.demo_fancy_name!({ x: \"world\" })\n    Ok({})\n}\n"
	if err := os.WriteFile(script, []byte(main), 0o600); err != nil {
		t.Fatal(err)
	}
	var stdout, stderr bytes.Buffer
	res, err := Run(context.Background(), Options{
		File:    script,
		Clients: map[string]Client{codegen.Roc.Name: {Module: codegen.RocModule(nss, srv.URL, "sess-roc")}},
		Env:     map[string]string{"MCPX_ENDPOINT": srv.URL, "MCPX_SESSION": "sess-roc"},
		Stdout:  &stdout,
		Stderr:  &stderr,
	})
	if err != nil {
		t.Fatalf("roc run failed: %v\nstderr:\n%s", err, stderr.String())
	}
	if !strings.Contains(res.Runtime, "roc") {
		t.Errorf("Result.Runtime = %q, want Roc interpreter", res.Runtime)
	}
	select {
	case got := <-seen:
		if got["server"] != "demo-server" || got["tool"] != "fancy-name" {
			t.Errorf("daemon call = %#v, want demo-server/fancy-name", got)
		}
		args, _ := got["args"].(map[string]any)
		if args["x"] != "world" {
			t.Errorf("daemon args = %#v, want x=world", args)
		}
	default:
		t.Fatal("Roc tool call did not reach mock daemon")
	}
}
