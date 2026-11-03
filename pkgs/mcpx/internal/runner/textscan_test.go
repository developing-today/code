package runner

import (
	"os"
	"path/filepath"
	"testing"

	"github.com/developing-today/code/pkgs/mcpx/internal/preflight"
)

// Codex round (PR #2575): a computed import in a launcher phase must be
// visible to the text-mode import scan. The scan text is the script
// plus the generated launcher, whose phase snippets are executable.
func TestTextScanSourceIncludesLauncherPhases(t *testing.T) {
	dir := t.TempDir()
	launcher := filepath.Join(dir, "launcher.ts")
	content := "// phase: before\nawait import(name);\n"
	if err := os.WriteFile(launcher, []byte(content), 0o644); err != nil {
		t.Fatal(err)
	}
	scan := textScanSource(Options{Source: "return 1;\n"}, launcher)
	dyn := preflight.ScanDynamicImports("the script", scan)
	if len(dyn) != 1 {
		t.Fatalf("phase import: got %+v, want 1 dynamic import from the launcher content", dyn)
	}
	// An unreadable launcher degrades to the script alone.
	scan = textScanSource(Options{Source: "return 1;\n"}, filepath.Join(dir, "missing.ts"))
	if dyn := preflight.ScanDynamicImports("the script", scan); len(dyn) != 0 {
		t.Fatalf("missing launcher: got %+v, want no dynamic imports", dyn)
	}
}
