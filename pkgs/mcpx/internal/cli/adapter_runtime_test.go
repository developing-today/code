package cli

import (
	"context"
	"encoding/json"
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/dezren39/mcpx/internal/adapter"
	"github.com/dezren39/mcpx/internal/mcpserver"
	"github.com/dezren39/mcpx/internal/recipes"
)

// These spawn for real: `mcpx adapter serve` mounts exactly these extras,
// and a runtime that resolved but could not run would pass every other test.

func servedAdapter(t *testing.T, spec adapter.Spec) *mcpserver.Server {
	t.Helper()
	if err := spec.Validate(); err != nil {
		t.Fatal(err)
	}
	if err := spec.Probe(); err != nil {
		t.Fatal(err)
	}
	srv := mcpserver.New(nil, spec.Name, "test")
	srv.ExtrasOnly = true
	return srv.WithExtras(adapterTools([]adapter.Spec{spec}, false))
}

func TestABashScriptDeclaredWithARuntimeServesAndAnswers(t *testing.T) {
	path := writeScript(t, t.TempDir(), "greet.sh", "#!/bin/sh\necho \"hello $1\"\n")
	spec := adapter.Spec{
		Name: "shout", Command: path, Runtime: "bash",
		Tools: []adapter.ToolSpec{{
			Name:   "greet",
			Params: []adapter.Param{{Name: "who", Required: true}},
		}},
	}
	srv := servedAdapter(t, spec)
	out, err := srv.InvokeTool(context.Background(), "greet",
		json.RawMessage(`{"who":"world"}`))
	if err != nil {
		t.Fatal(err)
	}
	if !strings.Contains(out, "hello world") {
		t.Errorf("out = %q", out)
	}
}

func TestAPythonScriptDeclaredWithARuntimeServesAndAnswers(t *testing.T) {
	if _, _, err := adapter.RuntimeInterpreter("py"); err != nil {
		t.Skip("no python3 on this machine: ", err)
	}
	path := writeScript(t, t.TempDir(), "greet.py",
		"import sys\nprint('hello ' + sys.argv[1])\n")
	spec := adapter.Spec{
		Name: "snake", Command: path, Runtime: "py",
		Tools: []adapter.ToolSpec{{
			Name:   "greet",
			Params: []adapter.Param{{Name: "who", Required: true}},
		}},
	}
	srv := servedAdapter(t, spec)
	out, err := srv.InvokeTool(context.Background(), "greet",
		json.RawMessage(`{"who":"world"}`))
	if err != nil {
		t.Fatal(err)
	}
	if !strings.Contains(out, "hello world") {
		t.Errorf("out = %q", out)
	}
}

func TestAScriptPathIsResolvedAgainstTheDeclarationsDir(t *testing.T) {
	// A declaration says where its program's files live; a script beside
	// them is the ordinary case, and "./x.sh" has to mean that file.
	dir := t.TempDir()
	if err := os.WriteFile(filepath.Join(dir, "local.sh"),
		[]byte("#!/bin/sh\necho local\n"), 0o755); err != nil {
		t.Fatal(err)
	}
	spec := adapter.Spec{
		Name: "local", Command: "./local.sh", Runtime: "bash", Dir: dir,
		Tools: []adapter.ToolSpec{{Name: "run"}},
	}
	srv := servedAdapter(t, spec)
	out, err := srv.InvokeTool(context.Background(), "run", nil)
	if err != nil {
		t.Fatal(err)
	}
	if !strings.Contains(out, "local") {
		t.Errorf("out = %q", out)
	}
}

func TestFormatRecipeToolStateNamesWhatWasSkipped(t *testing.T) {
	// `mcpx catalog` is where somebody finds out what is exposed; a recipe
	// kept out by a collision has to be visible there.
	set := recipes.Tools(
		[]recipes.Recipe{
			recipes.Parse("taken", "/tmp/taken.ts", "// x\n"),
			recipes.Parse("off", "/tmp/off.ts", "// x\n// @tool false\n"),
		},
		map[string]bool{"recipe_taken": true}, 0)
	text := formatRecipeToolState(set, nil)
	for _, want := range []string{
		"recipe tools:", "exposed (budget", "1 opted out",
		"recipe_taken", "already exposed",
	} {
		if !strings.Contains(text, want) {
			t.Errorf("missing %q from:\n%s", want, text)
		}
	}
}

func TestAnAdapterCommandWithNoRuntimeIsUnchanged(t *testing.T) {
	// Backwards compatible: a declared binary is still looked up on PATH
	// and exec'd directly.
	spec := adapter.Spec{
		Name: "echoer", Command: "echo",
		Tools: []adapter.ToolSpec{{Name: "say", Params: []adapter.Param{{Name: "x"}}}},
	}
	srv := servedAdapter(t, spec)
	out, err := srv.InvokeTool(context.Background(), "say", json.RawMessage(`{"x":"hi"}`))
	if err != nil {
		t.Fatal(err)
	}
	if !strings.Contains(out, "hi") {
		t.Errorf("out = %q", out)
	}
}
