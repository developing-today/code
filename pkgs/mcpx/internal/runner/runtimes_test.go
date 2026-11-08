package runner

import (
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"testing"
)

// #263: node was invoked with --experimental-strip-types, whose strip-only
// mode rejects non-erasable TypeScript (enum, value namespaces) that Node
// runs fine under --experimental-transform-types. On machines without deno
// or bun, auto-resolution lands on node, so every such script failed with a
// raw SyntaxError that looked like the author's mistake.
func TestNodeArgsUseTransformTypesNotStripTypes(t *testing.T) {
	fakeRuntimes(t, "node")
	got, err := Resolve("node", "all", Setup{})
	if err != nil {
		t.Fatal(err)
	}
	args := got.Args("s.ts")
	joined := strings.Join(args, " ")
	if !strings.Contains(joined, "--experimental-transform-types") {
		t.Errorf("node argv %q does not contain --experimental-transform-types", args)
	}
	if strings.Contains(joined, "--experimental-strip-types") {
		t.Errorf("node argv %q still contains --experimental-strip-types (#263)", args)
	}
}

// End-to-end guard for #263: a non-erasable TypeScript file must actually
// run under the resolved node runtime. Skips when no suitable node is on
// PATH (transform-types needs Node >= 22.6; older nodes reject the flag
// with "bad option").
func TestNodeRunsNonErasableTypeScript(t *testing.T) {
	if _, err := exec.LookPath("node"); err != nil {
		t.Skip("no node on PATH")
	}
	got, err := Resolve("node", "all", Setup{})
	if err != nil {
		t.Fatal(err)
	}
	dir := t.TempDir()
	script := filepath.Join(dir, "en.ts")
	src := "enum E { A }\nconsole.log(\"enum ok\", E.A);\n"
	if err := os.WriteFile(script, []byte(src), 0o644); err != nil {
		t.Fatal(err)
	}
	out, err := exec.Command(got.Bin, got.Args(script)...).CombinedOutput()
	if err != nil {
		if strings.Contains(string(out), "bad option") {
			t.Skipf("this node does not support --experimental-transform-types: %s", out)
		}
		t.Fatalf("node failed on enum script (#263): %v\n%s", err, out)
	}
	if !strings.Contains(string(out), "enum ok 0") {
		t.Errorf("unexpected output %q, want %q", out, "enum ok 0")
	}
}

// #1911 collision 11: what `auto` picks was asserted nowhere at the
// resolution level -- the issue's snapshot noted the only test caller just
// needed *a* runtime. These are the four stub-PATH cases the collision
// decision calls for: a PATH holding only a stub deno, only bun, only
// node, and none, with no real runtime involved. Permissions are "all" so
// no profile refusal can skew the pick; the stub executables from
// fakeRuntimes are never executed, only found by LookPath.
func TestAutoResolvesAgainstAStubPath(t *testing.T) {
	cases := []struct {
		name  string
		stubs []string
		want  string
	}{
		{"deno only", []string{"deno"}, "deno"},
		{"bun only", []string{"bun"}, "bun"},
		{"node only", []string{"node"}, "node"},
	}
	for _, c := range cases {
		t.Run(c.name, func(t *testing.T) {
			fakeRuntimes(t, c.stubs...)
			got, err := Resolve("auto", "all", Setup{})
			if err != nil {
				t.Fatalf("auto with only %s on PATH: %v", c.want, err)
			}
			if got.Name != c.want || got.Kind != c.want {
				t.Errorf("auto with only %s on PATH picked %q (kind %q)", c.want, got.Name, got.Kind)
			}
			if filepath.Base(got.Bin) != c.want {
				t.Errorf("auto picked bin %q, want the stub %s", got.Bin, c.want)
			}
		})
	}

	t.Run("none", func(t *testing.T) {
		fakeRuntimes(t)
		_, err := Resolve("auto", "all", Setup{})
		if err == nil {
			t.Fatal("auto with no runtime on PATH should fail")
		}
		// The refusal names every runtime it tried, in default order.
		for _, want := range []string{"no usable JavaScript runtime", "deno, bun, node"} {
			if !strings.Contains(err.Error(), want) {
				t.Errorf("auto with no runtime: %q does not mention %q", err, want)
			}
		}
	})

	// Default order prefers deno when several are present, and a declared
	// script.runtimeOrder (Setup.Order) overrides it.
	t.Run("order", func(t *testing.T) {
		fakeRuntimes(t, "deno", "bun", "node")
		got, err := Resolve("auto", "all", Setup{})
		if err != nil || got.Name != "deno" {
			t.Errorf("auto default order should pick deno first: %v %v", got, err)
		}
		got, err = Resolve("auto", "all", Setup{Order: []string{"node", "deno"}})
		if err != nil || got.Name != "node" {
			t.Errorf("auto with runtimeOrder node,deno should pick node: %v %v", got, err)
		}
	})
}
