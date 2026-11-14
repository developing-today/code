package testsupport

import (
	"os"
	"path/filepath"
	"strings"
	"testing"
)

// The override resolver is the whole contract of MCPX_TEST_BINARY
// (#1980): unset builds as before; set must yield the named executable
// or a loud error. Oracle: the filesystem state each case plants.

func TestResolveMCPXOverrideUnset(t *testing.T) {
	got, err := resolveMCPXOverride("")
	if err != nil || got != "" {
		t.Fatalf("unset override = (%q, %v), want (\"\", nil)", got, err)
	}
}

func TestResolveMCPXOverrideExecutable(t *testing.T) {
	dir := t.TempDir()
	bin := filepath.Join(dir, "mcpx")
	if err := os.WriteFile(bin, []byte("#!/bin/sh\nexit 0\n"), 0o755); err != nil {
		t.Fatal(err)
	}
	got, err := resolveMCPXOverride(bin)
	if err != nil || got != bin {
		t.Fatalf("executable override = (%q, %v), want (%q, nil)", got, err, bin)
	}
}

func TestResolveMCPXOverrideRelativePath(t *testing.T) {
	dir := t.TempDir()
	bin := filepath.Join(dir, "mcpx")
	if err := os.WriteFile(bin, []byte("x"), 0o755); err != nil {
		t.Fatal(err)
	}
	t.Chdir(dir)
	got, err := resolveMCPXOverride("./mcpx")
	if err != nil || got != bin {
		t.Fatalf("relative override = (%q, %v), want (%q, nil)", got, err, bin)
	}
}

func TestResolveMCPXOverrideMissing(t *testing.T) {
	_, err := resolveMCPXOverride(filepath.Join(t.TempDir(), "nope"))
	if err == nil || !strings.Contains(err.Error(), mcpxBinaryOverrideEnv) {
		t.Fatalf("missing override err = %v, want an error naming %s", err, mcpxBinaryOverrideEnv)
	}
}

func TestResolveMCPXOverrideDirectory(t *testing.T) {
	_, err := resolveMCPXOverride(t.TempDir())
	if err == nil || !strings.Contains(err.Error(), "directory") {
		t.Fatalf("directory override err = %v, want a directory error", err)
	}
}

func TestResolveMCPXOverrideNotExecutable(t *testing.T) {
	dir := t.TempDir()
	bin := filepath.Join(dir, "mcpx")
	if err := os.WriteFile(bin, []byte("x"), 0o644); err != nil {
		t.Fatal(err)
	}
	_, err := resolveMCPXOverride(bin)
	if err == nil || !strings.Contains(err.Error(), "not executable") {
		t.Fatalf("non-executable override err = %v, want a not-executable error", err)
	}
}
