package cli

import (
	"strings"
	"testing"

	"github.com/developing-today/code/pkgs/mcpx/internal/execsvc"
)

// Oracle: a prompt hook's advice is its sole output, and mcpx_exec's
// tool result is the only channel an MCP caller has — renderExec
// must carry retained advice and provider warnings in the rendered
// result (and in the error text on a failed run) without displacing
// the script's own outcome (Codex review, PR #9021).
func TestRenderExecCarriesHookAdvice(t *testing.T) {
	hooks := []execsvc.HookFrame{
		{Phase: "onSuccess", Action: "prompt", Advice: "cache the lookup"},
		{Phase: "onError", Action: "prompt", Error: "no model provider configured"},
	}

	out, err := renderExec(execsvc.Result{Stdout: "answer\n", Hooks: hooks})
	if err != nil {
		t.Fatalf("renderExec: %v", err)
	}
	for _, want := range []string{"answer", "hook advice (onSuccess): cache the lookup", "hook warning (onError): no model provider configured"} {
		if !strings.Contains(out, want) {
			t.Fatalf("rendered result missing %q:\n%s", want, out)
		}
	}

	_, err = renderExec(execsvc.Result{Error: "boom", Hooks: hooks})
	if err == nil || !strings.Contains(err.Error(), "boom") || !strings.Contains(err.Error(), "cache the lookup") {
		t.Fatalf("error path dropped hook advice: %v", err)
	}

	out, err = renderExec(execsvc.Result{Hooks: hooks})
	if err != nil || !strings.Contains(out, "cache the lookup") {
		t.Fatalf("advice-only result = %q, %v", out, err)
	}
}
