package buildcheck

import (
	"os"
	"path/filepath"
	"regexp"
	"strings"
	"testing"
)

var (
	nixVersionLine = regexp.MustCompile(`(?m)^\s*version = "([^"]+)";\s*$`)
	ldVersionStamp = regexp.MustCompile(`-X main\.version=([^"\s]+)`)
	mainVersionVar = regexp.MustCompile(`(?m)^var version = "([^"]+)"`)
)

// TestVersionHasOneSourceOfTruth is the guard for #1895: the version literal
// in package.nix, the ldflags build stamp and the default in cmd/mcpx/main.go
// are three sites for one value, so a disagreement must fail the build rather
// than ship a binary whose --version is not the package's version.
func TestVersionHasOneSourceOfTruth(t *testing.T) {
	nix := readPackageNix(t)

	nixMatch := nixVersionLine.FindStringSubmatch(nix)
	if nixMatch == nil {
		t.Fatal("package.nix has no `version = \"...\";` line")
	}
	nixVersion := nixMatch[1]

	stamps := ldVersionStamp.FindAllStringSubmatch(nix, -1)
	if len(stamps) != 1 {
		t.Fatalf("package.nix has %d `-X main.version=` stamps; want exactly one", len(stamps))
	}
	if got := stamps[0][1]; got != "${version}" {
		t.Errorf("package.nix ldflags stamp is %q; it must interpolate ${version} so the literal is written once (package.nix version = %q)", got, nixVersion)
	}

	mainSrc, err := os.ReadFile(filepath.Join(moduleRoot, "cmd", "mcpx", "main.go"))
	if err != nil {
		t.Fatal(err)
	}
	mainMatch := mainVersionVar.FindStringSubmatch(string(mainSrc))
	if mainMatch == nil {
		t.Fatal("cmd/mcpx/main.go has no `var version = \"...\"` default")
	}
	if mainMatch[1] != nixVersion {
		t.Errorf("version disagreement: package.nix says %q, cmd/mcpx/main.go default says %q", nixVersion, mainMatch[1])
	}

	// A second hard-coded literal in the derivation would reintroduce the
	// drift this test exists to catch.
	if strings.Count(nix, `"`+nixVersion+`"`) != 1 {
		t.Errorf("package.nix mentions %q more than once besides the version line; keep the literal in one place", nixVersion)
	}
}
