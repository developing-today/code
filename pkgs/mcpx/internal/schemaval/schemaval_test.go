package schemaval

import (
	"encoding/json"
	"errors"
	"net/http"
	"net/http/httptest"
	"strings"
	"sync/atomic"
	"testing"
)

func mustCompile(t *testing.T, raw string) *Schema {
	t.Helper()
	s, err := Compile(json.RawMessage(raw))
	if err != nil {
		t.Fatalf("Compile(%s): %v", raw, err)
	}
	return s
}

func mustValidate(t *testing.T, s *Schema, instance string) {
	t.Helper()
	if err := s.ValidateJSON(json.RawMessage(instance)); err != nil {
		t.Fatalf("ValidateJSON(%s): %v", instance, err)
	}
}

func mustNotValidate(t *testing.T, s *Schema, instance string) error {
	t.Helper()
	err := s.ValidateJSON(json.RawMessage(instance))
	if err == nil {
		t.Fatalf("ValidateJSON(%s): unexpectedly valid", instance)
	}
	return err
}

// (a) 2020-12 keyword corpus: prefixItems, unevaluatedProperties,
// boolean schemas, $defs internal refs.
func TestCorpus2020_12(t *testing.T) {
	s := mustCompile(t, `{
		"$defs": {"pos": {"type": "integer", "minimum": 1}},
		"type": "object",
		"properties": {
			"pair": {"prefixItems": [{"$ref": "#/$defs/pos"}, {"type": "string"}], "items": false},
			"flag": true,
			"never": false
		},
		"additionalProperties": false
	}`)
	if s.Dialect() != Dialect2020_12 {
		t.Fatalf("dialect = %q, want %q", s.Dialect(), Dialect2020_12)
	}
	mustValidate(t, s, `{"pair": [3, "x"], "flag": {"anything": 1}}`)
	mustNotValidate(t, s, `{"pair": [0, "x"]}`)          // $defs minimum via internal $ref
	mustNotValidate(t, s, `{"pair": [3, "x", "extra"]}`) // prefixItems + items:false
	mustNotValidate(t, s, `{"never": 1}`)                // boolean false schema
	mustNotValidate(t, s, `{"unknown": 1}`)              // additionalProperties:false

	u := mustCompile(t, `{
		"type": "object",
		"properties": {"a": {"type": "string"}},
		"unevaluatedProperties": false
	}`)
	mustValidate(t, u, `{"a": "x"}`)
	mustNotValidate(t, u, `{"a": "x", "b": 1}`)
}

func TestBooleanSchemas(t *testing.T) {
	mustValidate(t, mustCompile(t, `true`), `{"x": 1}`)
	mustNotValidate(t, mustCompile(t, `false`), `{"x": 1}`)
}

// (b) A declared draft-07 schema validates per draft-07: the array
// form of "items" is positional there (2020-12 calls it prefixItems
// and rejects the array form under "items").
func TestDraft07Declared(t *testing.T) {
	s := mustCompile(t, `{
		"$schema": "http://json-schema.org/draft-07/schema#",
		"items": [{"type": "integer"}, {"type": "string"}],
		"additionalItems": false
	}`)
	if s.Dialect() != DialectDraft07 {
		t.Fatalf("dialect = %q, want %q", s.Dialect(), DialectDraft07)
	}
	mustValidate(t, s, `[1, "x"]`)
	mustNotValidate(t, s, `["x", 1]`)
	mustNotValidate(t, s, `[1, "x", true]`)
}

func TestDeclaredDialectsCompile(t *testing.T) {
	for _, tc := range []struct{ url, want string }{
		{"https://json-schema.org/draft/2020-12/schema", Dialect2020_12},
		{"https://json-schema.org/draft/2019-09/schema#", Dialect2019_09},
		{"http://json-schema.org/draft-06/schema#", DialectDraft06},
		{"http://json-schema.org/draft-04/schema#", DialectDraft04},
	} {
		s := mustCompile(t, `{"$schema": "`+tc.url+`", "type": "integer"}`)
		if s.Dialect() != tc.want {
			t.Errorf("$schema %s: dialect = %q, want %q", tc.url, s.Dialect(), tc.want)
		}
		mustValidate(t, s, `3`)
		mustNotValidate(t, s, `"x"`)
	}
}

// (c) An unsupported dialect is its own typed error.
func TestUnsupportedDialect(t *testing.T) {
	for _, raw := range []string{
		`{"$schema": "http://json-schema.org/draft-03/schema#", "type": "integer"}`,
		`{"$schema": "https://example.com/dialects/custom", "type": "integer"}`,
	} {
		_, err := Compile(json.RawMessage(raw))
		var ude *UnsupportedDialectError
		if !errors.As(err, &ude) {
			t.Fatalf("Compile(%s): err = %T %v, want UnsupportedDialectError", raw, err, err)
		}
		var ce *CompileError
		if errors.As(err, &ce) {
			t.Fatalf("Compile(%s): UnsupportedDialect must be distinct from CompileError", raw)
		}
	}
}

// (d) An unresolvable internal $ref is a compile error of its own type.
func TestUnresolvedInternalRef(t *testing.T) {
	_, err := Compile(json.RawMessage(`{"$ref": "#/$defs/nope"}`))
	var ure *UnresolvedRefError
	if !errors.As(err, &ure) {
		t.Fatalf("err = %T %v, want UnresolvedRefError", err, err)
	}
}

// (e) A remote $ref is a compile error and produces zero HTTP traffic.
func TestRemoteRefMakesNoRequests(t *testing.T) {
	var hits atomic.Int64
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		hits.Add(1)
		_, _ = w.Write([]byte(`{"type": "integer"}`))
	}))
	defer srv.Close()
	_, err := Compile(json.RawMessage(`{"$ref": "` + srv.URL + `/schema.json"}`))
	var ure *UnresolvedRefError
	if !errors.As(err, &ure) {
		t.Fatalf("err = %T %v, want UnresolvedRefError", err, err)
	}
	if got := hits.Load(); got != 0 {
		t.Fatalf("remote $ref produced %d HTTP requests, want 0", got)
	}
}

// (f) Integers past float64 precision keep their exact value.
func TestIntegerFidelityPast2To53(t *testing.T) {
	s := mustCompile(t, `{"type": "integer", "maximum": 9007199254740992}`)
	// 2^53+1 rounds to 2^53+... as a float64 it would pass the maximum;
	// decoded exactly it must fail it.
	mustNotValidate(t, s, `9007199254740993`)
	mustValidate(t, s, `9007199254740992`)
	exact := mustCompile(t, `{"type": "integer", "maximum": 9007199254740993}`)
	mustValidate(t, exact, `9007199254740993`)
}

// (g) Validation failures flatten to path-qualified one-line problems.
func TestFlattenedPathErrors(t *testing.T) {
	s := mustCompile(t, `{
		"type": "object",
		"properties": {
			"count": {"type": "integer"},
			"outer": {"type": "object", "properties": {"inner": {"type": "string"}}, "required": ["inner"]}
		},
		"required": ["count"]
	}`)
	err := mustNotValidate(t, s, `{"count": "x", "outer": {"inner": 7}}`)
	var ve *ValidationError
	if !errors.As(err, &ve) {
		t.Fatalf("err = %T, want ValidationError", err)
	}
	if got := err.Error(); !strings.Contains(got, "$.count:") || !strings.Contains(got, "$.outer.inner:") {
		t.Fatalf("flattened error = %q, want $.count and $.outer.inner problems", got)
	}
	if strings.Contains(err.Error(), "\n") {
		t.Fatalf("flattened error must be one line: %q", err.Error())
	}
	// A missing required property is reported at the owning object.
	err = mustNotValidate(t, s, `{"outer": {}}`)
	if !strings.Contains(err.Error(), "$.outer:") {
		t.Fatalf("required error = %q, want a $.outer problem", err.Error())
	}
}

// Combinator failures (anyOf/oneOf/allOf/not/if-then) produce a
// DetailedOutput tree whose interior units carry a nil Error;
// Validate must flatten to the leaves instead of dereferencing it.
// Regression: an anyOf argument failure panicked the daemon in the
// mcpx conformance suite (#1483, PR #1596 server-all crash).
func TestFlattenedCombinatorErrors(t *testing.T) {
	cases := []struct {
		name, schema, inst, wantPath string
	}{
		{"anyOf", `{"anyOf": [{"type": "integer"}, {"type": "boolean"}]}`, `"x"`, "$"},
		{"oneOf", `{"oneOf": [{"type": "integer"}, {"type": "number"}]}`, `3`, "$"},
		{"allOf", `{"allOf": [{"type": "integer"}, {"minimum": 5}]}`, `"x"`, "$"},
		{"not", `{"not": {"type": "integer"}}`, `3`, "$"},
		{"ifThen", `{"if": {"type": "integer"}, "then": {"minimum": 10}}`, `3`, "$"},
		{"nestedAnyOf", `{"type": "object", "properties": {"mode": {"anyOf": [{"const": "a"}, {"const": "b"}]}}}`, `{"mode": "c"}`, "$.mode"},
		{"contains", `{"type": "array", "contains": {"type": "integer"}}`, `["a", "b"]`, "$[0]"},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			s := mustCompile(t, tc.schema)
			err := mustNotValidate(t, s, tc.inst)
			var ve *ValidationError
			if !errors.As(err, &ve) {
				t.Fatalf("err = %T, want ValidationError", err)
			}
			if len(ve.Problems) == 0 {
				t.Fatalf("no problems flattened from %v", err)
			}
			found := false
			for _, pr := range ve.Problems {
				if pr.Path == tc.wantPath {
					found = true
				}
			}
			if !found {
				t.Fatalf("problems = %v, want one at %s", ve.Problems, tc.wantPath)
			}
		})
	}
}

// A schema over the 1 MiB bound fails compilation with the typed error.
func TestSchemaTooLarge(t *testing.T) {
	big := `{"description": "` + strings.Repeat("x", MaxSchemaBytes) + `", "type": "integer"}`
	_, err := Compile(json.RawMessage(big))
	var tle *SchemaTooLargeError
	if !errors.As(err, &tle) {
		t.Fatalf("err = %T %v, want SchemaTooLargeError", err, err)
	}
	if tle.Limit != MaxSchemaBytes {
		t.Fatalf("limit = %d, want %d", tle.Limit, MaxSchemaBytes)
	}
}

// Formats are annotations in v1: they never fail validation.
func TestFormatsAreAnnotations(t *testing.T) {
	s := mustCompile(t, `{"type": "string", "format": "email"}`)
	mustValidate(t, s, `"not-an-email"`)
}

// A malformed schema is a CompileError, not a dialect or ref error.
func TestMalformedSchema(t *testing.T) {
	_, err := Compile(json.RawMessage(`{"type": 123}`))
	var ce *CompileError
	if !errors.As(err, &ce) {
		t.Fatalf("err = %T %v, want CompileError", err, err)
	}
	var ure *UnresolvedRefError
	if errors.As(err, &ure) {
		t.Fatalf("malformed schema misclassified as UnresolvedRefError: %v", err)
	}
	_, err = Compile(json.RawMessage(`{not json`))
	if !errors.As(err, &ce) {
		t.Fatalf("bad JSON: err = %T %v, want CompileError", err, err)
	}
}
