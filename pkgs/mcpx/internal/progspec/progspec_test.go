package progspec_test

import (
	"context"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"github.com/dezren39/mcpx/internal/progspec"
)

func TestParse(t *testing.T) {
	for _, tc := range []struct {
		raw     string
		nix     bool
		ref     string
		attr    string
		wantErr bool
	}{
		{"npx", false, "", "", false},
		{"./tool", false, "", "", false},
		{"/abs/tool", false, "", "", false},
		{"nix:github:owner/repo#tool", true, "github:owner/repo", "tool", false},
		{"nix:github:owner/repo/v1.0#python3Packages.thing", true, "github:owner/repo/v1.0", "python3Packages.thing", false},
		{"nix:nixpkgs#hello", true, "nixpkgs", "hello", false},
		{"nix:.", true, ".", "", false},
		{"nix:github:owner/repo", true, "github:owner/repo", "", false},
		{"", false, "", "", true},
		{"nix:", false, "", "", true},
		{"nix:#tool", false, "", "", true},
		{"nix:nixpkgs#", false, "", "", true},
		{"nix:   ", false, "", "", true},
		{"nix:nixpkgs#  ", false, "", "", true},
	} {
		s, err := progspec.Parse(tc.raw)
		if tc.wantErr {
			if err == nil {
				t.Errorf("Parse(%q): expected error, got %+v", tc.raw, s)
			}
			continue
		}
		if err != nil {
			t.Errorf("Parse(%q): %v", tc.raw, err)
			continue
		}
		if s.Nix != tc.nix || s.Ref != tc.ref || s.Attr != tc.attr {
			t.Errorf("Parse(%q) = %+v, want nix=%v ref=%q attr=%q", tc.raw, s, tc.nix, tc.ref, tc.attr)
		}
	}
}

// stubNix installs a fake nix binary on PATH. Its `build` creates a fake
// store output with one executable and prints the output path, unless the
// installable names "bad" (fails like an unresolvable ref) or "slow"
// (sleeps past any test deadline). Every invocation is counted in a file
// so tests can prove the memo works.
func stubNix(t *testing.T) (storeOut string) {
	t.Helper()
	dir := t.TempDir()
	storeOut = filepath.Join(dir, "store", "aaaa-fakepkg-1.0")
	bin := filepath.Join(storeOut, "bin")
	if err := os.MkdirAll(bin, 0o755); err != nil {
		t.Fatal(err)
	}
	tool := filepath.Join(bin, "faketool")
	if err := os.WriteFile(tool, []byte("#!/bin/sh\nexit 0\n"), 0o755); err != nil {
		t.Fatal(err)
	}
	stub := "#!/bin/sh\n" +
		"echo x >> \"" + filepath.Join(dir, "calls") + "\"\n" +
		"mkdir -p \"" + bin + "\"\n" +
		"printf '#!/bin/sh\nexit 0\n' > \"" + tool + "\"\n" +
		"chmod +x \"" + tool + "\"\n" +
		"for a in \"$@\"; do\n" +
		"  case \"$a\" in *bad*) echo 'error: flake bad does not exist' >&2; exit 1;; esac\n" +
		"  case \"$a\" in *slow*) sleep 30;; esac\n" +
		"done\n" +
		"echo \"" + storeOut + "\"\n"
	if err := os.WriteFile(filepath.Join(dir, "nix"), []byte(stub), 0o755); err != nil {
		t.Fatal(err)
	}
	t.Setenv("PATH", dir+string(os.PathListSeparator)+os.Getenv("PATH"))
	return storeOut
}

func TestResolveNixSpecBuildsOnceAndSpawnsTheStorePath(t *testing.T) {
	out := stubNix(t)
	ctx := context.Background()
	r, err := progspec.Resolve(ctx, "nix:nixpkgs#faketool")
	if err != nil {
		t.Fatal(err)
	}
	if want := filepath.Join(out, "bin", "faketool"); r.Path != want {
		t.Errorf("Path = %q, want the store path %q", r.Path, want)
	}
	if r.StorePath != out || !r.Nix {
		t.Errorf("Resolved = %+v, want StorePath %q and Nix", r, out)
	}
	// Second resolution of the same spec is served from the memo: still
	// exactly one stub invocation on disk.
	if _, err := progspec.Resolve(ctx, "nix:nixpkgs#faketool"); err != nil {
		t.Fatal(err)
	}
	calls, err := os.ReadFile(filepath.Join(filepath.Dir(filepath.Dir(out)), "calls"))
	if err != nil {
		t.Fatal(err)
	}
	if n := len(strings.Fields(string(calls))); n != 1 {
		t.Errorf("nix ran %d times for two resolutions of one spec, want 1 (in-process memo)", n)
	}
}

// A memoised path whose store output has since been deleted (GC, or a
// previous test invocation's temp dir under -count=N) must not be handed
// back: the resolver evicts it and builds again.
func TestResolveNixSpecMemoEvictsDeletedOutput(t *testing.T) {
	out := stubNix(t)
	ctx := context.Background()
	r, err := progspec.Resolve(ctx, "nix:nixpkgs#faketool")
	if err != nil {
		t.Fatal(err)
	}
	if err := os.RemoveAll(out); err != nil {
		t.Fatal(err)
	}
	r2, err := progspec.Resolve(ctx, "nix:nixpkgs#faketool")
	if err != nil {
		t.Fatalf("second Resolve after output deletion: %v", err)
	}
	if r2.Path != r.Path {
		t.Errorf("second Resolve Path = %q, want %q (stub recreates the same output)", r2.Path, r.Path)
	}
	calls, err := os.ReadFile(filepath.Join(filepath.Dir(filepath.Dir(out)), "calls"))
	if err != nil {
		t.Fatal(err)
	}
	if n := len(strings.Fields(string(calls))); n != 2 {
		t.Errorf("nix ran %d times, want 2 (memo evicted, rebuilt)", n)
	}
}

func TestResolveNixSpecWithoutNixBinaryNamesTheSpec(t *testing.T) {
	t.Setenv("PATH", t.TempDir()) // no nix here
	_, err := progspec.Resolve(context.Background(), "nix:nixpkgs#hello")
	if err == nil || !strings.Contains(err.Error(), "nix:nixpkgs#hello") {
		t.Fatalf("err = %v, want an error naming the spec", err)
	}
}

func TestResolveNixSpecBadRefNamesTheSpec(t *testing.T) {
	stubNix(t)
	_, err := progspec.Resolve(context.Background(), "nix:badref#tool")
	if err == nil || !strings.Contains(err.Error(), "nix:badref#tool") {
		t.Fatalf("err = %v, want an error naming the spec", err)
	}
}

func TestResolveNixSpecTimeoutNamesTheSpec(t *testing.T) {
	stubNix(t)
	ctx, cancel := context.WithTimeout(context.Background(), 300*time.Millisecond)
	defer cancel()
	_, err := progspec.Resolve(ctx, "nix:slowref#tool")
	if err == nil || !strings.Contains(err.Error(), "nix:slowref#tool") {
		t.Fatalf("err = %v, want a timeout error naming the spec", err)
	}
}

func TestResolvePlainSpec(t *testing.T) {
	p, err := progspec.ResolvePath(context.Background(), "sh")
	if err != nil || !strings.HasSuffix(p, "/sh") {
		t.Errorf("ResolvePath(sh) = %q, %v", p, err)
	}
	if p, err := progspec.ResolvePath(context.Background(), "./relative/tool"); err != nil || p != "./relative/tool" {
		t.Errorf("ResolvePath(./relative/tool) = %q, %v; a path is used as-is", p, err)
	}
	_, err = progspec.ResolvePath(context.Background(), "mcpx-no-such-program-3199")
	if err == nil || !strings.Contains(err.Error(), "mcpx-no-such-program-3199") {
		t.Errorf("err = %v, want an error naming the missing program", err)
	}
}

func TestResolveNixSpecInDirBuildsThereAndMemoisesPerDir(t *testing.T) {
	out := stubNix(t)
	dirA := t.TempDir()
	dirB := t.TempDir()
	ctx := context.Background()
	// The stub records its working directory instead of the usual call
	// count when MCPX_STUB_PWD is set; here the plain calls file still
	// counts invocations, which is what the per-dir memo is judged by.
	if _, err := progspec.ResolveInDir(ctx, "nix:.#faketool", dirA); err != nil {
		t.Fatal(err)
	}
	if _, err := progspec.ResolveInDir(ctx, "nix:.#faketool", dirA); err != nil {
		t.Fatal(err)
	}
	if _, err := progspec.ResolveInDir(ctx, "nix:.#faketool", dirB); err != nil {
		t.Fatal(err)
	}
	calls, err := os.ReadFile(filepath.Join(filepath.Dir(filepath.Dir(out)), "calls"))
	if err != nil {
		t.Fatal(err)
	}
	if n := len(strings.Fields(string(calls))); n != 2 {
		t.Errorf("nix ran %d times for one spec in two directories (repeat in dir A memoised), want 2", n)
	}
}
