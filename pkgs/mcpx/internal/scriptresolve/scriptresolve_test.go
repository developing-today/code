package scriptresolve

import (
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/dezren39/mcpx/internal/settings"
	"github.com/dezren39/mcpx/internal/testsupport"
)

// These tests mirror internal/cli's resolveScript tests: the CLI and the
// daemon resolve through this package, so the fixtures here are the shared
// contract both surfaces are held to (#2168).

func writeScript(t *testing.T, dir, name string) string {
	t.Helper()
	if err := os.MkdirAll(dir, 0o755); err != nil {
		t.Fatal(err)
	}
	p := filepath.Join(dir, name)
	if err := os.WriteFile(p, []byte("// s\n"), 0o644); err != nil {
		t.Fatal(err)
	}
	return p
}

// isolate points HOME and XDG at empty fixtures under root, so the real
// user's scripts never leak into a test's search path.
func isolate(t *testing.T, root string) {
	t.Helper()
	t.Setenv("HOME", filepath.Join(root, "home-empty"))
	t.Setenv("XDG_CONFIG_HOME", filepath.Join(root, "xdg"))
}

func TestPathShapedArgumentsAreUsedVerbatim(t *testing.T) {
	r := Resolver{Dir: t.TempDir()}
	for _, arg := range []string{"./a.ts", "/abs/b.ts", "sub/c.ts", "d.ts", "e.js"} {
		got, err := r.Resolve(arg)
		if err != nil {
			t.Fatalf("%q: %v", arg, err)
		}
		if got != arg {
			t.Errorf("%q was rewritten to %q; path-shaped arguments must pass through", arg, got)
		}
	}
}

func TestBareNameResolvesFromProjectScripts(t *testing.T) {
	root := testsupport.TempDir(t)
	want := writeScript(t, filepath.Join(root, DirName), "report.ts")
	isolate(t, root)

	got, err := Resolver{Dir: root}.Resolve("report")
	if err != nil {
		t.Fatal(err)
	}
	if got != want {
		t.Fatalf("got %q, want %q", got, want)
	}
}

func TestBareNameFoundInAParentDirectory(t *testing.T) {
	root := testsupport.TempDir(t)
	want := writeScript(t, filepath.Join(root, DirName), "shared.ts")
	deep := filepath.Join(root, "a", "b", "c")
	if err := os.MkdirAll(deep, 0o755); err != nil {
		t.Fatal(err)
	}
	isolate(t, root)

	got, err := Resolver{Dir: deep}.Resolve("shared")
	if err != nil {
		t.Fatalf("a script in an ancestor scripts dir should be found: %v", err)
	}
	if got != want {
		t.Fatalf("got %q, want %q", got, want)
	}
}

func TestNearerScriptShadowsFurtherOne(t *testing.T) {
	root := testsupport.TempDir(t)
	writeScript(t, filepath.Join(root, DirName), "dup.ts")
	inner := filepath.Join(root, "inner")
	want := writeScript(t, filepath.Join(inner, DirName), "dup.ts")
	isolate(t, root)

	got, err := Resolver{Dir: inner}.Resolve("dup")
	if err != nil {
		t.Fatal(err)
	}
	if got != want {
		t.Fatalf("nearest scripts dir must win: got %q, want %q", got, want)
	}
}

func TestExtensionFallbackOrder(t *testing.T) {
	root := testsupport.TempDir(t)
	want := writeScript(t, filepath.Join(root, DirName), "onlyjs.js")
	isolate(t, root)

	got, err := Resolver{Dir: root}.Resolve("onlyjs")
	if err != nil {
		t.Fatal(err)
	}
	if got != want {
		t.Fatalf("got %q, want %q", got, want)
	}
}

func TestMissingScriptErrorNamesEveryDirectorySearched(t *testing.T) {
	root := testsupport.TempDir(t)
	extra := filepath.Join(root, "extra")
	isolate(t, root)

	r := Resolver{Dir: root, Configured: []string{extra, settings.NullMarker}}
	_, err := r.Resolve("nope")
	if err == nil {
		t.Fatal("expected an error")
	}
	msg := err.Error()
	for _, want := range []string{"nope", "looked in", extra, filepath.Join(root, DirName)} {
		if !strings.Contains(msg, want) {
			t.Errorf("error should mention %q:\n%s", want, msg)
		}
	}
}

func TestConfiguredPathWithNullSplice(t *testing.T) {
	root := testsupport.TempDir(t)
	extra := filepath.Join(root, "extra")
	wantExtra := writeScript(t, extra, "fromextra.ts")
	wantBuiltin := writeScript(t, filepath.Join(root, DirName), "frombuiltin.ts")
	isolate(t, root)

	r := Resolver{Dir: root, Configured: []string{extra, settings.NullMarker}}
	for name, want := range map[string]string{"fromextra": wantExtra, "frombuiltin": wantBuiltin} {
		got, err := r.Resolve(name)
		if err != nil {
			t.Fatalf("%s: %v", name, err)
		}
		if got != want {
			t.Errorf("%s: got %q, want %q", name, got, want)
		}
	}
}

func TestSplitPathListForms(t *testing.T) {
	if got := SplitPathList(""); got != nil {
		t.Errorf("empty: got %v", got)
	}
	got := SplitPathList(`["/a", null, "/b"]`)
	if len(got) != 3 || got[0] != "/a" || got[1] != settings.NullMarker || got[2] != "/b" {
		t.Errorf("json form: got %v", got)
	}
	got = SplitPathList("/a" + string(os.PathListSeparator) + "-")
	if len(got) != 2 || got[0] != "/a" || got[1] != settings.NullMarker {
		t.Errorf("separator form: got %v", got)
	}
}
