package mcpserver_test

import (
	"encoding/json"
	"reflect"
	"testing"

	"github.com/dezren39/mcpx/internal/mcpserver"
)

// Phase 1 of #2077 (#2078): a 2025-11-25 upstream's rich enum shapes
// must not reach a pre-2025-11-25 host verbatim. Titled single-select
// downgrades to the legacy enum+enumNames shape in paramsFor;
// multi-select is withheld whole by Sendable, never flattened; and a
// non-boolean default -- illegal at 2025-06-18 (ELI-05) -- is stripped
// in the same pass while a boolean default stays.

func enumQuestion(schema string) mcpserver.Question {
	return mcpserver.Question{ID: "e", Method: "elicitation/create", Mode: "form",
		Params: json.RawMessage(`{"message":"pick","mode":"form","requestedSchema":{"type":"object","properties":` + schema + `,"required":["choice"]}}`)}
}

func renderedProps(t *testing.T, q mcpserver.Question, p mcpserver.Peer) map[string]any {
	t.Helper()
	out, err := q.ParamsFor(p)
	if err != nil {
		t.Fatal(err)
	}
	var got struct {
		RequestedSchema struct {
			Properties map[string]any `json:"properties"`
		} `json:"requestedSchema"`
	}
	if err := json.Unmarshal(out, &got); err != nil {
		t.Fatal(err)
	}
	return got.RequestedSchema.Properties
}

func TestTitledSingleSelectDowngradesForPre20251125Host(t *testing.T) {
	old := peer("2025-06-18", `{"elicitation":{}}`)
	q := enumQuestion(`{"choice":{"oneOf":[{"const":"a","title":"Alpha"},{"const":"b","title":"Beta"}],"description":"pick one"}}`)
	props := renderedProps(t, q, old)
	want := map[string]any{
		"type": "string", "description": "pick one",
		"enum": []any{"a", "b"}, "enumNames": []any{"Alpha", "Beta"},
	}
	if !reflect.DeepEqual(props["choice"], want) {
		t.Errorf("titled single-select not downgraded for 2025-06-18: got %v want %v", props["choice"], want)
	}
}

func TestEnumShapesPassThroughUnchanged(t *testing.T) {
	old := peer("2025-06-18", `{"elicitation":{}}`)
	newer := peer("2025-11-25", `{"elicitation":{"form":{}}}`)
	cases := map[string]string{
		"untitled single": `{"choice":{"type":"string","enum":["a","b"]}}`,
		"legacy titled":   `{"choice":{"type":"string","enum":["a","b"],"enumNames":["Alpha","Beta"]}}`,
		"plain string":    `{"choice":{"type":"string"}}`,
		"number":          `{"choice":{"type":"number"}}`,
	}
	for name, schema := range cases {
		var want map[string]any
		if err := json.Unmarshal([]byte(schema), &want); err != nil {
			t.Fatal(err)
		}
		props := renderedProps(t, enumQuestion(schema), old)
		if !reflect.DeepEqual(props, want) {
			t.Errorf("%s must pass byte-identical in shape to a 2025-06-18 host: got %v want %v", name, props, want)
		}
	}
	// A 2025-11-25 host sees even the titled shape byte-identical.
	titled := `{"choice":{"oneOf":[{"const":"a","title":"Alpha"}]}}`
	var want map[string]any
	if err := json.Unmarshal([]byte(titled), &want); err != nil {
		t.Fatal(err)
	}
	if props := renderedProps(t, enumQuestion(titled), newer); !reflect.DeepEqual(props, want) {
		t.Errorf("titled single-select must pass unchanged to a 2025-11-25 host: got %v want %v", props, want)
	}
}

func TestMultiSelectWithheldFromPre20251125Host(t *testing.T) {
	old := peer("2025-06-18", `{"elicitation":{}}`)
	newer := peer("2025-11-25", `{"elicitation":{"form":{}}}`)
	untitled := enumQuestion(`{"choice":{"type":"array","items":{"type":"string","enum":["a","b"]}}}`)
	titled := enumQuestion(`{"choice":{"type":"array","items":{"anyOf":[{"const":"a","title":"Alpha"}]}}}`)
	single := enumQuestion(`{"choice":{"type":"string","enum":["a","b"]}}`)
	for name, q := range map[string]mcpserver.Question{"untitled multi": untitled, "titled multi": titled} {
		if q.Sendable(old) {
			t.Errorf("%s question must be withheld from a 2025-06-18 host", name)
		}
		if !q.Sendable(newer) {
			t.Errorf("%s question must be offered to a 2025-11-25 host", name)
		}
	}
	if !single.Sendable(old) {
		t.Error("a single-select question must stay sendable to a 2025-06-18 host")
	}
}

func TestMultiSelectToleratesOtherPropertyShapes(t *testing.T) {
	old := peer("2025-06-18", `{"elicitation":{}}`)
	newer := peer("2025-11-25", `{"elicitation":{"form":{}}}`)
	cases := map[string]string{
		"single control":        `{"choice":{"type":"string","enum":["a"]}}`,
		"multi control":         `{"choice":{"type":"array","items":{"enum":["a"]}}}`,
		"union sibling":         `{"choice":{"type":"array","items":{"enum":["a"]}},"other":{"type":["string","null"]}}`,
		"boolean items sibling": `{"choice":{"type":"array","items":{"anyOf":[]}},"other":{"type":"array","items":true}}`,
		"union multi":           `{"choice":{"type":["array","null"],"items":{"enum":["a"]}}}`,
		"non-array union":       `{"choice":{"type":["string","null"],"items":{"enum":["a"]}}}`,
	}
	for name, schema := range cases {
		t.Run(name, func(t *testing.T) {
			q := enumQuestion(schema)
			want := name == "single control" || name == "non-array union"
			if got := q.Sendable(old); got != want {
				t.Errorf("old host Sendable = %v, want %v", got, want)
			}
			if !q.Sendable(newer) {
				t.Error("new host must receive question")
			}
		})
	}
}

func TestNonBooleanDefaultsStrippedForPre20251125Host(t *testing.T) {
	old := peer("2025-06-18", `{"elicitation":{}}`)
	newer := peer("2025-11-25", `{"elicitation":{"form":{}}}`)
	schema := `{"choice":{"type":"string","enum":["a","b"],"default":"a"},"titled":{"oneOf":[{"const":"x","title":"X"}],"default":"x"},"count":{"type":"number","default":3},"flag":{"type":"boolean","default":true}}`
	props := renderedProps(t, enumQuestion(schema), old)
	for _, name := range []string{"choice", "titled", "count"} {
		if _, ok := props[name].(map[string]any)["default"]; ok {
			t.Errorf("non-boolean default must be stripped for a 2025-06-18 host: %s = %v", name, props[name])
		}
	}
	if props["flag"].(map[string]any)["default"] != true {
		t.Errorf("boolean default must be kept for a 2025-06-18 host: %v", props["flag"])
	}
	props = renderedProps(t, enumQuestion(schema), newer)
	if props["choice"].(map[string]any)["default"] != "a" {
		t.Errorf("defaults must pass unchanged to a 2025-11-25 host: %v", props["choice"])
	}
}
