package e2e_test

import (
	"os"
	"path/filepath"
	"strings"
	"testing"
)

// Effect probes for the dependency chain settings (#1913, probes owed
// by PR #2589). The observable surface is the chain's conflict rule: a
// pin that contradicts the manifest a run resolves against fails the
// run, naming both versions, before anything executes. Each probe
// below flips exactly one setting and watches that surface change --
// no probe needs a network install, because resolution fails first.

func depsProbeProject(t *testing.T, e *env, manifest string) string {
	t.Helper()
	proj := filepath.Join(e.dir, "proj")
	if err := os.MkdirAll(proj, 0o755); err != nil {
		t.Fatal(err)
	}
	if manifest != "" {
		if err := os.WriteFile(filepath.Join(proj, "package.json"), []byte(manifest), 0o644); err != nil {
			t.Fatal(err)
		}
	}
	script := filepath.Join(proj, "main.ts")
	src := "export default function () { return \"ran\"; }\n"
	if err := os.WriteFile(script, []byte(src), 0o644); err != nil {
		t.Fatal(err)
	}
	return script
}

func withEnvVars(e *env, kv ...string) func() {
	saved := e.envVars
	e.envVars = append(append([]string{}, saved...), kv...)
	return func() { e.envVars = saved }
}

// deps.packages: naming a pin that contradicts the project manifest
// turns a run that would have executed into a named conflict.
func TestDepsPackagesPinContradictingTheManifestFailsTheRun(t *testing.T) {
	e := newEnv(t, oneServer)
	script := depsProbeProject(t, e, "{\"name\":\"proj\",\"dependencies\":{\"foo\":\"^2.0.0\"}}\n")

	if out := e.run("run", script); !strings.Contains(out, "ran") {
		t.Fatalf("without deps.packages the script should run:\n%s", out)
	}
	defer withEnvVars(e, "MCPX_DEPS_PACKAGES=foo@1.0.0")()
	out, err := e.try("run", script)
	if err == nil || !strings.Contains(out+err.Error(), "contradicts") {
		t.Fatalf("deps.packages=foo@1.0.0 against manifest ^2.0.0 should fail with a conflict, got out=%q err=%v", out, err)
	}
}

// deps.manifest: the named manifest is the one resolution reads -- a
// pin contradicting IT fails the run even though the script's own
// directory declares nothing.
func TestDepsManifestNamesTheManifestResolutionReads(t *testing.T) {
	e := newEnv(t, oneServer)
	script := depsProbeProject(t, e, "")
	alt := filepath.Join(e.dir, "alt-manifest.json")
	if err := os.WriteFile(alt, []byte("{\"name\":\"alt\",\"dependencies\":{\"foo\":\"^2.0.0\"}}\n"), 0o644); err != nil {
		t.Fatal(err)
	}

	defer withEnvVars(e, "MCPX_DEPS_MANIFEST="+alt, "MCPX_DEPS_PACKAGES=foo@1.0.0")()
	out, err := e.try("run", script)
	if err == nil || !strings.Contains(out+err.Error(), "contradicts") {
		t.Fatalf("a pin contradicting the deps.manifest manifest should fail, got out=%q err=%v", out, err)
	}
}

// deps.source=none: the chain resolves to nothing, so the manifest
// layer -- and its conflict with the pin -- disappears and the same
// run that failed under the default source executes.
func TestDepsSourceNoneDropsTheManifestLayer(t *testing.T) {
	e := newEnv(t, oneServer)
	script := depsProbeProject(t, e, "{\"name\":\"proj\",\"dependencies\":{\"foo\":\"^2.0.0\"}}\n")

	defer withEnvVars(e, "MCPX_DEPS_PACKAGES=foo@1.0.0", "MCPX_DEPS_SOURCE=none")()
	if out := e.run("run", script); !strings.Contains(out, "ran") {
		t.Fatalf("deps.source=none should drop the manifest layer and let the run proceed:\n%s", out)
	}
}

// paths.deps: discovery reaches a manifest that lives only on the
// configured search path -- the pin contradicts it there, though the
// script's own tree declares nothing.
func TestPathsDepsExtendsManifestDiscovery(t *testing.T) {
	e := newEnv(t, oneServer)
	script := depsProbeProject(t, e, "")
	lib := filepath.Join(e.dir, "libproj")
	if err := os.MkdirAll(lib, 0o755); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(lib, "package.json"), []byte("{\"name\":\"lib\",\"dependencies\":{\"foo\":\"^2.0.0\"}}\n"), 0o644); err != nil {
		t.Fatal(err)
	}

	defer withEnvVars(e, "MCPX_PATHS_DEPS="+lib, "MCPX_DEPS_PACKAGES=foo@1.0.0")()
	out, err := e.try("run", script)
	if err == nil || !strings.Contains(out+err.Error(), "contradicts") {
		t.Fatalf("paths.deps should bring the lib manifest into resolution, got out=%q err=%v", out, err)
	}
}
