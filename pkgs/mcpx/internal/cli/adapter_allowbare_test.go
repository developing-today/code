package cli

import (
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/developing-today/code/pkgs/mcpx/internal/adapter"
)

// The adapters.allowBare gate (#11147): a bare declaration is a
// passthrough of a binary's whole argv surface, so it loads only when
// the setting says so. These tests drive loadAdapters the way a user
// reaches it -- a declaration file named by MCPX_PATHS_ADAPTERS and the
// setting itself from the environment -- and pin the permutations: off
// by default, off explicitly, on, and a non-bare declaration unaffected
// either way.

const bareDecl = `{"adapters":[{"name":"git","command":"git","description":"git, passed straight through","bare":true}]}`
const plainDecl = `{"adapters":[{"name":"jq","command":"jq","tools":[{"name":"run","params":[{"name":"filter"}]}]}]}`

func writeDecl(t *testing.T, decl string) string {
	t.Helper()
	file := filepath.Join(t.TempDir(), "adapters.json")
	if err := os.WriteFile(file, []byte(decl), 0o644); err != nil {
		t.Fatal(err)
	}
	return file
}

func TestBareDeclarationRefusedWhileAllowBareOff(t *testing.T) {
	for _, env := range []string{"", "false"} {
		name := "unset"
		if env != "" {
			name = env
		}
		t.Run(name, func(t *testing.T) {
			if env != "" {
				t.Setenv("MCPX_ADAPTERS_ALLOW_BARE", env)
			}
			a := adapterApp(t, writeDecl(t, bareDecl))
			_, err := a.loadAdapters()
			if err == nil {
				t.Fatal("a bare declaration loaded while adapters.allowBare is off")
			}
			if !strings.Contains(err.Error(), "adapters.allowBare") || !strings.Contains(err.Error(), `"git"`) {
				t.Fatalf("the refusal should name the adapter and the setting that permits it, got: %v", err)
			}
		})
	}
}

func TestBareDeclarationLoadsWhenAllowBareOn(t *testing.T) {
	t.Setenv("MCPX_ADAPTERS_ALLOW_BARE", "true")
	a := adapterApp(t, writeDecl(t, bareDecl))
	specs, err := a.loadAdapters()
	if err != nil {
		t.Fatalf("a bare declaration should load once adapters.allowBare is on: %v", err)
	}
	if len(specs) != 1 || !specs[0].Bare || specs[0].Name != "git" {
		t.Fatalf("loaded specs %+v, want the one bare git declaration", specs)
	}
}

func TestNonBareDeclarationLoadsEitherWay(t *testing.T) {
	for _, on := range []bool{false, true} {
		if on {
			t.Setenv("MCPX_ADAPTERS_ALLOW_BARE", "true")
		}
		a := adapterApp(t, writeDecl(t, plainDecl))
		specs, err := a.loadAdapters()
		if err != nil {
			t.Fatalf("allowBare on=%v: a declared-tools adapter should load regardless: %v", on, err)
		}
		if len(specs) != 1 || specs[0].Bare {
			t.Fatalf("allowBare on=%v: loaded specs %+v, want the one non-bare jq declaration", on, specs)
		}
	}
}

// The gate itself, as a class guard: every bare spec in a mixed load is
// refused, not just the first declaration in the file, and the refusal
// names the bare one rather than an innocent neighbour.
func TestCheckBareAllowedRefusesAnyBareSpec(t *testing.T) {
	specs := []adapter.Spec{
		{Name: "jq", Command: "jq"},
		{Name: "git", Command: "git", Bare: true},
		{Name: "rg", Command: "rg", Bare: true},
	}
	err := checkBareAllowed(specs, false)
	if err == nil || !strings.Contains(err.Error(), `"git"`) {
		t.Fatalf("want a refusal naming the first bare spec, got %v", err)
	}
	if err := checkBareAllowed(specs, true); err != nil {
		t.Fatalf("allowBare on should pass any load: %v", err)
	}
	if err := checkBareAllowed(specs[:1], false); err != nil {
		t.Fatalf("a load with no bare spec should pass while the gate is off: %v", err)
	}
}
