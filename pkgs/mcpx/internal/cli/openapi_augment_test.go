package cli

import (
	"context"
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/dezren39/mcpx/internal/config"
	"github.com/dezren39/mcpx/internal/daemon"
	"github.com/dezren39/mcpx/internal/testsupport"
)

// apiApp builds an App whose paths.apis names file, in a scratch home so
// no real declaration can leak into the test.
func apiApp(t *testing.T, apisFile string) *App {
	t.Helper()
	root := testsupport.TempDir(t)
	t.Setenv("HOME", root)
	t.Setenv("XDG_CONFIG_HOME", filepath.Join(root, "xdg"))
	t.Setenv("MCPX_PATHS_APIS", apisFile)
	cfg := filepath.Join(root, "mcpx.json")
	if err := os.WriteFile(cfg, []byte(`{"mcpServers":{}}`), 0o600); err != nil {
		t.Fatal(err)
	}
	return &App{ConfigPath: cfg, Paths: daemon.PathsAt(root, "")}
}

func writeAPIDecl(t *testing.T, dir, specPath string) string {
	t.Helper()
	decl := `{"apis":[{"name":"petstore","spec":` + quoteJSON(specPath) + `,"description":"pets"}]}`
	p := filepath.Join(dir, "apis.json")
	if err := os.WriteFile(p, []byte(decl), 0o644); err != nil {
		t.Fatal(err)
	}
	return p
}

func quoteJSON(s string) string {
	return `"` + strings.ReplaceAll(s, `\`, `\\`) + `"`
}

const petstoreSpec = `{
  "openapi": "3.0.0",
  "info": {"title": "Petstore", "version": "1"},
  "servers": [{"url": "http://127.0.0.1:9"}],
  "paths": {
    "/pets": {
      "get": {"operationId": "listPets", "responses": {"200": {"description": "ok"}}}
    }
  }
}`

// A declared API becomes a server in the config the daemon builds pools
// from -- the first-class requirement of #2118 (split of #1875), not an
// extra. Before this, declared APIs were extras-only: they reached
// mcpx's own tools/list and not the pool, ls, catalog or /v1.
func TestAugmentAPIsAddsServer(t *testing.T) {
	dir := t.TempDir()
	spec := filepath.Join(dir, "petstore.json")
	if err := os.WriteFile(spec, []byte(petstoreSpec), 0o644); err != nil {
		t.Fatal(err)
	}
	a := apiApp(t, writeAPIDecl(t, dir, spec))

	cfg := &config.Config{MCPServers: map[string]*config.Server{}}
	if err := a.augmentAPIs(cfg); err != nil {
		t.Fatal(err)
	}
	s, ok := cfg.MCPServers["petstore"]
	if !ok {
		t.Fatalf("api not added as a server: %v", cfg.MCPServers)
	}
	want := []string{"api", "serve", "petstore"}
	if len(s.Args) < 3 || strings.Join(s.Args[:3], " ") != strings.Join(want, " ") {
		t.Fatalf("server does not serve the api: args %v", s.Args)
	}
	if len(s.Args) != 4 || !filepath.IsAbs(s.Args[3]) {
		t.Fatalf("server args must name the declaration file absolutely: %v", s.Args)
	}
	if s.Command == "" {
		t.Fatal("server has no command; it must be mcpx itself")
	}
	if s.Env["MCPX_API_DECLARATION_SHA256"] == "" {
		t.Fatal("server env lacks the declaration fingerprint; a declaration edit would not replace the pool on reload")
	}
}

// A name already taken by a configured server is a refusal, not a
// silent shadow -- the same rule adapters and plugins follow.
func TestAugmentAPIsRefusesNameCollision(t *testing.T) {
	dir := t.TempDir()
	a := apiApp(t, writeAPIDecl(t, dir, filepath.Join(dir, "spec.json")))
	cfg := &config.Config{MCPServers: map[string]*config.Server{
		"petstore": {Name: "petstore", Command: "other"},
	}}
	if err := a.augmentAPIs(cfg); err == nil {
		t.Fatal("expected a collision refusal")
	}
}

// The extras a served API presents are its operations, named for the
// API -- the same tools apiTools has always exposed, now also behind
// `mcpx api serve`.
func TestAPIExtrasNamesToolsForTheAPI(t *testing.T) {
	dir := t.TempDir()
	spec := filepath.Join(dir, "petstore.json")
	if err := os.WriteFile(spec, []byte(petstoreSpec), 0o644); err != nil {
		t.Fatal(err)
	}
	a := apiApp(t, writeAPIDecl(t, dir, spec))
	specs, err := a.loadAPIs()
	if err != nil || len(specs) != 1 {
		t.Fatalf("loadAPIs: %v (%d specs)", err, len(specs))
	}
	extras := a.apiExtras(context.Background(), specs)
	if len(extras) != 1 || extras[0].Tool.Name != "petstore_listpets" {
		t.Fatalf("unexpected extras: %+v", extras)
	}
}
