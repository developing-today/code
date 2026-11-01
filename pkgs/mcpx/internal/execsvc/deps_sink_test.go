package execsvc_test

import (
	"context"
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/dezren39/mcpx/internal/execsvc"
	"github.com/dezren39/mcpx/internal/runner"
)

// TestDepsSurviveANonNilSink is the P1 Codex finding on PR #2589:
// RunWith converted the wire dependency set into the runner's only
// inside `if sink == nil`, so every caller that supplies a sink (the
// streaming CLI path, the daemon's execStream handler) silently ran
// without its requested packages. The observable end of the
// conversion is the materialised set in the run's workdir.
func TestDepsSurviveANonNilSink(t *testing.T) {
	svc := service(t, store(t))
	if _, err := runner.Resolve("deno", "", runner.Setup{}); err != nil {
		t.Skip("no deno runtime:", err)
	}
	work := t.TempDir()
	var frames []execsvc.Frame
	_, err := svc.RunWith(context.Background(), runner.Options{
		Source:       `console.log("hi");`,
		ClientSource: minimalClient,
		Permissions:  "all",
		WorkDir:      work,
		Runtime:      "deno",
	}, execsvc.Options{
		Deps: &execsvc.DepsOptions{
			Packages:    []string{"zod@3.0.0"},
			AutoInstall: "off",
			Lockfiles: []execsvc.DepsLockfile{
				{Name: "bun.lock", Content: []byte("bun")},
				{Name: "deno.lock", Content: []byte(`{"version":"4","specifiers":{},"npm":{}}`)},
			},
		},
	}, func(f execsvc.Frame) error {
		frames = append(frames, f)
		return nil
	})
	if err != nil {
		t.Fatalf("run: %v", err)
	}
	if len(frames) == 0 {
		t.Fatal("the sink received no frames; the test did not exercise the sink path")
	}
	if _, err := os.Stat(filepath.Join(work, "deno.json")); err != nil {
		t.Fatalf("deps were dropped with a non-nil sink: no deno.json materialised: %v", err)
	}
	if _, err := os.Stat(filepath.Join(work, "deno.lock")); err != nil {
		t.Fatalf("the runtime-selected lockfile (deno.lock) was not materialised: %v", err)
	}
	if _, err := os.Stat(filepath.Join(work, "bun.lock")); !os.IsNotExist(err) {
		t.Fatal("bun.lock materialised for a deno run")
	}
}

// TestRunWithRejectsPinsThatContradictTheManifest is the CodeRabbit
// finding on PR #2589 (runner/deps.go): a /v1/exec request could send
// deps.packages contradicting deps.manifest and RunWith forwarded the
// set with no conflict check, so the manifest version silently won.
// The boundary every caller crosses rejects the pin naming both
// versions, as the CLI already does.
func TestRunWithRejectsPinsThatContradictTheManifest(t *testing.T) {
	svc := service(t, store(t))
	_, err := svc.RunWith(context.Background(), runner.Options{
		Source:       `console.log("hi");`,
		ClientSource: minimalClient,
		Permissions:  "all",
		WorkDir:      t.TempDir(),
	}, execsvc.Options{
		Runtime: "deno",
		Deps: &execsvc.DepsOptions{
			Packages: []string{"foo@2.0.0"},
			Manifest: `{"dependencies":{"foo":"1.0.0"}}`,
		},
	}, nil)
	if err == nil || !strings.Contains(err.Error(), "foo: explicit 2.0.0 contradicts manifest 1.0.0") {
		t.Fatalf("a contradicting pin must be rejected naming both versions, got %v", err)
	}
}

func TestRunWithManifestConflictSectionPrecedence(t *testing.T) {
	for _, tc := range []struct {
		name, manifest, want string
	}{
		{"dev", `{"devDependencies":{"foo":"1.0.0"}}`, "1.0.0"},
		{"production overrides dev", `{"devDependencies":{"foo":"1.0.0"},"dependencies":{"foo":"1.1.0"}}`, "1.1.0"},
		{"optional overrides both", `{"devDependencies":{"foo":"1.0.0"},"dependencies":{"foo":"1.1.0"},"optionalDependencies":{"foo":"1.2.0"}}`, "1.2.0"},
	} {
		t.Run(tc.name, func(t *testing.T) {
			svc := service(t, store(t))
			_, err := svc.RunWith(context.Background(), runner.Options{}, execsvc.Options{
				Deps: &execsvc.DepsOptions{Packages: []string{"foo@2.0.0"}, Manifest: tc.manifest},
			}, nil)
			want := "foo: explicit 2.0.0 contradicts manifest " + tc.want
			if err == nil || !strings.Contains(err.Error(), want) {
				t.Fatalf("expected %q, got %v", want, err)
			}
		})
	}
}
