package schemaval

import (
	"encoding/json"
	"testing"
)

// Probe: draft-04 boolean exclusiveMinimum/exclusiveMaximum semantics.
func TestProbeDraft04Exclusive(t *testing.T) {
	s, err := Compile(json.RawMessage(`{
		"$schema": "http://json-schema.org/draft-04/schema#",
		"type": "integer", "minimum": 5, "exclusiveMinimum": true
	}`))
	if err != nil {
		t.Fatalf("draft-04 exclusiveMinimum:true compile: %v", err)
	}
	if err := s.ValidateJSON(json.RawMessage(`5`)); err == nil {
		t.Fatalf("draft-04: 5 valid but exclusiveMinimum:true must refuse it")
	}
	if err := s.ValidateJSON(json.RawMessage(`6`)); err != nil {
		t.Fatalf("draft-04: 6 invalid: %v", err)
	}
	s2, err := Compile(json.RawMessage(`{
		"$schema": "http://json-schema.org/draft-04/schema#",
		"type": "integer", "maximum": 5, "exclusiveMaximum": true
	}`))
	if err != nil {
		t.Fatalf("draft-04 exclusiveMaximum:true compile: %v", err)
	}
	if err := s2.ValidateJSON(json.RawMessage(`5`)); err == nil {
		t.Fatalf("draft-04: 5 valid but exclusiveMaximum:true must refuse it")
	}
}

// Probe: draft-04 non-exclusive control.
func TestProbeDraft04NonExclusive(t *testing.T) {
	s, err := Compile(json.RawMessage(`{
		"$schema": "http://json-schema.org/draft-04/schema#",
		"type": "integer", "minimum": 5, "exclusiveMinimum": false
	}`))
	if err != nil {
		t.Fatalf("compile: %v", err)
	}
	if err := s.ValidateJSON(json.RawMessage(`5`)); err != nil {
		t.Fatalf("draft-04 exclusiveMinimum:false: 5 invalid: %v", err)
	}
}
