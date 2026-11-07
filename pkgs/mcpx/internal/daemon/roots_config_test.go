package daemon

import (
	"net/url"
	"path/filepath"
	"testing"

	"github.com/developing-today/code/pkgs/mcpx/internal/elicit"
	"github.com/developing-today/code/pkgs/mcpx/internal/events"
	"github.com/developing-today/code/pkgs/mcpx/internal/mcpclient"
	"github.com/developing-today/code/pkgs/mcpx/internal/settings"
)

func TestRootsFromPathsBuildsFileURIs(t *testing.T) {
	path := filepath.Join(t.TempDir(), "work #1")
	abs, err := filepath.Abs(path)
	if err != nil {
		t.Fatal(err)
	}
	want := (&url.URL{Scheme: "file", Path: filepath.ToSlash(abs)}).String()

	got, err := rootsFromPaths([]string{path})
	if err != nil {
		t.Fatal(err)
	}
	if len(got) != 1 || got[0] != (mcpclient.Root{URI: want, Name: filepath.Base(path)}) {
		t.Fatalf("rootsFromPaths = %#v, want one root with URI %q", got, want)
	}
}

func TestInstallHooksOmitsUnanswerableBrokerHandler(t *testing.T) {
	root := mcpclient.Root{URI: "file:///workspace", Name: "workspace"}
	r := &Registry{}
	r.InstallHooks(events.New(0), nil, []mcpclient.Root{root})

	if r.hooks.Elicit != nil {
		t.Fatal("nil broker installed a handler that advertises sampling and URL elicitation")
	}
	if r.hooks.URLElicitation != nil {
		t.Fatal("nil broker installed a URL elicitation handler")
	}
	if len(r.hooks.Roots) != 1 || r.hooks.Roots[0] != root {
		t.Fatalf("configured roots = %#v, want %#v", r.hooks.Roots, []mcpclient.Root{root})
	}
}

func TestInstallHooksKeepsBrokerAnswerHandler(t *testing.T) {
	broker, err := elicit.Open(filepath.Join(t.TempDir(), "elicit.db"))
	if err != nil {
		t.Fatal(err)
	}
	defer broker.Close()

	r := &Registry{}
	r.InstallHooks(events.New(0), broker, nil)
	if r.hooks.Elicit == nil || r.hooks.URLElicitation == nil {
		t.Fatal("broker-backed hooks must keep both answer handlers installed")
	}
}

func TestServerRootsComeFromDaemonSettings(t *testing.T) {
	path := filepath.Join(t.TempDir(), "configured")
	sch, err := settings.New(settings.Registry())
	if err != nil {
		t.Fatal(err)
	}
	set := settings.NewSet(sch).WithOverrides(map[string]string{"daemon.roots": path}, "test")
	roots, err := rootsFromPaths(set.List("daemon.roots"))
	if err != nil {
		t.Fatal(err)
	}
	if len(roots) != 1 || roots[0].Name != "configured" || roots[0].URI == "" {
		t.Fatalf("configured roots = %#v", roots)
	}
}
