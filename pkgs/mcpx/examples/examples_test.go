// Package examples_test runs the scripts in this directory for real.
//
// The point of the directory is that the documentation cannot rot: every
// script here is executed against the deterministic fixture server, and its
// stdout is compared with the .expected.txt beside it. If an example stops
// printing what the gallery page says it prints, this test fails the build.
package examples_test

import (
	"bytes"
	"context"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"regexp"
	"slices"
	"strings"
	"testing"
	"time"

	"github.com/dezren39/mcpx/internal/testsupport"
)

// paramDecl is the head of a recipe declaration, the same shape
// internal/recipes parses. A script that declares one is run through
// `mcpx recipes save` and `mcpx recipes run`, because `mcpx run` refuses a
// file whose @param holes it cannot fill.
var paramDecl = regexp.MustCompile(`(?m)^\s*//\s*@param\s+[A-Za-z_]`)

// recipeArgs are the values each recipe-shaped example is run with. A
// script that declares @param and has no entry here fails the test rather
// than being silently skipped.
var recipeArgs = map[string][]string{
	"recipe-echo": {"message=piped through a recipe"},
}

func TestExamples(t *testing.T) {
	mcpx := testsupport.MCPXBinary(t)
	fake := testsupport.FakeMCPBinary(t)

	scripts, err := filepath.Glob("*.ts")
	if err != nil {
		t.Fatal(err)
	}
	// Running a script writes the generated client beside it. Skip those in
	// a tree where somebody has already run an example by hand; they are
	// ignored by .gitignore and are not examples.
	scripts = slices.DeleteFunc(scripts, isGenerated)
	// The gallery is built from these files, so an empty directory means the
	// walk is wrong, not that there is nothing to show.
	if len(scripts) < 3 {
		t.Fatalf("found %d examples in %s; expected at least 3", len(scripts), mustGetwd(t))
	}

	for _, script := range scripts {
		name := strings.TrimSuffix(script, ".ts")
		script := script
		t.Run(name, func(t *testing.T) {
			runExample(t, mcpx, fake, name, script)
		})
	}
}

func runExample(t *testing.T, mcpx, fake, name, script string) {
	t.Helper()

	src, err := os.ReadFile(script)
	if err != nil {
		t.Fatal(err)
	}
	expected, err := os.ReadFile(name + ".expected.txt")
	if err != nil {
		t.Fatalf("every example needs an expected file: %v", err)
	}

	// A fully isolated installation: its own HOME, config, state and daemon,
	// so one example cannot see what another recorded. The arrangement is the
	// one internal/e2e uses; the reason for each variable is there.
	dir := testsupport.TempDir(t)
	home := filepath.Join(dir, "home")
	if err := os.MkdirAll(home, 0o700); err != nil {
		t.Fatal(err)
	}
	cfgPath := filepath.Join(dir, ".mcpx.json")
	cfg := strings.ReplaceAll(configTemplate, "FAKE", fake)
	if err := os.WriteFile(cfgPath, []byte(cfg), 0o644); err != nil {
		t.Fatal(err)
	}
	env := isolatedEnv(dir, cfgPath)

	// Run a copy, not the file in place: `mcpx run <file>` writes
	// mcpx-client.ts and its declarations beside the script, and this
	// directory belongs to the repository. The copy keeps every generated
	// file inside the test's own temp directory.
	runPath := filepath.Join(dir, filepath.Base(script))
	if err := os.WriteFile(runPath, src, 0o644); err != nil {
		t.Fatal(err)
	}

	t.Cleanup(func() {
		// --all reads the daemon-<key>.json info files, which record the
		// socket path the daemon actually bound. Plain `stop` recomputes that
		// path and can miss. Scoped to this test by its own state directory.
		if out, err := runIn(context.Background(), dir, env, mcpx, "stop", "--all"); err != nil {
			t.Logf("cleanup: mcpx stop --all failed: %v\n%s", err, out)
		}
	})

	args := []string{"run", runPath}
	if paramDecl.Match(src) {
		values, ok := recipeArgs[name]
		if !ok {
			t.Fatalf("%s declares @param but recipeArgs has no values for it", script)
		}
		if out, err := runIn(t.Context(), dir, env, mcpx, "recipes", "save", name, runPath); err != nil {
			t.Fatalf("recipes save %s: %v\n%s", name, err, out)
		}
		args = append([]string{"recipes", "run", name}, values...)
	}

	stdout, stderr, err := runFull(context.Background(), dir, env, mcpx, args...)
	if err != nil {
		t.Fatalf("mcpx %s: %v\nstdout:\n%s\nstderr:\n%s",
			strings.Join(args, " "), err, stdout, stderr)
	}
	if stdout != string(expected) {
		t.Errorf("stdout changed; update %s.expected.txt if the new output is right:\n%s",
			name, diffLines(string(expected), stdout))
	}
}

// configTemplate points the demo namespace at the fixture server. FAKE is
// replaced with the compiled fakemcp path.
const configTemplate = `{
  "mcpServers": {
    "demo": { "command": "FAKE" }
  }
}
`

// isolatedEnv is the ambient environment with every MCPX_ variable dropped
// and this installation's own values added, so a developer's exported
// settings cannot change what an example prints.
func isolatedEnv(dir, cfgPath string) []string {
	var env []string
	for _, kv := range os.Environ() {
		if strings.HasPrefix(kv, "MCPX_") {
			continue
		}
		env = append(env, kv)
	}
	return append(env,
		"HOME="+filepath.Join(dir, "home"),
		// Recipes and scripts are discovered under XDG_CONFIG_HOME as well as
		// HOME; point both somewhere empty rather than at the developer's.
		"XDG_CONFIG_HOME="+filepath.Join(dir, "xdg"),
		"MCPX_STATE_DIR="+filepath.Join(dir, "state"),
		"MCPX_CACHE_DIR="+filepath.Join(dir, "cache"),
		"MCPX_CONFIG="+cfgPath,
		// A daemon left behind by a crashed run clears itself instead of
		// holding a fixture server for hours.
		"MCPX_AUTOSTART_IDLE_EXIT=60s",
	)
}

// runIn runs mcpx with its output combined, for setup commands.
func runIn(ctx context.Context, dir string, env []string, bin string, args ...string) (string, error) {
	stdout, stderr, err := runFull(ctx, dir, env, bin, args...)
	return stdout + stderr, err
}

// runFull runs mcpx and keeps stdout and stderr apart: only stdout is
// compared, because log() and console.error go to the durable log on stderr.
func runFull(ctx context.Context, dir string, env []string, bin string, args ...string) (string, string, error) {
	ctx, cancel := context.WithTimeout(ctx, 120*time.Second)
	defer cancel()

	cmd := exec.CommandContext(ctx, bin, args...)
	cmd.Dir = dir
	cmd.Env = env
	var stdout, stderr bytes.Buffer
	cmd.Stdout = &stdout
	cmd.Stderr = &stderr
	err := cmd.Run()
	return stdout.String(), stderr.String(), err
}

// diffLines is deliberately small: line numbers, and the old and new text of
// each line that differs. Enough for output this length.
func diffLines(want, got string) string {
	w := strings.Split(strings.TrimSuffix(want, "\n"), "\n")
	g := strings.Split(strings.TrimSuffix(got, "\n"), "\n")

	var b strings.Builder
	for i := 0; i < len(w) || i < len(g); i++ {
		switch {
		case i >= len(w):
			fmt.Fprintf(&b, "  +%d %q\n", i+1, g[i])
		case i >= len(g):
			fmt.Fprintf(&b, "  -%d %q\n", i+1, w[i])
		case w[i] != g[i]:
			fmt.Fprintf(&b, "  -%d %q\n  +%d %q\n", i+1, w[i], i+1, g[i])
		}
	}
	return b.String()
}

// isGenerated reports whether path is a file `mcpx run` leaves behind rather
// than an example script.
func isGenerated(path string) bool {
	base := filepath.Base(path)
	return base == "mcpx-client.ts" || base == "mcpx-globals.d.ts" ||
		strings.Contains(base, ".mcpx-entry")
}

func mustGetwd(t *testing.T) string {
	t.Helper()
	dir, err := os.Getwd()
	if err != nil {
		t.Fatal(err)
	}
	return dir
}
