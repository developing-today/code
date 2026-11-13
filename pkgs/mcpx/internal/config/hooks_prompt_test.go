package config

import "testing"

// A config-declared hook may carry a prompt action (#5276): the entry is
// usable with a prompt and no script, and its effect/model survive
// resolution. An entry with no action at all is still dropped.
func TestResolveHooksPromptEntry(t *testing.T) {
	got := ResolveHooks([][]any{{
		map[string]any{
			"phase": "onSuccess", "prompt": "advise on @result",
			"effect": "advise", "model": "openai/gpt-5", "name": "reviewer",
		},
		map[string]any{"phase": "before", "script": "prep.ts"},
		map[string]any{"phase": "suffix"},
	}})
	if len(got) != 2 {
		t.Fatalf("resolved %d hooks, want 2 (the actionless entry dropped): %+v", len(got), got)
	}
	p := got[0]
	if p.Prompt != "advise on @result" || p.Effect != "advise" || p.Model != "openai/gpt-5" || p.Name != "reviewer" {
		t.Fatalf("prompt hook mangled in resolution: %+v", p)
	}
	if got[1].Script != "prep.ts" {
		t.Fatalf("script hook mangled in resolution: %+v", got[1])
	}
}
