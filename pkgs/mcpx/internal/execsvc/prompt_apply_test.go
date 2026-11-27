package execsvc_test

import (
	"context"
	"strings"
	"testing"

	"github.com/developing-today/code/pkgs/mcpx/internal/execsvc"
	"github.com/developing-today/code/pkgs/mcpx/internal/model"
)

// promptApplyProvider is a model.Provider that answers from a
// script and records its last request, so prompt hook tests never
// reach a network or a real model.
type promptApplyProvider struct {
	text string
	last model.Request
}

func (f *promptApplyProvider) Name() string { return "fake" }

func (f *promptApplyProvider) Generate(_ context.Context, req model.Request) (model.Response, error) {
	f.last = req
	return model.Response{Text: f.text, Model: "fake-1", Provider: "fake"}, nil
}

// Composition oracle for #15845: an apply prompt hook after a
// script hook sees the script hook's transformed value (the
// runHooks context-refresh rule), and its own replacement is what
// the caller receives.
func TestPromptApplyAfterScriptHookSeesTransformedValue(t *testing.T) {
	svc := service(t, store(t))
	fake := &promptApplyProvider{text: `{"apply": {"total": 11}}`}
	svc.ModelProvider = fake
	res, _ := run(t, svc, `
console.log(JSON.stringify({ total: 3 }));
`, execsvc.Options{Hooks: []execsvc.Hook{
		{Phase: execsvc.HookOnSuccess, Script: `console.log(JSON.stringify({ apply: { total: 10 } }));`},
		{Phase: execsvc.HookOnSuccess, Prompt: "fix @result", Effect: "apply"},
	}})
	if len(fake.last.Messages) != 2 || !strings.Contains(fake.last.Messages[1].Content, `"total":10`) {
		t.Fatalf("prompt hook did not see the script hook's transformed value: %+v", fake.last.Messages)
	}
	m, ok := res.Result.(map[string]any)
	if !ok || m["total"] != float64(11) {
		t.Fatalf("result = %#v, want the prompt hook's replacement 11", res.Result)
	}
}
