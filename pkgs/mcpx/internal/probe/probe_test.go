package probe_test

import (
	"context"
	"errors"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"github.com/developing-today/code/pkgs/mcpx/internal/probe"
	"github.com/developing-today/code/pkgs/mcpx/internal/testsupport"
)

// writeFixture lays out one probe directory: an ESM package.json, the
// data file the fs probe reads, a node_modules/probe-pkg package for
// the package-resolution probe, and every probe source file.
func writeFixture(t *testing.T) string {
	t.Helper()
	dir := t.TempDir()
	files := map[string]string{
		"package.json": `{"type":"module"}` + "\n",
		"data.txt":     "probe-data\n",
		filepath.Join("node_modules", "probe-pkg", "package.json"): `{"name":"probe-pkg","type":"module","main":"index.js","exports":{".":"./index.js"}}` + "\n",
		filepath.Join("node_modules", "probe-pkg", "index.js"):     `export const greeting = "hello-from-probe-pkg";` + "\n",
		probe.TypeCheckGood: probe.TypeCheckGoodSource + "\n",
		probe.TypeCheckBad:  probe.TypeCheckBadSource + "\n",
	}
	for _, p := range append(append([]probe.Probe{}, probe.RunProbes...), probe.PackageProbe) {
		files[p.File] = p.Source + "\n"
	}
	for name, content := range files {
		path := filepath.Join(dir, name)
		if err := os.MkdirAll(filepath.Dir(path), 0o755); err != nil {
			t.Fatal(err)
		}
		if err := os.WriteFile(path, []byte(content), 0o644); err != nil {
			t.Fatal(err)
		}
	}
	return dir
}

// run executes one probe file under one runtime in dir and returns its
// combined output and exit code.
func run(t *testing.T, bin, runtime, dir, file string) (string, int) {
	t.Helper()
	ctx, cancel := context.WithTimeout(context.Background(), 60*time.Second)
	defer cancel()
	cmd := exec.CommandContext(ctx, bin, probe.RunArgs(runtime, file)...)
	cmd.Dir = dir
	cmd.Env = append(os.Environ(), "MCPX_PROBE_TOKEN=probe-token")
	out, err := cmd.CombinedOutput()
	if err == nil {
		return string(out), 0
	}
	var ee *exec.ExitError
	if errors.As(err, &ee) {
		return string(out), ee.ExitCode()
	}
	t.Fatalf("%s %s: %v\n%s", runtime, file, err, out)
	return string(out), -1
}

// TestRuntimeCapabilityProbes runs the five one-line probes plus the
// package-resolution probe under every runtime and asserts the results
// the docs/runtimes.md matrix publishes.
func TestRuntimeCapabilityProbes(t *testing.T) {
	dir := writeFixture(t)
	for _, rt := range probe.Runtimes {
		rt := rt
		t.Run(rt, func(t *testing.T) {
			bin := testsupport.RequireBinary(t, rt)
			for _, p := range append(append([]probe.Probe{}, probe.RunProbes...), probe.PackageProbe) {
				out, code := run(t, bin, rt, dir, p.File)
				if code != p.WantCode {
					t.Errorf("%s %s: exit %d, want %d\n%s", rt, p.Name, code, p.WantCode, out)
					continue
				}
				if !strings.Contains(out, p.Want) {
					t.Errorf("%s %s: output missing %q\n%s", rt, p.Name, p.Want, out)
				}
			}
		})
	}
}

// TestRuntimeTypeCheckProbes asserts the type-checking row of the
// matrix: deno statically checks (deno check accepts the good file and
// rejects the bad one), while bun and node strip types unchecked, so
// the ill-typed probe still runs under them.
func TestRuntimeTypeCheckProbes(t *testing.T) {
	dir := writeFixture(t)
	for _, rt := range probe.Runtimes {
		rt := rt
		t.Run(rt, func(t *testing.T) {
			bin := testsupport.RequireBinary(t, rt)
			if probe.TypeChecks[rt] {
				for _, tc := range []struct {
					file string
					ok   bool
				}{{probe.TypeCheckGood, true}, {probe.TypeCheckBad, false}} {
					ctx, cancel := context.WithTimeout(context.Background(), 60*time.Second)
					cmd := exec.CommandContext(ctx, bin, "check", tc.file)
					cmd.Dir = dir
					out, err := cmd.CombinedOutput()
					cancel()
					if tc.ok && err != nil {
						t.Errorf("%s check %s: %v\n%s", rt, tc.file, err, out)
					}
					if !tc.ok && err == nil {
						t.Errorf("%s check %s: succeeded, want a type error\n%s", rt, tc.file, out)
					}
				}
				return
			}
			// No static checker: the ill-typed file runs as-is.
			out, code := run(t, bin, rt, dir, probe.TypeCheckBad)
			if code != 0 || !strings.Contains(out, "probe type=not a number") {
				t.Errorf("%s: ill-typed probe should run unchecked (exit %d)\n%s", rt, code, out)
			}
		})
	}
}
