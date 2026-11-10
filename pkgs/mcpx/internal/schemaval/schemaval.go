// Package schemaval is mcpx's JSON Schema validator for tool and
// elicitation schemas: a real JSON Schema 2020-12 validator with an
// explicit dialect policy, compiled once and offline-only.
//
// Division of labour: internal/mcpspec validates MCP protocol frames and
// is test-only (it must never reach the binary); schemaval validates
// tool/elicitation schemas and is the validator the binary carries. This
// package is the only one in mcpx that imports
// github.com/santhosh-tekuri/jsonschema/v6.
//
// Policy (see docs/schema-validation.md):
//   - Dialects: no $schema means 2020-12. A declared $schema must name
//     2020-12, 2019-09, draft-07, draft-06 or draft-04; anything else is
//     an *UnsupportedDialectError.
//   - No network, ever: the compiler's loader refuses every URL, so a
//     schema can only reference itself or resources already compiled
//     into the same document. An unresolvable $ref — internal or
//     external — is an *UnresolvedRefError at compile time.
//   - Size bound: a schema document larger than MaxSchemaBytes fails
//     compilation with a *SchemaTooLargeError.
//   - Formats are annotations, not assertions, in v1: a value that
//     violates a "format" keyword still validates.
//   - Instances are decoded with jsonschema.UnmarshalJSON, so integers
//     beyond float64 precision (e.g. 2^53+1) keep their exact value.
package schemaval

import (
	"bytes"
	"encoding/json"
	"errors"
	"fmt"
	"strconv"
	"strings"

	"github.com/santhosh-tekuri/jsonschema/v6"
)

// MaxSchemaBytes is the largest schema document Compile accepts: 1 MiB.
// The largest legitimate schema documents in the repo (the vendored
// mcpspec protocol schemas) are under 200 KiB, so this is >5x headroom
// while still bounding a hostile schema.
const MaxSchemaBytes = 1048576 // 1 MiB

// baseURL is the synthetic location every schema is registered under.
// It exists only inside the compiler; nothing is ever fetched from it
// (or from anywhere else — see offlineLoader).
const baseURL = "schemaval:///schema.json"

// Dialect names Compile reports via Schema.Dialect.
const (
	Dialect2020_12 = "2020-12"
	Dialect2019_09 = "2019-09"
	DialectDraft07 = "draft-07"
	DialectDraft06 = "draft-06"
	DialectDraft04 = "draft-04"
)

// dialects maps a normalized $schema URL (scheme ignored, trailing '#'
// stripped) to its dialect name.
var dialects = map[string]string{
	"json-schema.org/draft/2020-12/schema": Dialect2020_12,
	"json-schema.org/draft/2019-09/schema": Dialect2019_09,
	"json-schema.org/draft-07/schema":      DialectDraft07,
	"json-schema.org/draft-06/schema":      DialectDraft06,
	"json-schema.org/draft-04/schema":      DialectDraft04,
}

// UnsupportedDialectError reports a declared $schema dialect outside the
// supported set. It is deliberately distinct from a generic compile
// error so callers can tell "this schema speaks a dialect we refuse"
// apart from "this schema is malformed".
type UnsupportedDialectError struct {
	Dialect string // the $schema value as declared
}

func (e *UnsupportedDialectError) Error() string {
	return fmt.Sprintf("schemaval: unsupported dialect %q (supported: 2020-12, 2019-09, draft-07, draft-06, draft-04)", e.Dialect)
}

// UnresolvedRefError reports a $ref that cannot be resolved offline:
// either a reference into the schema document that does not exist, or
// any external reference (the loader never fetches).
type UnresolvedRefError struct {
	Ref string // the reference target that could not be resolved
	Err error  // the underlying compiler error
}

func (e *UnresolvedRefError) Error() string {
	return fmt.Sprintf("schemaval: unresolved $ref %q: %v", e.Ref, e.Err)
}

func (e *UnresolvedRefError) Unwrap() error { return e.Err }

// SchemaTooLargeError reports a schema document over MaxSchemaBytes.
type SchemaTooLargeError struct {
	Size  int
	Limit int
}

func (e *SchemaTooLargeError) Error() string {
	return fmt.Sprintf("schemaval: schema is %d bytes, over the %d-byte limit", e.Size, e.Limit)
}

// CompileError wraps any other failure to compile a schema (malformed
// JSON, a schema that fails its own metaschema, and so on).
type CompileError struct {
	Err error
}

func (e *CompileError) Error() string { return fmt.Sprintf("schemaval: compile: %v", e.Err) }
func (e *CompileError) Unwrap() error { return e.Err }

// Problem is one validation failure: where the instance broke the
// schema, and how.
type Problem struct {
	Path    string // JSONPath-style location, e.g. "$.count" or "$.items[2]"
	Message string // e.g. "got string, want integer"
}

func (p Problem) String() string { return p.Path + ": " + p.Message }

// ValidationError is the flattened, path-carrying form of a failed
// validation: one Problem per leaf failure, in document order.
type ValidationError struct {
	Problems []Problem
}

func (e *ValidationError) Error() string {
	parts := make([]string, len(e.Problems))
	for i, p := range e.Problems {
		parts[i] = p.String()
	}
	return strings.Join(parts, "; ")
}

// Schema is a compiled schema, safe for concurrent use.
type Schema struct {
	inner   *jsonschema.Schema
	dialect string
}

// Dialect reports the schema's dialect (the declared one, or 2020-12
// when the schema declares none).
func (s *Schema) Dialect() string { return s.dialect }

// errOffline is returned by offlineLoader for every URL.
var errOffline = errors.New("schemaval: loading external resources is disabled (no network)")

// offlineLoader is the compiler's only loader: it refuses everything.
// The standard dialect metaschemas are embedded in the jsonschema
// library and never reach this loader; anything that does reach it is
// an external resource, which v1 does not resolve.
type offlineLoader struct{}

func (offlineLoader) Load(string) (any, error) { return nil, errOffline }

// Compile compiles one schema document under the package policy.
// The document is decoded once; the result can validate any number of
// instances.
func Compile(raw json.RawMessage) (*Schema, error) {
	if len(raw) > MaxSchemaBytes {
		return nil, &SchemaTooLargeError{Size: len(raw), Limit: MaxSchemaBytes}
	}
	dialect, err := dialectOf(raw)
	if err != nil {
		return nil, err
	}
	doc, err := jsonschema.UnmarshalJSON(bytes.NewReader(raw))
	if err != nil {
		return nil, &CompileError{Err: err}
	}
	c := jsonschema.NewCompiler()
	c.UseLoader(offlineLoader{})
	if err := c.AddResource(baseURL, doc); err != nil {
		return nil, classifyCompileError(err)
	}
	// A schema that names itself with an absolute $id re-bases its
	// internal references at that $id; register the same document
	// there too so those references stay resolvable offline.
	if obj, ok := doc.(map[string]any); ok {
		if id, ok := obj["$id"].(string); ok && id != "" && id != baseURL {
			if err := c.AddResource(id, doc); err != nil {
				return nil, classifyCompileError(err)
			}
		}
	}
	sch, err := c.Compile(baseURL)
	if err != nil {
		return nil, classifyCompileError(err)
	}
	return &Schema{inner: sch, dialect: dialect}, nil
}

// dialectOf reads the top-level $schema declaration and applies the
// dialect policy. Boolean schemas and documents without $schema are
// 2020-12.
func dialectOf(raw json.RawMessage) (string, error) {
	var obj map[string]json.RawMessage
	if err := json.Unmarshal(raw, &obj); err != nil {
		// Not an object (a boolean schema, or malformed JSON — the
		// decode in Compile reports malformed JSON precisely).
		return Dialect2020_12, nil
	}
	decl, ok := obj["$schema"]
	if !ok {
		return Dialect2020_12, nil
	}
	var name string
	if err := json.Unmarshal(decl, &name); err != nil {
		return "", &UnsupportedDialectError{Dialect: string(decl)}
	}
	norm := strings.TrimSuffix(name, "#")
	if i := strings.Index(norm, "://"); i >= 0 {
		norm = norm[i+3:]
	}
	if d, ok := dialects[norm]; ok {
		return d, nil
	}
	return "", &UnsupportedDialectError{Dialect: name}
}

// classifyCompileError maps the compiler's reference-resolution
// failures onto UnresolvedRefError and wraps everything else.
func classifyCompileError(err error) error {
	var loadErr *jsonschema.LoadURLError
	if errors.As(err, &loadErr) {
		return &UnresolvedRefError{Ref: loadErr.URL, Err: err}
	}
	var ptrErr *jsonschema.JSONPointerNotFoundError
	if errors.As(err, &ptrErr) {
		return &UnresolvedRefError{Ref: ptrErr.URL, Err: err}
	}
	return &CompileError{Err: err}
}

// DecodeInstance decodes an instance document the way Validate expects:
// numbers keep full precision (json.Number), so integers past 2^53
// validate exactly.
func DecodeInstance(raw json.RawMessage) (any, error) {
	return jsonschema.UnmarshalJSON(bytes.NewReader(raw))
}

// Validate validates a decoded instance (see DecodeInstance; values
// decoded by encoding/json also work, with float64 precision limits).
// It returns nil for a valid instance, or a *ValidationError whose
// problems carry instance paths.
func (s *Schema) Validate(v any) error {
	err := s.inner.Validate(v)
	if err == nil {
		return nil
	}
	var ve *jsonschema.ValidationError
	if !errors.As(err, &ve) {
		return err
	}
	out := ve.DetailedOutput()
	// DetailedOutput is a tree, not a flat list: a unit that groups
	// sub-errors (every combinator failure — anyOf/oneOf/allOf/not —
	// and any failure under a grouping keyword) carries Errors but a
	// nil Error. Flatten by walking to the leaves; reading Error off
	// an interior unit panics (found by the mcpx conformance suite:
	// an anyOf argument failure crashed the daemon, #1483).
	problems := make([]Problem, 0, len(out.Errors))
	var walk func(u *jsonschema.OutputUnit)
	walk = func(u *jsonschema.OutputUnit) {
		if u.Error != nil {
			problems = append(problems, Problem{Path: jsonPath(u.InstanceLocation), Message: u.Error.String()})
		}
		for i := range u.Errors {
			walk(&u.Errors[i])
		}
	}
	walk(out)
	if len(problems) == 0 {
		problems = append(problems, Problem{Path: "$", Message: ve.Error()})
	}
	return &ValidationError{Problems: problems}
}

// ValidateJSON decodes raw with DecodeInstance and validates it.
func (s *Schema) ValidateJSON(raw json.RawMessage) error {
	v, err := DecodeInstance(raw)
	if err != nil {
		return fmt.Errorf("schemaval: decode instance: %w", err)
	}
	return s.Validate(v)
}

// jsonPath renders a JSON Pointer ("/outer/0/inner") in the flattened
// error form ("$.outer[0].inner"); the root is "$".
func jsonPath(ptr string) string {
	if ptr == "" || ptr == "/" {
		return "$"
	}
	var b strings.Builder
	b.WriteString("$")
	for _, tok := range strings.Split(strings.TrimPrefix(ptr, "/"), "/") {
		tok = strings.ReplaceAll(strings.ReplaceAll(tok, "~1", "/"), "~0", "~")
		if n, err := strconv.Atoi(tok); err == nil && strconv.Itoa(n) == tok {
			fmt.Fprintf(&b, "[%d]", n)
		} else if isSimpleKey(tok) {
			b.WriteString("." + tok)
		} else {
			fmt.Fprintf(&b, "[%q]", tok)
		}
	}
	return b.String()
}

func isSimpleKey(s string) bool {
	if s == "" {
		return false
	}
	for i, r := range s {
		if r == '_' || ('a' <= r && r <= 'z') || ('A' <= r && r <= 'Z') || (i > 0 && '0' <= r && r <= '9') {
			continue
		}
		return false
	}
	return true
}
