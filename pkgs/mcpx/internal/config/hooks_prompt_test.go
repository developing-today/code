package config_test

import (
	"testing"

	"github.com/developing-today/code/pkgs/mcpx/internal/config"
)

// A prompt-action entry in the script hooks list resolves with its
// prompt fields intact (#5276); an entry with no action at all is
// dropped, as before.
func TestResolveHooksPromptEntry(t *testing.T) {
	layers := [][]any{
		{
			map[string]any{"phase": "onSuccess", "prompt": "Review @result", "effect": "advise",
				"model": "openai/gpt-5", "values": map[string]any{"who": "drew"}},
			map[string]any{"phase": "before", "script": "console.log(1)"},
			map[string]any{"phase": "onError"},
		},
	}
	hooks := config.ResolveHooks(layers)
	if len(hooks) != 2 {
		t.Fatalf("expected 2 hooks (the action-less entry is dropped), got %+v", hooks)
	}
	if hooks[0].Prompt != "Review @result" || hooks[0].Effect != "advise" ||
		hooks[0].Model != "openai/gpt-5" || hooks[0].Values["who"] != "drew" {
		t.Errorf("prompt hook did not survive resolution: %+v", hooks[0])
	}
	if hooks[1].Script == "" || hooks[1].Prompt != "" {
		t.Errorf("script hook changed shape: %+v", hooks[1])
	}
}
