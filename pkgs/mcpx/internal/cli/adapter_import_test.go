package cli

import (
	"encoding/json"
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/dezren39/mcpx/internal/adapter"
)

func TestBuildBareImportEmitsLoadableDeclaration(t *testing.T) {
	spec, err := buildBareImport("git", "version control for tests")
	if err != nil {
		t.Fatal(err)
	}
	if !spec.Bare || spec.Name != "git" || spec.Command != "git" {
		t.Fatalf("got %+v", spec)
	}
	if spec.Description == "" {
		t.Fatal("the description must survive")
	}
	// The emission round-trips through the same load path declarations
	// take, unchanged: validation and dedup decide, not the emitter.
	doc, err := json.Marshal(AdapterFile{Adapters: []adapter.Spec{spec}})
	if err != nil {
		t.Fatal(err)
	}
	path := filepath.Join(t.TempDir(), "adapters.json")
	if err := os.WriteFile(path, doc, 0o644); err != nil {
		t.Fatal(err)
	}
	loaded, err := loadAdapterFiles([]string{path})
	if err != nil {
		t.Fatalf("an emitted bare declaration should load: %v", err)
	}
	if len(loaded) != 1 || !loaded[0].Bare || len(loaded[0].ExposedTools()) != 1 {
		t.Fatalf("got %+v", loaded)
	}
}

func TestBuildBareImportDerivesTheName(t *testing.T) {
	spec, err := buildBareImport("/usr/bin/git", "version control for tests")
	if err != nil {
		t.Fatal(err)
	}
	if spec.Name != "git" {
		t.Errorf("the name should be the binary's basename: %+v", spec)
	}
}

func TestBuildBareImportNormalizesDottedNames(t *testing.T) {
	// adapter call splits <name>.<tool> at the first dot, so a
	// dotted basename must not become the adapter name verbatim.
	spec, err := buildBareImport("python3.12", "python for tests")
	if err != nil {
		t.Fatal(err)
	}
	if spec.Name != "python3-12" || strings.Contains(spec.Name, ".") {
		t.Fatalf("dotted basename should normalize to a dot-free name: %+v", spec)
	}
}

func TestBuildBareImportStoresPathBinariesAbsolute(t *testing.T) {
	spec, err := buildBareImport("./bin/tool", "a tool for tests")
	if err != nil {
		t.Fatal(err)
	}
	if !filepath.IsAbs(spec.Command) || !strings.HasSuffix(spec.Command, filepath.Join("bin", "tool")) {
		t.Fatalf("a path-like binary should be stored absolute: %+v", spec)
	}
	plain, err := buildBareImport("git", "version control for tests")
	if err != nil {
		t.Fatal(err)
	}
	if plain.Command != "git" {
		t.Fatalf("a bare command name should be stored unchanged: %+v", plain)
	}
}

func TestBuildBareImportRefusesWithoutDescription(t *testing.T) {
	for _, desc := range []string{"", "   "} {
		if _, err := buildBareImport("git", desc); err == nil {
			t.Fatalf("no description %q should be refused", desc)
		} else if !strings.Contains(err.Error(), "description is required") {
			t.Errorf("the message should name the rule: %v", err)
		}
	}
}

func TestSplitImportArgsLiftsFlagsAnywhere(t *testing.T) {
	// The stdlib parser stops at the first positional; import must not.
	bare, desc, out, rest, err := splitImportArgs(
		[]string{"--bare", "echo", "--description", "lines", "--out", "f.json"})
	if err != nil {
		t.Fatal(err)
	}
	if !bare || desc != "lines" || out != "f.json" || len(rest) != 1 || rest[0] != "echo" {
		t.Errorf("got bare=%v desc=%q out=%q rest=%q", bare, desc, out, rest)
	}
	bare, desc, out, rest, err = splitImportArgs(
		[]string{"--description=lines", "--out=f.json", "--bare", "echo"})
	if err != nil || !bare || desc != "lines" || out != "f.json" || len(rest) != 1 {
		t.Errorf("the = forms should work too: %v %q %q %q", err, desc, out, rest)
	}
}

func TestSplitImportArgsRefusesUnknownFlags(t *testing.T) {
	if _, _, _, _, err := splitImportArgs([]string{"--bare", "--from-man", "echo"}); err == nil {
		t.Fatal("an unknown flag should be refused")
	} else if !strings.Contains(err.Error(), "--from-man") {
		t.Errorf("the message should name it: %v", err)
	}
}

func TestBareWarningStatesTheCosts(t *testing.T) {
	spec, err := buildBareImport("git", "version control for tests")
	if err != nil {
		t.Fatal(err)
	}
	warning := bareImportWarning(spec)
	for _, want := range []string{
		"WARNING",
		"no per-subcommand validation",
		"argv",
		"shell escape",
		"git",
	} {
		if !strings.Contains(warning, want) {
			t.Errorf("the warning should state %q:\n%s", want, warning)
		}
	}
}
