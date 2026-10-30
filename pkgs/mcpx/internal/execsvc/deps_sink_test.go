package execsvc_test

import (
	"context"
	"os"
	"path/filepath"
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
	needRuntime(t)
	work := t.TempDir()
	var frames []execsvc.Frame
	_, err := svc.RunWith(context.Background(), runner.Options{
		Source:       `console.log("hi");`,
		ClientSource: minimalClient,
		Permissions:  "all",
		WorkDir:      work,
	}, execsvc.Options{
		Runtime: "deno",
		Deps: &execsvc.DepsOptions{
			Packages:    []string{"zod@3.0.0"},
			AutoInstall: "off",
			Lockfiles: []execsvc.DepsLockfile{
				{Name: "bun.lock", Content: []byte("bun")},
				{Name: "deno.lock", Content: []byte("{}")},
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
