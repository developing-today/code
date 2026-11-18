package testsupport

import (
	"os"
	"os/exec"
	"os/user"
	"path/filepath"
	"strings"
	"testing"
)

func envValue(env []string, name string) (string, bool) {
	for _, kv := range env {
		k, v, ok := strings.Cut(kv, "=")
		if ok && k == name {
			return v, true
		}
	}
	return "", false
}

func TestSharedBuildCacheDir(t *testing.T) {
	home := t.TempDir()
	got, ok := sharedBuildCacheDir(home)
	if !ok {
		t.Fatal("sharedBuildCacheDir reported not-ok for a writable home")
	}
	want := filepath.Join(home, ".cache", "mcpx-testsupport")
	if got != want {
		t.Fatalf("sharedBuildCacheDir = %q, want %q", got, want)
	}
	if st, err := os.Stat(filepath.Join(want, "go-build")); err != nil || !st.IsDir() {
		t.Fatalf("go-build subdir not created: %v", err)
	}
	if _, ok := sharedBuildCacheDir(""); ok {
		t.Fatal("sharedBuildCacheDir reported ok for an empty home")
	}
	// A home that is a regular file cannot gain a cache dir.
	fileHome := filepath.Join(t.TempDir(), "file")
	if err := os.WriteFile(fileHome, []byte("x"), 0o644); err != nil {
		t.Fatal(err)
	}
	if _, ok := sharedBuildCacheDir(fileHome); ok {
		t.Fatal("sharedBuildCacheDir reported ok for a file home")
	}
}

// TestBuildEnvCachePin: the build env always pins GOCACHE and
// XDG_CACHE_HOME explicitly (never the caller's values), at the shared
// cache when the real home is usable and inside the build dir only as
// the fallback; HOME is the passwd home, never the planted one.
func TestBuildEnvCachePin(t *testing.T) {
	t.Setenv("GOCACHE", "/planted/gocache")
	t.Setenv("XDG_CACHE_HOME", "/planted/xdg")
	t.Setenv("HOME", "/planted/home")
	t.Setenv("HTTPS_PROXY", "http://planted.invalid")

	dir := t.TempDir()
	env := buildEnv(dir)

	gocache, ok := envValue(env, "GOCACHE")
	if !ok || gocache == "" {
		t.Fatal("buildEnv did not pin GOCACHE")
	}
	if strings.HasPrefix(gocache, "/planted") {
		t.Fatalf("buildEnv kept the planted GOCACHE: %q", gocache)
	}
	xdg, ok := envValue(env, "XDG_CACHE_HOME")
	if !ok || filepath.Join(xdg, "go-build") != gocache {
		t.Fatalf("XDG_CACHE_HOME %q is not the GOCACHE parent of %q", xdg, gocache)
	}
	// The pin is either the shared cache or the per-dir fallback; both
	// are chosen by testsupport, never by the caller.
	shared, sharedOK := sharedBuildCacheDir(homeDirForTest(t))
	switch {
	case sharedOK && xdg == shared:
	case xdg == filepath.Join(dir, ".cache"):
	default:
		t.Fatalf("cache root %q is neither the shared cache %q nor the per-dir fallback", xdg, shared)
	}
	home, ok := envValue(env, "HOME")
	if !ok || home == "/planted/home" {
		t.Fatalf("buildEnv HOME = %q, want the passwd home", home)
	}
	if _, ok := envValue(env, "HTTPS_PROXY"); ok {
		t.Fatal("buildEnv leaked the planted proxy")
	}
}

func homeDirForTest(t *testing.T) string {
	t.Helper()
	// Mirror buildEnv's home resolution: the passwd entry first (the
	// planted HOME env must not win), os.UserHomeDir only as fallback.
	if u, err := user.Current(); err == nil && u.HomeDir != "" {
		return u.HomeDir
	}
	home, err := os.UserHomeDir()
	if err != nil {
		t.Fatal(err)
	}
	return home
}

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

func TestChildEnvCarriesCoverDirOnceEstablished(t *testing.T) {
	prev := mcpxCoverDir
	t.Cleanup(func() { mcpxCoverDir = prev })

	mcpxCoverDir = ""
	if _, ok := envValue(ChildEnv(nil, t.TempDir()), "GOCOVERDIR"); ok {
		t.Fatal("GOCOVERDIR present before the cover dir is established")
	}

	mcpxCoverDir = filepath.Join(t.TempDir(), "cover")
	got, ok := envValue(ChildEnv(nil, t.TempDir()), "GOCOVERDIR")
	if !ok || got != mcpxCoverDir {
		t.Fatalf("GOCOVERDIR = %q, %v; want %q", got, ok, mcpxCoverDir)
	}

	// A test's own GOCOVERDIR in extra still wins: os/exec keeps the
	// last value for a duplicated key, and extra is appended last.
	last := ""
	for _, kv := range ChildEnv(nil, t.TempDir(), "GOCOVERDIR=/elsewhere") {
		if k, v, ok := strings.Cut(kv, "="); ok && k == "GOCOVERDIR" {
			last = v
		}
	}
	if last != "/elsewhere" {
		t.Fatalf("extra GOCOVERDIR did not win: last = %q", last)
	}
}

func TestMCPXBinaryEmitsCoverage(t *testing.T) {
	if os.Getenv(mcpxBinaryOverrideEnv) != "" {
		t.Skip("override binary in use; whether it is a cover build is the packager's choice")
	}
	bin := MCPXBinary(t)
	if MCPXCoverDir() == "" {
		t.Fatal("MCPXBinary did not establish a cover dir")
	}
	dir := t.TempDir()
	cmd := exec.Command(bin, "--help")
	cmd.Env = append(os.Environ(), "GOCOVERDIR="+dir)
	if out, err := cmd.CombinedOutput(); err != nil {
		t.Fatalf("run mcpx --help: %v\n%s", err, out)
	}
	entries, err := os.ReadDir(dir)
	if err != nil {
		t.Fatal(err)
	}
	for _, e := range entries {
		if strings.HasPrefix(e.Name(), "covmeta.") || strings.HasPrefix(e.Name(), "covcounters.") {
			return
		}
	}
	t.Fatalf("no coverage files written to GOCOVERDIR by the suite binary; dir holds %v", entries)
}
