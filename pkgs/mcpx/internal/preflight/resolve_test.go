package preflight_test

import (
	"context"
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/developing-today/code/pkgs/mcpx/internal/preflight"
)

func TestResolveGraphPinningPrecedenceAndAnnouncesLatest(t *testing.T) {
	manifest := preflight.PackageManifest{Path: "/project/package.json", Dir: "/project", Packages: map[string]string{"explicit": "1.0.0", "from-manifest": "2.3.4"}}
	g := &preflight.Graph{
		Roots: []string{"/project/main.ts"},
		Missing: []preflight.MissingSpecifier{
			{Specifier: "explicit/subpath", From: "/project/main.ts"},
			{Specifier: "from-manifest", From: "/project/main.ts"},
			{Specifier: "latest-pkg/subpath", From: "/project/main.ts"},
		},
	}
	set, report := preflight.ResolveGraph(context.Background(), g, preflight.ResolveOptions{
		Explicit: []string{"explicit@9.8.7"}, Manifests: []preflight.PackageManifest{manifest}, Network: true,
		LatestVersion: func(_ context.Context, name string) (string, error) {
			if name != "latest-pkg" {
				t.Fatalf("looked up unexpected package %q", name)
			}
			return "4.5.6", nil
		},
	}, nil, preflight.DynamicWarn)
	if len(set.Packages) != 3 {
		t.Fatalf("resolved set = %+v", set)
	}
	want := map[string]string{"explicit": "9.8.7", "from-manifest": "2.3.4", "latest-pkg": "4.5.6"}
	for _, got := range set.Packages {
		if want[got.Name] != got.Version {
			t.Errorf("%s pin = %q, want %q", got.Name, got.Version, want[got.Name])
		}
	}
	var warningText strings.Builder
	for _, warning := range report.Warnings() {
		warningText.WriteString(warning.What)
		warningText.WriteByte('\n')
	}
	if !strings.Contains(warningText.String(), "using latest latest-pkg@4.5.6") {
		t.Errorf("latest pin must be announced: %+v", report.Warnings())
	}
}

func TestResolveGraphKeepsCrossProjectManifestSubsetSeparate(t *testing.T) {
	root := preflight.PackageManifest{Path: "/root/package.json", Dir: "/root", Packages: map[string]string{"root-only": "1"}}
	foreign := preflight.PackageManifest{Path: "/foreign/package.json", Dir: "/foreign", Packages: map[string]string{"foreign-only": "2"}}
	g := &preflight.Graph{
		Roots:   []string{"/root/main.ts"},
		Edges:   []preflight.ImportEdge{{From: "/root/main.ts", Specifier: "../foreign/helper.ts", Resolved: "/foreign/helper.ts"}},
		Missing: []preflight.MissingSpecifier{{Specifier: "extra-pkg", From: "/foreign/helper.ts"}},
	}
	set, _ := preflight.ResolveGraph(context.Background(), g, preflight.ResolveOptions{
		Explicit: []string{"extra-pkg@3.0.0"}, Manifests: []preflight.PackageManifest{root, foreign}, CrossProject: preflight.CrossProjectIsolate,
	}, nil, preflight.DynamicWarn)
	if len(set.Subsets) != 1 || set.Subsets[0].Manifest.Path != foreign.Path {
		t.Fatalf("cross-project manifest must remain a distinct subset: %+v", set)
	}
	if len(set.Subsets[0].Packages) != 2 {
		t.Fatalf("foreign packages leaked or disappeared: %+v", set.Subsets)
	}
	merged, _ := preflight.ResolveGraph(context.Background(), g, preflight.ResolveOptions{
		Explicit: []string{"extra-pkg@3.0.0"}, Manifests: []preflight.PackageManifest{root, foreign}, CrossProject: preflight.CrossProjectMerge,
	}, nil, preflight.DynamicWarn)
	if len(merged.Subsets) != 0 || len(merged.Packages) != 2 {
		t.Fatalf("merge policy should flatten only by explicit request: %+v", merged)
	}
	ignored, ignoredReport := preflight.ResolveGraph(context.Background(), g, preflight.ResolveOptions{
		Explicit: []string{"extra-pkg@3.0.0"}, Manifests: []preflight.PackageManifest{root, foreign}, CrossProject: preflight.CrossProjectIgnore,
	}, nil, preflight.DynamicWarn)
	if len(ignored.Subsets) != 0 || len(ignored.Packages) != 0 || ignoredReport.Err() == nil || !strings.Contains(ignoredReport.Err().Error(), "importsCrossProject=ignore") {
		t.Fatalf("ignore policy should omit the foreign set: %+v", ignored)
	}
}

func TestResolveGraphMissingPolicyAndNetworkGate(t *testing.T) {
	g := &preflight.Graph{Roots: []string{"/project/main.ts"}, Missing: []preflight.MissingSpecifier{{Specifier: "new-pkg", From: "/project/main.ts"}}}
	lookup := func(context.Context, string) (string, error) { return "1.2.3", nil }
	for _, tc := range []struct {
		mode  preflight.MissingMode
		fatal bool
	}{
		{preflight.MissingFail, true},
		{preflight.MissingWarn, false},
		{preflight.MissingInstall, false},
	} {
		_, report := preflight.ResolveGraph(context.Background(), g, preflight.ResolveOptions{Missing: tc.mode, Network: true, LatestVersion: lookup}, nil, preflight.DynamicWarn)
		if (report.Err() != nil) != tc.fatal {
			t.Errorf("missing policy %q fatal=%t, want %t: %+v", tc.mode, report.Err() != nil, tc.fatal, report.Problems)
		}
		if len(report.Problems) < 2 {
			t.Errorf("policy %q should report both announced latest and missing disposition: %+v", tc.mode, report.Problems)
		}
	}
	set, report := preflight.ResolveGraph(context.Background(), g, preflight.ResolveOptions{Network: false, LatestVersion: lookup}, nil, preflight.DynamicWarn)
	if len(set.Packages) != 0 || report.Err() == nil || !strings.Contains(report.Err().Error(), "script.importsNetwork is off") {
		t.Errorf("network off must not silently select latest: set=%+v report=%+v", set, report.Problems)
	}
}

func TestDiscoverPackageManifestsReadsProjectBoundaries(t *testing.T) {
	root := t.TempDir()
	project := filepath.Join(root, "project")
	foreign := filepath.Join(root, "foreign")
	if err := os.MkdirAll(project, 0o755); err != nil {
		t.Fatal(err)
	}
	if err := os.MkdirAll(foreign, 0o755); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(project, "package.json"), []byte(`{"dependencies":{"root-pkg":"1.0.0"}}`), 0o644); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(foreign, "deno.json"), []byte(`{"imports":{"foreign-pkg":"npm:foreign-pkg@2.0.0"}}`), 0o644); err != nil {
		t.Fatal(err)
	}
	set, err := preflight.DiscoverPackageManifests(&preflight.Graph{Roots: []string{filepath.Join(project, "main.ts")}, LocalFiles: []string{filepath.Join(foreign, "helper.ts")}})
	if err != nil {
		t.Fatal(err)
	}
	if len(set) != 2 {
		t.Fatalf("manifests = %+v", set)
	}
	got := map[string]map[string]string{}
	for _, m := range set {
		got[filepath.Base(m.Dir)] = m.Packages
	}
	if got["project"]["root-pkg"] != "1.0.0" || got["foreign"]["foreign-pkg"] != "2.0.0" {
		t.Fatalf("manifest package pins = %+v", set)
	}
}
