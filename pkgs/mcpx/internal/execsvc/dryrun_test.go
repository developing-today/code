package execsvc_test

import (
	"encoding/json"
	"testing"

	"github.com/developing-today/code/pkgs/mcpx/internal/execsvc"
)

func TestSyntheticResultFollowsTheOutputSchema(t *testing.T) {
	schema := json.RawMessage(`{
		"type": "object",
		"properties": {
			"name": {"type": "string"},
			"count": {"type": "integer"},
			"state": {"type": "string", "enum": ["open", "closed"]},
			"tags": {"type": "array", "items": {"type": "string"}},
			"nested": {"type": "object", "properties": {"ok": {"type": "boolean"}}}
		},
		"required": ["name", "count"]
	}`)
	var res struct {
		Structured map[string]any `json:"structuredContent"`
		IsError    bool           `json:"isError"`
		DryRun     bool           `json:"dryRun"`
		Content    []struct {
			Type string `json:"type"`
			Text string `json:"text"`
		} `json:"content"`
	}
	if err := json.Unmarshal(execsvc.SyntheticResult(schema), &res); err != nil {
		t.Fatal(err)
	}
	if res.IsError || !res.DryRun {
		t.Fatalf("result must be a successful dry-run result: %+v", res)
	}
	want := map[string]any{
		"name": "", "count": float64(0), "state": "open",
		"tags": []any{}, "nested": map[string]any{"ok": false},
	}
	for k, v := range want {
		got, _ := json.Marshal(res.Structured[k])
		exp, _ := json.Marshal(v)
		if string(got) != string(exp) {
			t.Errorf("structuredContent[%q] = %s, want %s", k, got, exp)
		}
	}
	if len(res.Content) != 1 || res.Content[0].Type != "text" || res.Content[0].Text == "" {
		t.Errorf("content must carry the synthetic value as text: %+v", res.Content)
	}
}

func TestSyntheticResultWithoutASchemaIsAMarkedPlaceholder(t *testing.T) {
	var res struct {
		Structured map[string]any `json:"structuredContent"`
		DryRun     bool           `json:"dryRun"`
	}
	if err := json.Unmarshal(execsvc.SyntheticResult(nil), &res); err != nil {
		t.Fatal(err)
	}
	if !res.DryRun || res.Structured["dryRun"] != true {
		t.Fatalf("no-schema result must be the marked placeholder: %+v", res)
	}
}

func TestSyntheticResultIsDeterministic(t *testing.T) {
	schema := json.RawMessage(`{"type":"object","properties":{"b":{"type":"string"},"a":{"type":"integer"}}}`)
	first := execsvc.SyntheticResult(schema)
	for i := 0; i < 5; i++ {
		if got := execsvc.SyntheticResult(schema); string(got) != string(first) {
			t.Fatalf("synthesis must be deterministic: %s vs %s", got, first)
		}
	}
}

func TestCallRegistryDryRunMarking(t *testing.T) {
	r := execsvc.NewCallRegistry()
	if r.IsDryRun("run-1") {
		t.Fatal("an unmarked run is not a dry run")
	}
	unregister := r.RegisterDryRun("run-1")
	if !r.IsDryRun("run-1") {
		t.Fatal("a marked run is a dry run")
	}
	unregister()
	unregister() // idempotent
	if r.IsDryRun("run-1") {
		t.Fatal("an unregistered run is no longer a dry run")
	}

	// Call frames for a dry run say so.
	frames := []execsvc.Frame{}
	r.Register("run-2", func(f execsvc.Frame) { frames = append(frames, f) })
	r.RegisterDryRun("run-2")
	if !r.ReportCall("run-2", "demo", "echo", map[string]any{}, 0, nil, nil) {
		t.Fatal("the call should be reported")
	}
	if len(frames) != 1 || frames[0].Call == nil || !frames[0].Call.DryRun {
		t.Fatalf("dry-run call frame must be marked: %+v", frames)
	}
}
