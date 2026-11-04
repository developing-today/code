package cli

import (
	"path/filepath"
	"testing"
)

// The tailscale declaration shipped in examples/ is the #1936 item-1
// deliverable: a paths.adapters spec over the local tailscale CLI exposing
// exactly listTailnet and listHostnames, with no credential anywhere in it
// (the CLI session the user already has is the authentication). It is
// pinned here so the shipped file cannot rot into a shape the loader
// would reject or a tool set wider than the request asked for.
func TestShippedTailscaleAdapterDeclaration(t *testing.T) {
	path := filepath.Join("..", "..", "examples", "tailscale.json")
	specs, err := loadAdapterFiles([]string{path})
	if err != nil {
		t.Fatalf("shipped tailscale declaration does not load: %v", err)
	}
	if len(specs) != 1 {
		t.Fatalf("got %d adapters, want exactly 1", len(specs))
	}
	spec := specs[0]
	if spec.Name != "tailscale" || spec.Command != "tailscale" {
		t.Fatalf("adapter is %q over %q, want tailscale over tailscale", spec.Name, spec.Command)
	}
	if len(spec.Env) != 0 || spec.Runtime != "" || spec.Bare {
		t.Fatalf("declaration must carry no env, runtime or bare mode: %+v", spec)
	}
	tools := spec.ExposedTools()
	if len(tools) != 2 {
		t.Fatalf("got %d tools, want exactly listTailnet and listHostnames", len(tools))
	}
	for i, name := range []string{"listTailnet", "listHostnames"} {
		tool := tools[i]
		if tool.Name != name {
			t.Fatalf("tool %d is %q, want %q", i, tool.Name, name)
		}
		if len(tool.Args) != 2 || tool.Args[0] != "status" || tool.Args[1] != "--json" {
			t.Fatalf("%s args are %v, want [status --json]", name, tool.Args)
		}
		if !tool.JSON {
			t.Fatalf("%s must parse the status output as JSON", name)
		}
		if len(tool.Params) != 0 {
			t.Fatalf("%s takes no caller parameters, got %+v", name, tool.Params)
		}
	}
}
