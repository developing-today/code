package adapter_test

import (
	"context"
	"strings"
	"testing"

	"github.com/developing-today/code/pkgs/mcpx/internal/adapter"
)

func bareEchoSpec() adapter.Spec {
	return adapter.Spec{
		Name: "e", Command: "echo", Bare: true,
		Description: "a bare echo for tests",
	}
}

func TestBareExposesExactlyOneTool(t *testing.T) {
	tools := bareEchoSpec().ExposedTools()
	if len(tools) != 1 || tools[0].Name != "run" {
		t.Fatalf("a bare spec should expose one tool named run: %+v", tools)
	}
	var args, stdin bool
	for _, p := range tools[0].Params {
		if p.Name == "args" && p.Type == "array" {
			args = true
		}
		if p.Name == "stdin" {
			stdin = true
		}
	}
	if !args || !stdin {
		t.Errorf("the passthrough tool should take args: string[] plus stdin: %+v", tools[0].Params)
	}
	if tools[0].Stdin != "stdin" {
		t.Errorf("stdin should ride stdin, not argv: %+v", tools[0])
	}
}

func TestNonBareExposesTheDeclaredTools(t *testing.T) {
	if got := echoSpec().ExposedTools(); len(got) != 1 || got[0].Name != "say" {
		t.Errorf("a declared spec should expose what it declares: %+v", got)
	}
}

func TestBareWithoutDescriptionDoesNotValidate(t *testing.T) {
	spec := bareEchoSpec()
	spec.Description = ""
	err := spec.Validate()
	if err == nil {
		t.Fatal("a bare declaration with no description should not validate")
	}
	if !strings.Contains(err.Error(), "description is required") {
		t.Errorf("the message should name the rule: %v", err)
	}
}

func TestBareWithDeclaredToolsDoesNotValidate(t *testing.T) {
	spec := bareEchoSpec()
	spec.Tools = []adapter.ToolSpec{{Name: "extra"}}
	err := spec.Validate()
	if err == nil {
		t.Fatal("declared tools beside bare should not validate")
	}
	if !strings.Contains(err.Error(), "exactly one tool") {
		t.Errorf("the message should say what bare exposes: %v", err)
	}
}

func TestBareValidSpecValidates(t *testing.T) {
	if err := bareEchoSpec().Validate(); err != nil {
		t.Fatalf("a bare spec with a description should validate: %v", err)
	}
}

func TestBareCallPassesArgvLiterally(t *testing.T) {
	// No shell anywhere: metacharacters must arrive at the binary
	// untouched, or a caller smuggles execution through argv.
	res, err := bareEchoSpec().Call(context.Background(), "run",
		map[string]any{"args": []any{"a;b", "$(touch /tmp/bare-must-not-exist)", "x y"}})
	if err != nil {
		t.Fatal(err)
	}
	for _, want := range []string{"a;b", "$(touch /tmp/bare-must-not-exist)", "x y"} {
		if !strings.Contains(res.Stdout, want) {
			t.Errorf("argv should arrive literally, missing %q from %q", want, res.Stdout)
		}
	}
}

func TestBareCallCarriesStdin(t *testing.T) {
	spec := adapter.Spec{Name: "c", Command: "cat", Bare: true, Description: "a bare cat for tests"}
	res, err := spec.Call(context.Background(), "run", map[string]any{"stdin": "piped"})
	if err != nil {
		t.Fatal(err)
	}
	if strings.TrimSpace(res.Stdout) != "piped" {
		t.Errorf("got %q", res.Stdout)
	}
	if strings.Contains(res.Command, "piped") {
		t.Errorf("a stdin parameter must not also reach the command line: %q", res.Command)
	}
}

func TestBareUnknownToolNamesRun(t *testing.T) {
	_, err := bareEchoSpec().Call(context.Background(), "nope", nil)
	if err == nil || !strings.Contains(err.Error(), "run") {
		t.Errorf("the error should list the one real tool: %v", err)
	}
}
