package artifacts_test

import (
	"os"
	"path/filepath"
	"strings"
	"testing"
)

// TestDeliveryClaimsMatchCopyImplementation keeps user-facing and generated
// artifact text aligned with the store. If artifact delivery gains an actual
// os.Link path, the wording can be reviewed alongside that implementation.
func TestDeliveryClaimsMatchCopyImplementation(t *testing.T) {
	implementation, err := os.ReadFile("artifacts.go")
	if err != nil {
		t.Fatal(err)
	}
	if strings.Contains(string(implementation), "os.Link(") || strings.Contains(string(implementation), "os.Linkname") {
		return
	}

	files := []string{
		"internal/artifacts/artifacts.go",
		"internal/api/ops_exec.go",
		"internal/settings/exec.go",
		"internal/execsvc/execsvc.go",
		"internal/daemon/routes_exec.go",
		"internal/cli/exec.go",
		"internal/cli/commands.go",
		"internal/codegen/emit.go",
		"internal/e2e/exec_test.go",
		"docs/exec.md",
		"docs/remote.md",
		"docs/home-example/client.ts",
		"docs/home-example/client.browser.js",
	}
	for _, name := range files {
		path := filepath.Join("..", "..", filepath.FromSlash(name))
		contents, err := os.ReadFile(path)
		if err != nil {
			t.Errorf("read %s: %v", name, err)
			continue
		}
		text := strings.ToLower(string(contents))
		text = strings.ReplaceAll(text, "hard-linked", "hardlinked")
		text = strings.ReplaceAll(text, "hard-link", "hardlink")
		text = strings.ReplaceAll(text, "hard link", "hardlink")
		if strings.Contains(text, "hardlink") || strings.Contains(text, "costs an inode") || strings.Contains(text, "inode rather than a copy") {
			t.Errorf("%s still promises hardlink delivery without an os.Link path in artifacts", name)
		}
	}
}
