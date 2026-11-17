package e2e_test

import (
	"os"
	"path/filepath"
	"strings"
	"testing"
)

// The #8460 burn-down: the cli configuration group's commands had no
// integration-test exercise, so the feature-coverage guard carried
// them as exemptions. Each test below dispatches one command for real
// through the coverage-recording runner (a help-only invocation would
// be classified as "help" by resolveCLICommand and record nothing).

func TestCoverageCLIAdapterList(t *testing.T) {
	e := newEnv(t, "{}")
	out := e.run("adapter", "list")
	if !strings.Contains(out, "No adapters declared") {
		t.Fatalf("adapter list with nothing declared:\n%s", out)
	}
}

func TestCoverageCLIAPIListAndTools(t *testing.T) {
	e := newEnv(t, "{}")
	out := e.run("api", "list")
	if !strings.Contains(out, "No API specifications declared") {
		t.Fatalf("api list with nothing declared:\n%s", out)
	}
	spec := `{
  "openapi": "3.0.0",
  "info": { "title": "Coverage Probe", "version": "1.0.0" },
  "servers": [{ "url": "http://127.0.0.1:9" }],
  "paths": {
    "/widgets": {
      "get": { "operationId": "listwidgets", "responses": { "200": { "description": "ok" } } }
    }
  }
}`
	specPath := filepath.Join(e.dir, "probe-openapi.json")
	if err := os.WriteFile(specPath, []byte(spec), 0o644); err != nil {
		t.Fatal(err)
	}
	out = e.run("api", "tools", "--spec", specPath, "--name", "probe")
	if !strings.Contains(out, "listwidgets") || !strings.Contains(out, "1 operations.") {
		t.Fatalf("api tools --spec probe:\n%s", out)
	}
}

func TestCoverageCLICompletion(t *testing.T) {
	e := newEnv(t, "{}")
	out := e.run("completion", "bash")
	if !strings.Contains(out, "mcpx") || len(out) < 500 {
		t.Fatalf("completion bash produced no real script (%d bytes):\n%s", len(out), out)
	}
	if _, err := e.try("completion", "tcsh"); err == nil {
		t.Fatal("completion tcsh: expected a usage error for an unsupported shell")
	}
}

func TestCoverageCLIInitLifecycle(t *testing.T) {
	e := newEnv(t, "{}")
	// The env's own .mcpx.json already exists: a bare init must refuse
	// rather than overwrite it.
	out, err := e.try("init")
	if err == nil || !strings.Contains(out+err.Error(), "already exists") {
		t.Fatalf("init over an existing config: out=%q err=%v", out, err)
	}
	out = e.run("init", "--force")
	if !strings.Contains(out, "wrote .mcpx.json") {
		t.Fatalf("init --force:\n%s", out)
	}
	body, err := os.ReadFile(filepath.Join(e.dir, ".mcpx.json"))
	if err != nil {
		t.Fatal(err)
	}
	if !strings.Contains(string(body), "example-stateless") {
		t.Fatalf("init --force did not write the starter config:\n%s", body)
	}
}

func TestCoverageCLIMan(t *testing.T) {
	e := newEnv(t, "{}")
	out := e.run("man")
	if !strings.Contains(out, ".TH") || !strings.Contains(out, "mcpx") {
		t.Fatalf("man did not print the roff page:\n%s", out[:min(len(out), 400)])
	}
	installDir := filepath.Join(e.dir, "maninstall")
	e.run("man", "--install", installDir)
	page, err := os.ReadFile(filepath.Join(installDir, "man1", "mcpx.1"))
	if err != nil {
		t.Fatalf("man --install did not write the page: %v", err)
	}
	if !strings.Contains(string(page), ".TH") {
		t.Fatalf("installed page is not the roff page:\n%s", page[:min(len(page), 200)])
	}
}

func TestCoverageCLIPluginList(t *testing.T) {
	e := newEnv(t, "{}")
	out := e.run("plugin", "list")
	if !strings.Contains(out, "No plugins discovered") {
		t.Fatalf("plugin list with nothing discoverable:\n%s", out)
	}
}
