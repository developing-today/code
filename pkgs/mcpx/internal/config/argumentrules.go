package config

import (
	"bytes"
	"encoding/json"
	"fmt"
	"sort"
	"strconv"
	"strings"

	"github.com/dezren39/mcpx/internal/schemaval"
)

// Argument rules (#1986, Phase 2 of the write-policy design #2126):
// per-tool, per-argument call constraints written as JSON Schema. Where
// tools/excludeTools decide by tool name alone -- a tool is callable
// with any arguments or not at all -- an argument rule decides by the
// call itself: this tool, but never on branch main; this tool, but only
// ever private repositories in one org.
//
// A rule set is the union of the pool block's argumentRules and the
// server's own, across every layered config file. Rules only ever
// tighten: a call is allowed only if its arguments validate against
// EVERY rule whose key matches the tool name (allOf semantics), and no
// layer can remove or loosen a rule a farther layer set.

// ArgumentRule is one compiled argumentRules entry.
type ArgumentRule struct {
	// Key is the tool pattern as written: an exact name, a glob, or a
	// /regexp/, matched exactly as tools/excludeTools entries match.
	Key string
	// Origin names where the rule came from -- the config file, and
	// whether it sat in the pool block or a server's mcpx block -- so a
	// refusal can say whose rule refused the call.
	Origin string
	// Schema is the rule's JSON Schema, as written.
	Schema json.RawMessage

	pattern  ToolPatterns
	compiled *schemaval.Schema
}

// ArgumentRuleSet is a server's compiled argument rules, in a stable
// order (pool rules before server rules, keys sorted within each), so
// the first violation a call meets is deterministic.
type ArgumentRuleSet struct {
	rules []ArgumentRule
}

// ArgumentRuleSource is one uncompiled rule: where it was written, its
// pattern key, and its schema document.
type ArgumentRuleSource struct {
	Key    string
	Origin string
	Schema json.RawMessage
}

// CompileArgumentRules compiles sources into a set, failing on the
// first bad entry: a rule whose pattern is not a valid tool pattern
// (including a non-RE2 /regexp/) or whose schema schemaval refuses is
// a load-time config error, never a rule that silently matches nothing
// or validates nothing.
func CompileArgumentRules(sources []ArgumentRuleSource) (*ArgumentRuleSet, error) {
	set := &ArgumentRuleSet{}
	for _, src := range sources {
		pat, err := CompileToolPatterns([]string{src.Key})
		if err != nil {
			return nil, fmt.Errorf("argumentRules %q (%s): %w", src.Key, src.Origin, err)
		}
		compiled, err := schemaval.Compile(src.Schema)
		if err != nil {
			return nil, fmt.Errorf("argumentRules %q (%s): %w", src.Key, src.Origin, err)
		}
		set.rules = append(set.rules, ArgumentRule{
			Key: src.Key, Origin: src.Origin, Schema: src.Schema,
			pattern: pat, compiled: compiled,
		})
	}
	return set, nil
}

// Empty reports whether the set has no rules. A nil set is empty.
func (s *ArgumentRuleSet) Empty() bool { return s == nil || len(s.rules) == 0 }

// Rules returns the set's rules in evaluation order.
func (s *ArgumentRuleSet) Rules() []ArgumentRule {
	if s == nil {
		return nil
	}
	return append([]ArgumentRule(nil), s.rules...)
}

// Matching returns the rules whose key matches tool, in set order.
func (s *ArgumentRuleSet) Matching(tool string) []ArgumentRule {
	if s == nil {
		return nil
	}
	var out []ArgumentRule
	for _, r := range s.rules {
		if r.pattern.Match(tool) {
			out = append(out, r)
		}
	}
	return out
}

// ArgumentRuleViolation is one rule one call broke: the rule, the first
// validation problem (its path and the constraint it names), and the
// value the call actually sent at that path.
type ArgumentRuleViolation struct {
	Rule    ArgumentRule
	Problem schemaval.Problem
	Value   string
}

// Error renders the refusal in the HiddenTool style: the argument path,
// the sent value, the constraint it broke, and the rule's origin.
func (v *ArgumentRuleViolation) Error() string {
	return fmt.Sprintf("argument %s = %s violates %s (argument rule %q from %s)",
		v.Problem.Path, v.Value, v.Problem.Message, v.Rule.Key, v.Rule.Origin)
}

// Check validates args against every rule matching tool and returns
// the first violation, or nil when the call satisfies all of them.
// args is the call's argument value as the daemon holds it (usually a
// map decoded from JSON); it is re-decoded the way schemaval expects,
// so integers past float64 precision keep their exact value.
func (s *ArgumentRuleSet) Check(tool string, args any) *ArgumentRuleViolation {
	matching := s.Matching(tool)
	if len(matching) == 0 {
		return nil
	}
	raw, err := json.Marshal(args)
	if err == nil && string(raw) == "null" {
		// A caller can send "args": null (the daemon decodes it to a
		// nil map). Object keywords -- required, properties -- do not
		// reject JSON null, so the call would sail past rules written
		// exactly like the documented examples and only become the
		// empty object the client actually sends upstream afterwards.
		// Judge the value that will be sent: no arguments at all is
		// the empty object.
		raw = []byte("{}")
	}
	if err != nil {
		// Arguments that will not marshal cannot be validated; refuse
		// rather than wave an unchecked call through.
		return &ArgumentRuleViolation{
			Rule:    matching[0],
			Problem: schemaval.Problem{Path: "$", Message: "arguments could not be encoded for validation: " + err.Error()},
			Value:   fmt.Sprintf("%v", args),
		}
	}
	inst, err := schemaval.DecodeInstance(raw)
	if err != nil {
		return &ArgumentRuleViolation{
			Rule:    matching[0],
			Problem: schemaval.Problem{Path: "$", Message: "arguments could not be decoded for validation: " + err.Error()},
			Value:   string(raw),
		}
	}
	for _, r := range matching {
		verr := r.compiled.Validate(inst)
		if verr == nil {
			continue
		}
		ve, ok := verr.(*schemaval.ValidationError)
		if !ok || len(ve.Problems) == 0 {
			return &ArgumentRuleViolation{
				Rule:    r,
				Problem: schemaval.Problem{Path: "$", Message: verr.Error()},
				Value:   valueAt(raw, "$"),
			}
		}
		p := ve.Problems[0]
		return &ArgumentRuleViolation{Rule: r, Problem: p, Value: valueAt(raw, p.Path)}
	}
	return nil
}

// EffectiveSchema returns the inputSchema a surface should advertise
// for tool: the upstream schema with every matching rule folded in, so
// what a caller reads is the constraint the call will actually be
// judged against. With no matching rules the upstream schema is
// returned unchanged, byte for byte.
//
// The upstream schema stays at the root, never nested under allOf:
// the MCP schemas require inputSchema.type to be "object" at the root
// itself, header annotations (x-mcp-header) are only reachable where
// the upstream schema put them -- composition keywords hide them from
// mcpheaders.ToolParams -- and the upstream document's own local
// references keep resolving against the root they were written for.
// Each rule joins as an allOf member that is a bare $ref into a
// namespaced copy of the rule kept in the root $defs (see
// scopedSchemaMember), so a rule's local references keep their own
// scope no matter what the root already defines.
func (s *ArgumentRuleSet) EffectiveSchema(tool string, upstream json.RawMessage) json.RawMessage {
	matching := s.Matching(tool)
	if len(matching) == 0 {
		return upstream
	}
	root := map[string]any{}
	if len(upstream) > 0 {
		dec := json.NewDecoder(bytes.NewReader(upstream))
		dec.UseNumber()
		var doc any
		if err := dec.Decode(&doc); err != nil {
			return upstream
		}
		obj, ok := doc.(map[string]any)
		if !ok {
			// A non-object upstream schema cannot carry the root
			// this composition needs; advertise it unchanged
			// rather than a shape that misstates it.
			return upstream
		}
		root = obj
	}
	// The protocol's structural requirement, at the root where a
	// validating client looks for it.
	root["type"] = "object"
	defs, _ := root["$defs"].(map[string]any)
	if defs == nil {
		defs = map[string]any{}
	}
	var allOf []any
	if existing, ok := root["allOf"].([]any); ok {
		allOf = existing
	}
	for i, r := range matching {
		member, scoped := scopedSchemaMember(r.Schema, fmt.Sprintf("argumentrule%d", i))
		for name, def := range scoped {
			defs[name] = def
		}
		allOf = append(allOf, member)
	}
	if len(defs) > 0 {
		root["$defs"] = defs
	}
	root["allOf"] = allOf
	out, err := json.Marshal(root)
	if err != nil {
		return upstream
	}
	return out
}

// scopedSchemaMember prepares one schema document for composition
// into another document's allOf. The document is copied into the host
// document's $defs under "<prefix>$doc" and every local reference in
// the copy ("#", "#/$defs/...", any "#/..." pointer) is rewritten to
// resolve within that copy, so the copy means exactly what the
// document meant standalone. The returned member is a bare $ref to
// the copy; a document with no local references needs no copy and is
// returned as itself with no defs.
func scopedSchemaMember(raw json.RawMessage, prefix string) (member any, defs map[string]any) {
	dec := json.NewDecoder(bytes.NewReader(raw))
	dec.UseNumber()
	var doc any
	if err := dec.Decode(&doc); err != nil {
		return json.RawMessage(raw), nil
	}
	if !hasLocalRef(doc) {
		return doc, nil
	}
	name := prefix + "$doc"
	scoped := rewriteLocalRefs(doc, "#/$defs/"+name)
	return map[string]any{"$ref": "#/$defs/" + name}, map[string]any{name: scoped}
}

// hasLocalRef reports whether a decoded schema document contains a
// $ref pointing inside itself.
func hasLocalRef(v any) bool {
	switch t := v.(type) {
	case map[string]any:
		for k, val := range t {
			if k == "$ref" {
				if ref, ok := val.(string); ok && strings.HasPrefix(ref, "#") {
					return true
				}
			}
			if hasLocalRef(val) {
				return true
			}
		}
	case []any:
		for _, e := range t {
			if hasLocalRef(e) {
				return true
			}
		}
	}
	return false
}

// rewriteLocalRefs returns a copy of a decoded schema document with
// every local $ref re-based at base: "#" becomes base itself and
// "#/..." becomes base + "/...".
func rewriteLocalRefs(v any, base string) any {
	switch t := v.(type) {
	case map[string]any:
		out := make(map[string]any, len(t))
		for k, val := range t {
			if k == "$ref" {
				if ref, ok := val.(string); ok && strings.HasPrefix(ref, "#") {
					out[k] = base + strings.TrimPrefix(ref, "#")
					continue
				}
			}
			out[k] = rewriteLocalRefs(val, base)
		}
		return out
	case []any:
		out := make([]any, len(t))
		for i, e := range t {
			out[i] = rewriteLocalRefs(e, base)
		}
		return out
	default:
		return v
	}
}

// DeadRules returns the rules that match none of the known tool names:
// a rule keyed on a tool the server does not have constrains nothing,
// which is worth a warning where the catalog is known (at load and on
// catalog change) -- it is usually a typo.
func (s *ArgumentRuleSet) DeadRules(known []string) []ArgumentRule {
	if s == nil {
		return nil
	}
	var out []ArgumentRule
	for _, r := range s.rules {
		alive := false
		for _, name := range known {
			if r.pattern.Match(name) {
				alive = true
				break
			}
		}
		if !alive {
			out = append(out, r)
		}
	}
	return out
}

// argumentRuleSources flattens one argumentRules map into sources in
// key order, each stamped with the same origin.
func argumentRuleSources(m map[string]json.RawMessage, origin string) []ArgumentRuleSource {
	if len(m) == 0 {
		return nil
	}
	keys := make([]string, 0, len(m))
	for k := range m {
		keys = append(keys, k)
	}
	sort.Strings(keys)
	out := make([]ArgumentRuleSource, 0, len(keys))
	for _, k := range keys {
		out = append(out, ArgumentRuleSource{Key: k, Origin: origin, Schema: m[k]})
	}
	return out
}

// mergeArgumentRules unions the argumentRules of two layered Extras:
// the nearer file's rules and the farther file's rules both apply.
// When both files key a rule on the same pattern the two schemas are
// combined with allOf rather than one replacing the other -- a nearer
// file can tighten a farther rule, never loosen it.
func mergeArgumentRules(near, far map[string]json.RawMessage) map[string]json.RawMessage {
	if len(far) == 0 {
		return near
	}
	if len(near) == 0 {
		return far
	}
	out := make(map[string]json.RawMessage, len(near)+len(far))
	for k, v := range far {
		out[k] = v
	}
	for k, v := range near {
		if fv, dup := out[k]; dup {
			if combined := composeSchemas(fv, v); combined != nil {
				out[k] = combined
				continue
			}
		}
		out[k] = v
	}
	return out
}

// composeSchemas combines two layered schemas written for the same
// rule key into one document that means both (allOf), keeping each
// side's local references in their own scope via scopedSchemaMember:
// nesting the two documents under allOf without that moves their
// $defs but leaves "#/$defs/..." pointing at the combined root, and a
// rule that compiled standalone fails to compile merged.
func composeSchemas(far, near json.RawMessage) json.RawMessage {
	farMember, farDefs := scopedSchemaMember(far, "far")
	nearMember, nearDefs := scopedSchemaMember(near, "near")
	root := map[string]any{"allOf": []any{farMember, nearMember}}
	if len(farDefs)+len(nearDefs) > 0 {
		defs := make(map[string]any, len(farDefs)+len(nearDefs))
		for n, d := range farDefs {
			defs[n] = d
		}
		for n, d := range nearDefs {
			defs[n] = d
		}
		root["$defs"] = defs
	}
	out, err := json.Marshal(root)
	if err != nil {
		return nil
	}
	return out
}

// valueAt renders the value at a schemaval problem path ("$.a.b[0]")
// in the raw instance document, compactly, for a refusal message. A
// path that does not resolve (a missing required property, say)
// renders as "nothing".
func valueAt(raw json.RawMessage, path string) string {
	if path == "" || path == "$" {
		return compactJSON(raw)
	}
	dec := json.NewDecoder(bytes.NewReader(raw))
	dec.UseNumber()
	var doc any
	if err := dec.Decode(&doc); err != nil {
		return compactJSON(raw)
	}
	cur := doc
	rest := strings.TrimPrefix(path, "$")
	for rest != "" {
		switch {
		case strings.HasPrefix(rest, "."):
			rest = rest[1:]
			end := strings.IndexAny(rest, ".[")
			var key string
			if end < 0 {
				key, rest = rest, ""
			} else {
				key, rest = rest[:end], rest[end:]
			}
			m, ok := cur.(map[string]any)
			if !ok {
				return "nothing"
			}
			cur, ok = m[key]
			if !ok {
				return "nothing"
			}
		case strings.HasPrefix(rest, "["):
			end := strings.IndexByte(rest, ']')
			if end < 0 {
				return "nothing"
			}
			tok := rest[1:end]
			rest = rest[end+1:]
			if strings.HasPrefix(tok, "\"") {
				var key string
				if err := json.Unmarshal([]byte(tok), &key); err != nil {
					return "nothing"
				}
				m, ok := cur.(map[string]any)
				if !ok {
					return "nothing"
				}
				cur, ok = m[key]
				if !ok {
					return "nothing"
				}
				continue
			}
			n, err := strconv.Atoi(tok)
			if err != nil {
				return "nothing"
			}
			arr, ok := cur.([]any)
			if !ok || n < 0 || n >= len(arr) {
				return "nothing"
			}
			cur = arr[n]
		default:
			return "nothing"
		}
	}
	b, err := json.Marshal(cur)
	if err != nil {
		return fmt.Sprintf("%v", cur)
	}
	return compactJSON(b)
}

// compactJSON renders raw on one line, truncated so a refusal message
// stays a message even when the value is a document.
func compactJSON(raw json.RawMessage) string {
	var buf bytes.Buffer
	if err := json.Compact(&buf, raw); err != nil {
		return string(raw)
	}
	s := buf.String()
	const max = 160
	if len(s) > max {
		return s[:max] + "..."
	}
	return s
}
