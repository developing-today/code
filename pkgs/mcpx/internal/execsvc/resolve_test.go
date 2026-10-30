package execsvc

import (
	"context"
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/dezren39/mcpx/internal/scriptresolve"
	"github.com/dezren39/mcpx/internal/testsupport"
)

// Daemon-side resolution (#2168): /v1/exec's file field goes through
// Service.Run, so these tests exercise the same entry the daemon calls —
// a bare name resolves on the search path, and the CLI's resolver
// (scriptresolve, rooted the same way) agrees on the same fixture.

func writeExecScript(t *testing.T, dir, name string) string {
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

func TestResolveFileFindsNamedScriptOnSearchPath(t *testing.T) {
	root := testsupport.TempDir(t)
	want := writeExecScript(t, filepath.Join(root, scriptresolve.DirName), "report.ts")
	t.Setenv("HOME", filepath.Join(root, "home-empty"))
	t.Setenv("XDG_CONFIG_HOME", filepath.Join(root, "xdg"))

	svc := &Service{}
	req := Request{File: "report", Opts: Options{Cwd: root}}
	got, err := svc.resolveFile(req)
	if err != nil {
		t.Fatal(err)
	}
	if got != want {
		t.Fatalf("daemon resolution: got %q, want %q", got, want)
	}

	// The CLI resolves the same name through the same package and must
	// land on the same file — one rule, both surfaces.
	cliGot, err := scriptresolve.Resolver{Dir: root}.Resolve("report")
	if err != nil {
		t.Fatal(err)
	}
	if cliGot != got {
		t.Fatalf("CLI and daemon disagree: cli %q, daemon %q", cliGot, got)
	}
}

func TestResolveFileMissNamesEveryDirectorySearched(t *testing.T) {
	root := testsupport.TempDir(t)
	t.Setenv("HOME", filepath.Join(root, "home-empty"))
	t.Setenv("XDG_CONFIG_HOME", filepath.Join(root, "xdg"))

	svc := &Service{}
	_, err := svc.resolveFile(Request{File: "nope", Opts: Options{Cwd: root}})
	if err == nil {
		t.Fatal("expected an error")
	}
	if !strings.Contains(err.Error(), filepath.Join(root, scriptresolve.DirName)) {
		t.Errorf("error should list the project scripts dir:\n%s", err)
	}
}

func TestRunRejectsUnresolvableNameBeforeRunning(t *testing.T) {
	root := testsupport.TempDir(t)
	t.Setenv("HOME", filepath.Join(root, "home-empty"))
	t.Setenv("XDG_CONFIG_HOME", filepath.Join(root, "xdg"))

	svc := &Service{}
	_, err := svc.Run(context.Background(),
		Request{File: "nope", Opts: Options{Cwd: root}}, nil)
	if err == nil {
		t.Fatal("expected the resolution error, not a run")
	}
	if !strings.Contains(err.Error(), "no script named") {
		t.Errorf("expected the shared resolution error, got: %v", err)
	}
}

func TestResolveFilePassesPathShapedFilesThrough(t *testing.T) {
	svc := &Service{}
	for _, f := range []string{"./a.ts", "/abs/b.ts", "sub/c.ts"} {
		got, err := svc.resolveFile(Request{File: f})
		if err != nil {
			t.Fatalf("%q: %v", f, err)
		}
		if got != f {
			t.Errorf("%q was rewritten to %q", f, got)
		}
	}
}
