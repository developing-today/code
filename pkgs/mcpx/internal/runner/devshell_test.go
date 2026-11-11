package runner

import (
	"errors"
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
