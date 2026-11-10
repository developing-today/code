package buildcheck

import (
	"os"
	"path/filepath"
	"regexp"
	"strings"
	"testing"
)

var (
	nixVersionDerivation = regexp.MustCompile(`version = lib\.removeSuffix "\\n" \(builtins\.readFile \./VERSION\);`)
	ldVersionStamp       = regexp.MustCompile(`-X main\.version=([^"\s]+)`)
	mainVersionVar       = regexp.MustCompile(`(?m)^var version = "([^"]+)"`)
	containerVersionArg  = regexp.MustCompile(`(?m)^ARG VERSION=(\S+)$`)
)

// devSentinel is the one version literal allowed outside VERSION
// (decision 0005 §3): a build that was not stamped announces itself
// instead of impersonating a release.
const devSentinel = "0.0.0-dev"

// TestVersionHasOneSourceOfTruth is the guard for #1895, updated for
// decision 0005 §3 (#1881 slice 2): pkgs/mcpx/VERSION is the version's
// single source. package.nix derives its version from that file and
// interpolates it into the ldflags stamp; cmd/mcpx/main.go's default
// and the Containerfile's ARG default are the 0.0.0-dev sentinel, so
// no second copy of the release literal exists to drift.
func TestVersionHasOneSourceOfTruth(t *testing.T) {
	vb, err := os.ReadFile(filepath.Join(moduleRoot, "VERSION"))
	if err != nil {
		t.Fatalf("VERSION file (the version's single source, decision 0005 §3): %v", err)
	}
	version := strings.TrimSpace(string(vb))
	if version == "" || strings.ContainsAny(version, " \t\r\n") {
		t.Fatalf("VERSION file content %q is not a single version token", string(vb))
	}
	if version == devSentinel {
		t.Fatalf("VERSION file holds the dev sentinel; it must name a release version")
	}

	nix := readPackageNix(t)
	if !nixVersionDerivation.MatchString(nix) {
		t.Error("package.nix does not derive its version from ./VERSION via lib.removeSuffix/builtins.readFile (decision 0005 §3)")
	}
	stamps := ldVersionStamp.FindAllStringSubmatch(nix, -1)
	if len(stamps) != 1 {
		t.Fatalf("package.nix has %d `-X main.version=` stamps; want exactly one", len(stamps))
	}
	if got := stamps[0][1]; got != "${version}" {
		t.Errorf("package.nix ldflags stamp is %q; it must interpolate ${version} so VERSION is written once", got)
	}
	// A hard-coded copy of the release literal in the derivation would
	// reintroduce the drift this test exists to catch.
	if strings.Contains(nix, `"`+version+`"`) {
		t.Errorf("package.nix contains the version literal %q; VERSION is the only place it may be written", version)
	}

	mainSrc, err := os.ReadFile(filepath.Join(moduleRoot, "cmd", "mcpx", "main.go"))
	if err != nil {
		t.Fatal(err)
	}
	mainMatch := mainVersionVar.FindStringSubmatch(string(mainSrc))
	if mainMatch == nil {
		t.Fatal("cmd/mcpx/main.go has no `var version = \"...\"` default")
	}
	if mainMatch[1] != devSentinel {
		t.Errorf("cmd/mcpx/main.go default is %q; want the %q sentinel (decision 0005 §3) — release binaries are stamped from VERSION via ldflags", mainMatch[1], devSentinel)
	}

	cf, err := os.ReadFile(filepath.Join(moduleRoot, "Containerfile"))
	if err != nil {
		t.Fatal(err)
	}
	cfMatch := containerVersionArg.FindStringSubmatch(string(cf))
	if cfMatch == nil {
		t.Fatal("Containerfile has no `ARG VERSION=` line")
	}
	if cfMatch[1] != devSentinel {
		t.Errorf("Containerfile ARG VERSION default is %q; want the %q sentinel — the release workflow passes VERSION's contents as a build arg (decision 0005 §3)", cfMatch[1], devSentinel)
	}
}
