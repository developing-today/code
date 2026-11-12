package execsvc_test

import (
	"context"
	"errors"
	"strings"
	"testing"

	"github.com/developing-today/code/pkgs/mcpx/internal/execsvc"
	"github.com/developing-today/code/pkgs/mcpx/internal/model"
	"github.com/developing-today/code/pkgs/mcpx/internal/runner"
)

// fakeProvider is a model.Provider that answers from a script of its
// own: it records the request it was given and replies with canned
// text, or an error when err is set.
type fakeProvider struct {
	name string
	text string
	err  error
	last model.Request
}

func (f *fakeProvider) Name() string { return f.name }

func (f *fakeProvider) Generate(_ context.Context, req model.Request) (model.Response, error) {
	f.last = req
	if f.err != nil {
		return model.Response{}, f.err
	}
	return model.Response{Text: f.text, Model: "fake/model-1", Provider: f.name}, nil
}

func promptService(t *testing.T, prov model.Provider) *execsvc.Service {
	t.Helper()
	svc := service(t, store(t))
	if prov != nil {
		svc.ProviderFor = func(context.Context) (model.Provider, error) { return prov, nil }
	}
	return svc
}

func TestPromptHookAdvisesOnSuccessWithoutTransforming(t *testing.T) {
	prov := &fakeProvider{name: "fake", text: "  looks fine  "}
	svc := promptService(t, prov)
	res, frames := run(t, svc, "console.log(JSON.stringify({ total: 2 }));\n", execsvc.Options{Hooks: []execsvc.Hook{
		{Phase: execsvc.HookOnSuccess, Prompt: "Review @result for problems.", Model: "fake/model-1"},
	}})

	// The result is the script's own: advice never transforms.
	m, ok := res.Result.(map[string]any)
	if !ok || m["total"] != float64(2) {
		t.Fatalf("a prompt hook must not transform the result: %#v", res.Result)
	}
	hooks := hookFrames(frames)
	if len(hooks) != 1 {
		t.Fatalf("expected 1 hook frame, got %d", len(hooks))
	}
	hf := hooks[0].Hook
	if hf.Action != "prompt" || hf.Effect != "advise" || hf.Changed {
		t.Errorf("prompt hook frame = %+v", hf)
	}
	if hf.Advice != "looks fine" {
		t.Errorf("advice = %q, want the provider's trimmed text", hf.Advice)
	}
	if hf.Model != "fake/model-1" {
		t.Errorf("frame model = %q, want the answering model", hf.Model)
	}
	// The provider saw the rendered prompt with @result as JSON data,
	// and the hook's own model selection.
	if len(prov.last.Messages) != 1 || !strings.Contains(prov.last.Messages[0].Content, "{\"total\":2}") {
		t.Errorf("provider request = %+v", prov.last)
	}
	if prov.last.Model != "fake/model-1" {
		t.Errorf("request model = %q, want the hook's selection", prov.last.Model)
	}
	// Placement: the hook frame precedes the result frame it surrounds.
	var kinds []string
	for _, f := range frames {
		kinds = append(kinds, f.Type)
	}
	if idx(kinds, execsvc.FrameHook) > idx(kinds, execsvc.FrameResult) {
		t.Errorf("hook frame must precede the result frame: %s", strings.Join(kinds, ","))
	}
}

func TestPromptHookAdvisesOnErrorAndKeepsTheError(t *testing.T) {
	prov := &fakeProvider{name: "fake", text: "check the input"}
	svc := promptService(t, prov)
	res, frames := run(t, svc, "throw new Error(\"boom\");\n", execsvc.Options{Hooks: []execsvc.Hook{
		{Phase: execsvc.HookOnError, Prompt: "Explain @error briefly."},
	}})
	if res.Error == "" {
		t.Fatalf("the script's error must survive a prompt hook: %+v", res)
	}
	hooks := hookFrames(frames)
	if len(hooks) != 1 || hooks[0].Hook.Advice != "check the input" || hooks[0].Hook.Changed {
		t.Fatalf("onError prompt hook frames = %+v", hooks)
	}
	if len(prov.last.Messages) != 1 || !strings.Contains(prov.last.Messages[0].Content, "boom") {
		t.Errorf("provider did not see the error as data: %+v", prov.last)
	}
}

func TestPromptHookWithoutProviderWarnsAndContinues(t *testing.T) {
	svc := promptService(t, nil)
	res, frames := run(t, svc, "console.log(JSON.stringify({ ok: true }));\n", execsvc.Options{Hooks: []execsvc.Hook{
		{Phase: execsvc.HookOnSuccess, Prompt: "Any advice on @result?"},
	}})
	if res.Result == nil {
		t.Fatalf("the run must complete without a provider: %+v", res)
	}
	hooks := hookFrames(frames)
	if len(hooks) != 1 || !strings.Contains(hooks[0].Hook.Error, "no model provider") || hooks[0].Hook.Advice != "" {
		t.Fatalf("expected a warning frame with no advice: %+v", hooks)
	}
}

func TestPromptHookProviderErrorWarnsAndContinues(t *testing.T) {
	prov := &fakeProvider{name: "fake", err: errors.New("connection refused")}
	svc := promptService(t, prov)
	res, frames := run(t, svc, "console.log(JSON.stringify({ ok: true }));\n", execsvc.Options{Hooks: []execsvc.Hook{
		{Phase: execsvc.HookBefore, Prompt: "Heads-up on @phase?"},
	}})
	if res.Result == nil {
		t.Fatalf("the run must complete when the provider fails: %+v", res)
	}
	hooks := hookFrames(frames)
	if len(hooks) != 1 || !strings.Contains(hooks[0].Hook.Error, "advice unavailable") || hooks[0].Hook.Advice != "" {
		t.Fatalf("expected an advice-unavailable frame: %+v", hooks)
	}
}

func TestPromptPlaceholdersAreDataNotText(t *testing.T) {
	prov := &fakeProvider{name: "fake", text: "noted"}
	svc := promptService(t, prov)
	// The result value contains quote-bearing, placeholder-shaped text.
	// Rendered as JSON data its quote stays escaped inside the string;
	// rendered as raw text it would break out of the value.
	run(t, svc, "console.log(JSON.stringify({ note: \"a\\\"b @error\" }));\n", execsvc.Options{Hooks: []execsvc.Hook{
		{Phase: execsvc.HookOnSuccess, Prompt: "Note: @result. Who: @who.", Values: map[string]any{"who": "drew"}},
	}})
	got := prov.last.Messages[0].Content
	if !strings.Contains(got, "\"drew\"") {
		t.Errorf("user placeholder not rendered as JSON data: %q", got)
	}
	if !strings.Contains(got, "a\\\"b") {
		t.Errorf("value was not JSON-encoded (escaped quote missing): %q", got)
	}
	if strings.Contains(got, "a\"b") {
		t.Errorf("value was substituted as raw text: %q", got)
	}
}

func TestPromptHookValidation(t *testing.T) {
	needRuntime(t)
	svc := promptService(t, &fakeProvider{name: "fake", text: "x"})
	cases := []struct {
		name string
		hook execsvc.Hook
		want string
	}{
		{"both actions", execsvc.Hook{Phase: execsvc.HookBefore, Script: "1", Prompt: "hi"}, "exactly one action"},
		{"no action", execsvc.Hook{Phase: execsvc.HookBefore}, "script is empty"},
		{"prompt above advise", execsvc.Hook{Phase: execsvc.HookOnSuccess, Prompt: "hi", Effect: "apply"}, "advise"},
		{"model without prompt", execsvc.Hook{Phase: execsvc.HookBefore, Script: "1", Model: "fake/m"}, "no prompt action"},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			_, err := svc.RunWith(context.Background(), runner.Options{
				Source: "console.log(1);\n", ClientSource: minimalClient, Permissions: "all",
			}, execsvc.Options{Hooks: []execsvc.Hook{tc.hook}}, func(execsvc.Frame) error { return nil })
			if err == nil || !strings.Contains(err.Error(), tc.want) {
				t.Fatalf("err = %v, want it to name %q", err, tc.want)
			}
		})
	}
}
