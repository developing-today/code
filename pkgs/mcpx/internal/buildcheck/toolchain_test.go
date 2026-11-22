package buildcheck

import (
	"os"
	"path/filepath"
	"regexp"
	"testing"
)

var (
	goModDirective  = regexp.MustCompile(`(?m)^go (\d+\.\d+\.\d+)$`)
	toolchainWanted = regexp.MustCompile(`(?m)^\s*wanted = "([^"]+)";$`)
)

// TestNixToolchainMatchesGoMod keeps go-toolchain.nix in step with
// go.mod. A sandboxed nix build cannot download a newer Go, so if
// go.mod's `go` line moves past the release go-toolchain.nix builds,
// the package stops building; this catches that before nix does.
func TestNixToolchainMatchesGoMod(t *testing.T) {
	mod, err := os.ReadFile(filepath.Join(moduleRoot, "go.mod"))
	if err != nil {
		t.Fatal(err)
	}
	m := goModDirective.FindSubmatch(mod)
	if m == nil {
		t.Fatal("go.mod has no `go X.Y.Z` line")
	}
	nix, err := os.ReadFile(filepath.Join(moduleRoot, "go-toolchain.nix"))
	if err != nil {
		t.Fatal(err)
	}
	w := toolchainWanted.FindSubmatch(nix)
	if w == nil {
		t.Fatal("go-toolchain.nix has no `wanted = \"X.Y.Z\";` line")
	}
	if string(m[1]) != string(w[1]) {
		t.Errorf("go.mod asks for Go %s but go-toolchain.nix builds %s; update wanted and its source hash together", m[1], w[1])
	}
}
