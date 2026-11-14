package runner

import (
	"errors"
	"os"
	"path/filepath"
	"reflect"
	"strings"
	"testing"
)

// The wrap decision is pure: setting value + nix presence -> argv or
// error (task 14245 implementation map, step 5). Oracle: the argv the
// spawn would exec, or an error naming script.devshell.

func TestWrapDevshellHostPassesThrough(t *testing.T) {
	look := func(string) (string, error) { return "", errors.New("must not be called") }
	for _, v := range []string{"", "host"} {
		bin, args, err := wrapDevshell(v, "deno", []string{"run", "x.ts"}, look)
		if err != nil || bin != "deno" || !reflect.DeepEqual(args, []string{"run", "x.ts"}) {
			t.Fatalf("wrapDevshell(%q) = %q %v %v", v, bin, args, err)
		}
	}
}

func TestWrapDevshellFlakeWrapsArgv(t *testing.T) {
	look := func(name string) (string, error) {
		if name != "nix" {
			t.Fatalf("lookPath(%q), want nix", name)
		}
		return "/nix/bin/nix", nil
	}
	bin, args, err := wrapDevshell("flake:./tools", "deno", []string{"run", "x.ts"}, look)
	want := []string{"develop", "./tools", "-c", "deno", "run", "x.ts"}
	if err != nil || bin != "/nix/bin/nix" || !reflect.DeepEqual(args, want) {
		t.Fatalf("wrapDevshell(flake) = %q %v %v, want /nix/bin/nix %v", bin, args, err, want)
	}
}

func TestWrapDevshellFlakeWithoutNixFailsLoudly(t *testing.T) {
	look := func(string) (string, error) { return "", errors.New("not found") }
	_, _, err := wrapDevshell("flake:./tools", "deno", nil, look)
	if err == nil || !strings.Contains(err.Error(), "script.devshell") {
		t.Fatalf("err = %v, want an error naming script.devshell", err)
	}
}

func TestWrapDevshellRejectsUnknownValues(t *testing.T) {
	look := func(string) (string, error) { return "/nix/bin/nix", nil }
	for _, v := range []string{"auto", "shell:bash", "flake:"} {
		if _, _, err := wrapDevshell(v, "deno", nil, look); err == nil {
			t.Fatalf("wrapDevshell(%q) succeeded, want an error", v)
		}
	}
}

// The spawn name is the P1 fix's core: under a flake the shell resolves
// the runtime by name; on the host the resolved path is used verbatim.

func TestDevshellSpawnNameFlakeUsesInvokeName(t *testing.T) {
	if got := devshellSpawnName("flake:./tools", "/usr/bin/deno", "deno"); got != "deno" {
		t.Fatalf("flake spawn name = %q, want the invoke name %q", got, "deno")
	}
	for _, host := range []string{"", "host"} {
		if got := devshellSpawnName(host, "/usr/bin/deno", "deno"); got != "/usr/bin/deno" {
			t.Fatalf("devshell %q spawn name = %q, want the resolved bin", host, got)
		}
	}
	if got := devshellSpawnName("flake:./tools", "/usr/bin/deno", ""); got != "/usr/bin/deno" {
		t.Fatalf("flake spawn name with no invoke = %q, want the resolved bin", got)
	}
}

func TestResolveRecordsInvokeNameBeforePathResolution(t *testing.T) {
	dir := t.TempDir()
	deno := filepath.Join(dir, "deno")
	if err := os.WriteFile(deno, []byte("#!/bin/sh\nexit 0\n"), 0o755); err != nil {
		t.Fatal(err)
	}
	t.Setenv("PATH", dir)
	rt, err := Resolve("deno", "", Setup{})
	if err != nil {
		t.Fatalf("Resolve(deno): %v", err)
	}
	if rt.Bin != deno {
		t.Fatalf("Bin = %q, want the PATH-resolved %q", rt.Bin, deno)
	}
	if rt.Invoke != "deno" {
		t.Fatalf("Invoke = %q, want the unresolved name %q", rt.Invoke, "deno")
	}
}

func TestResolveDeferredCarriesTheBareName(t *testing.T) {
	// No deno anywhere on this PATH: the deferred runtime the devshell
	// case relies on keeps Bin == Invoke == the kind name, for the
	// shell to resolve.
	t.Setenv("PATH", t.TempDir())
	rt, err := ResolveDeferred("deno", "", Setup{})
	if err != nil {
		t.Fatalf("ResolveDeferred(deno): %v", err)
	}
	if rt.Bin != "deno" || rt.Invoke != "deno" {
		t.Fatalf("deferred runtime Bin/Invoke = %q/%q, want deno/deno", rt.Bin, rt.Invoke)
	}
}
