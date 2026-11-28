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

type fixedCallHistory struct {
	calls []execsvc.CallInfo
	err   error
	begin error
}

func (h *fixedCallHistory) Begin(context.Context, string) error { return h.begin }
func (h *fixedCallHistory) Snapshot(context.Context, string) ([]execsvc.CallInfo, error) {
	return h.calls, h.err
}

type capturePromptProvider struct{ request model.Request }

func (*capturePromptProvider) Name() string { return "capture" }
func (p *capturePromptProvider) Generate(_ context.Context, req model.Request) (model.Response, error) {
	p.request = req
	return model.Response{Text: "observed", Model: "capture-1"}, nil
}

func TestLocalOutcomePromptGetsCollectedCallsAndExplicitUnavailable(t *testing.T) {
	call := execsvc.CallInfo{Server: "weather", Tool: "forecast", ArgsHash: "sha256:abc", OK: true}
	cases := []struct {
		name    string
		history *fixedCallHistory
		want    string
		notWant string
	}{
		{name: "collected", history: &fixedCallHistory{calls: []execsvc.CallInfo{call}}, want: `"tool":"forecast"`, notWant: `"calls":[]`},
		{name: "snapshot failure", history: &fixedCallHistory{err: errors.New("daemon unavailable")}, want: `"available":false`, notWant: `"calls":[]`},
		{name: "begin failure", history: &fixedCallHistory{begin: errors.New("registration failed")}, want: `"available":false`, notWant: `"calls":[]`},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			provider := &capturePromptProvider{}
			svc := service(t, store(t))
			svc.CallHistory = tc.history
			svc.ModelProvider = provider
			_, err := svc.RunWith(context.Background(), runner.Options{
				Source: "console.log('script completed')", ClientSource: minimalClient, Permissions: "all",
			}, execsvc.Options{Hooks: []execsvc.Hook{{Phase: execsvc.HookOnSuccess, Prompt: "calls=@calls"}}}, nil)
			if err != nil {
				t.Fatalf("RunWith: %v", err)
			}
			if len(provider.request.Messages) != 2 {
				t.Fatalf("provider messages = %+v", provider.request.Messages)
			}
			prompt := provider.request.Messages[1].Content
			if !strings.Contains(prompt, tc.want) {
				t.Fatalf("prompt = %s, want substring %q", prompt, tc.want)
			}
			if strings.Contains(prompt, tc.notWant) {
				t.Fatalf("prompt = %s, must not contain %q", prompt, tc.notWant)
			}
			if tc.name == "collected" && !strings.Contains(prompt, `"argsHash":"sha256:abc"`) {
				t.Fatalf("prompt omitted the hashed arguments: %s", prompt)
			}
		})
	}
}
