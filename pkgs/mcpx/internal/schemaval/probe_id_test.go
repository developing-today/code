package schemaval

import (
	"encoding/json"
	"testing"
)

func TestProbeIDRelative(t *testing.T) {
	s, err := Compile(json.RawMessage(`{"$id": "foo.json", "type": "integer"}`))
	if err != nil {
		t.Fatalf("relative $id compile: %v", err)
	}
	if err := s.ValidateJSON(json.RawMessage(`3`)); err != nil {
		t.Fatalf("validate: %v", err)
	}
}

func TestProbeIDSelfRef(t *testing.T) {
	s, err := Compile(json.RawMessage(`{
		"$id": "https://example.com/root.json",
		"$defs": {"pos": {"type": "integer", "minimum": 1}},
		"$ref": "https://example.com/root.json#/$defs/pos"
	}`))
	if err != nil {
		t.Fatalf("self-$id absolute ref compile: %v", err)
	}
	if err := s.ValidateJSON(json.RawMessage(`0`)); err == nil {
		t.Fatalf("0 valid, want minimum 1 refusal via self-$id ref")
	}
	if err := s.ValidateJSON(json.RawMessage(`2`)); err != nil {
		t.Fatalf("2 invalid: %v", err)
	}
}

func TestProbeIDRelativeRef(t *testing.T) {
	s, err := Compile(json.RawMessage(`{
		"$id": "https://example.com/root.json",
		"$defs": {"pos": {"type": "integer", "minimum": 1}},
		"$ref": "root.json#/$defs/pos"
	}`))
	if err != nil {
		t.Fatalf("self-$id relative ref compile: %v", err)
	}
	if err := s.ValidateJSON(json.RawMessage(`0`)); err == nil {
		t.Fatalf("0 valid, want minimum 1 refusal via relative self ref")
	}
}
