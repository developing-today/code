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
	return s.effectiveSchema(tool, upstream, nil)
}

// EffectiveSchemaPinned is EffectiveSchema for a tool reached through
// a declared tool view (#1933): the view's pinned arguments are
// merged over the caller's before rules are enforced, so a rule
// requirement on a pinned property is satisfied by the pin itself.
// Projecting the rule verbatim would advertise a requirement the
// caller cannot meet for a property the view pins (and usually
// hides): against an upstream additionalProperties:false, that is a
// schema no caller can satisfy. Pinned names are therefore dropped
// from the rules' required and dependentRequired lists in the
// advertised projection only; enforcement is unchanged.
func (s *ArgumentRuleSet) EffectiveSchemaPinned(tool string, upstream json.RawMessage, pinned map[string]any) json.RawMessage {
	set := make(map[string]bool, len(pinned))
	for name := range pinned {
		set[name] = true
	}
	return s.effectiveSchema(tool, upstream, set)
}

func (s *ArgumentRuleSet) effectiveSchema(tool string, upstream json.RawMessage, pinned map[string]bool) json.RawMessage {
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
		taken := make(map[string]bool, len(defs))
		for name := range defs {
			taken[name] = true
		}
		schema := r.Schema
		if len(pinned) > 0 {
			schema = projectPinnedRule(schema, pinned)
		}
		member, scoped := scopedSchemaMemberTaken(schema, fmt.Sprintf("argumentrule%d", i), taken)
		for name, def := range scoped {
			defs[name] = def
		}
		allOf = append(allOf, member)
	}
	// An upstream unevaluatedProperties at the root judges the
	// properties its OWN adjacent applicators evaluated. Left at
	// the root here, the rule members under allOf would count as
	// evaluators too, and a property a rule mentions would stop
	// being "unevaluated" for the upstream keyword -- the
	// composition would loosen the upstream schema it advertises
	// (Codex P2 on PR #2855, round 6). Move the keyword into a
	// boundary member carrying copies of the upstream root's own
	// property applicators: inside that member, adjacency is the
	// upstream document's own again, so the keyword keeps exactly
	// its standalone meaning while the root applies the upstream
	// schema and the rules as before.
	if up, present := root["unevaluatedProperties"]; present {
		delete(root, "unevaluatedProperties")
		boundary := map[string]any{"unevaluatedProperties": up}
		for _, k := range []string{
			"properties", "patternProperties", "additionalProperties",
			"dependentSchemas",
			"allOf", "anyOf", "oneOf", "not", "if", "then", "else",
			"$ref", "$dynamicRef",
		} {
			if v, ok := root[k]; ok {
				boundary[k] = stripAnchorDefs(v)
			}
		}
		allOf = append([]any{boundary}, allOf...)
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
// the copy ("#", "#/$defs/...", any "#/..." pointer, via $ref or
// $dynamicRef) is rewritten to resolve within that copy, so the copy
// means exactly what the document meant standalone. The returned
// member is a bare $ref to the copy; a document with no local
// references and no anchor definitions needs no copy and is
// returned as itself with no defs.
func scopedSchemaMember(raw json.RawMessage, prefix string) (member any, defs map[string]any) {
	return scopedSchemaMemberTaken(raw, prefix, nil)
}

// scopedSchemaMemberTaken is scopedSchemaMember with a set of $defs
// names the host document already uses. The scoped copy must never
// take one of those names: overwriting an upstream definition would
// silently retarget the upstream schema's own references to the rule
// document, changing (and potentially loosening) constraints the
// upstream schema already carried. When the generated name is taken,
// allocate the next free suffixed name and rewrite references to it
// consistently.
func scopedSchemaMemberTaken(raw json.RawMessage, prefix string, taken map[string]bool) (member any, defs map[string]any) {
	dec := json.NewDecoder(bytes.NewReader(raw))
	dec.UseNumber()
	var doc any
	if err := dec.Decode(&doc); err != nil {
		return json.RawMessage(raw), nil
	}
	anchors := anchorNames(doc)
	if !hasLocalRef(doc) && len(anchors) == 0 {
		return doc, nil
	}
	name := prefix + "$doc"
	for n := 2; taken[name]; n++ {
		name = fmt.Sprintf("%s$doc$%d", prefix, n)
	}
	// Anchors are document-scoped names, not pointers: two documents
	// copied into one host resource can each define the same $anchor
	// (two layered rules both anchoring "T"), and the host then has
	// a duplicate anchor -- a compile error, or one rule's "#T"
	// resolving to the other rule's target (Codex P2 on PR #2855,
	// round 6). Namespace every anchor this document defines with
	// the copy's own (collision-free) defs name, and rewrite the
	// document's references to them to match. Anchor names cannot
	// contain "$", so the defs name's "$"s become "_"s.
	renamed := make(map[string]string, len(anchors))
	if len(anchors) > 0 {
		base := strings.NewReplacer("$", "_").Replace(name) + "_"
		names := make([]string, 0, len(anchors))
		for a := range anchors {
			names = append(names, a)
		}
		sort.Strings(names)
		for _, a := range names {
			renamed[a] = base + a
		}
	}
	scoped := rewriteLocalRefs(doc, "#/$defs/"+name, renamed)
	return map[string]any{"$ref": "#/$defs/" + name}, map[string]any{name: scoped}
}

// anchorNames returns the $anchor/$dynamicAnchor names a decoded
// schema document defines within its own resource. A map carrying
// its own $id -- the document root included -- is a separate
// resource whose anchors the host never sees (the ref rewrite
// returns such subtrees verbatim), so the walk does not descend
// into it; literal payloads are instance data, not definitions.
func anchorNames(doc any) map[string]bool {
	out := map[string]bool{}
	collectAnchorNames(doc, out)
	return out
}

func collectAnchorNames(v any, out map[string]bool) {
	switch t := v.(type) {
	case map[string]any:
		if id, ok := t["$id"].(string); ok && id != "" {
			return
		}
		for k, val := range t {
			switch {
			case k == "$anchor" || k == "$dynamicAnchor":
				if s, ok := val.(string); ok && s != "" {
					out[s] = true
				}
			case literalValueKey(k):
				continue
			case nameMapKey(k):
				if m, ok := val.(map[string]any); ok {
					for _, sub := range m {
						collectAnchorNames(sub, out)
					}
				}
			default:
				collectAnchorNames(val, out)
			}
		}
	case []any:
		for _, e := range t {
			collectAnchorNames(e, out)
		}
	}
}

// stripAnchorDefs returns a copy of a decoded schema with every
// $anchor/$dynamicAnchor definition removed in schema position.
// EffectiveSchema's unevaluatedProperties boundary member duplicates
// the upstream root's applicators; the originals stay at the root,
// so a "#name" reference inside a copy still resolves to the
// identical definition there, while defining the same anchor twice
// in one resource would be a compile error. Subtrees under their
// own $id are separate resources and are returned verbatim, as in
// the ref traversals.
func stripAnchorDefs(v any) any {
	switch t := v.(type) {
	case map[string]any:
		if id, ok := t["$id"].(string); ok && id != "" {
			return v
		}
		out := make(map[string]any, len(t))
		for k, val := range t {
			if k == "$anchor" || k == "$dynamicAnchor" {
				continue
			}
			switch {
			case literalValueKey(k):
				out[k] = val
			case nameMapKey(k):
				if m, ok := val.(map[string]any); ok {
					nm := make(map[string]any, len(m))
					for n, sub := range m {
						nm[n] = stripAnchorDefs(sub)
					}
					out[k] = nm
				} else {
					out[k] = val
				}
			default:
				out[k] = stripAnchorDefs(val)
			}
		}
		return out
	case []any:
		out := make([]any, len(t))
		for i, e := range t {
			out[i] = stripAnchorDefs(e)
		}
		return out
	default:
		return v
	}
}

// literalValueKey reports whether a schema keyword's value is
// instance data rather than a schema. A const/enum/default/examples
// payload is a literal argument value: a "$ref" key inside it names
// data, not a reference. Ref discovery and rewriting must not
// descend into these values -- merging layers would otherwise
// rewrite the very values a rule permits into ones it forbids,
// loosening (and corrupting) the farther rule instead of only
// tightening it.
func literalValueKey(k string) bool {
	switch k {
	case "const", "enum", "default", "examples", "example":
		return true
	}
	return false
}

// nameMapKey reports whether a schema keyword's value is a map of
// NAME -> schema (properties, $defs, ...). Keys in such a map are
// property or definition names, never keywords: an argument named
// "default" or a definition named "enum" is a name whose schema must
// be traversed like any other, not a literal payload to skip.
func nameMapKey(k string) bool {
	switch k {
	case "properties", "patternProperties", "$defs", "definitions", "dependentSchemas":
		return true
	}
	return false
}

// The two traversals below walk a decoded schema in SCHEMA position.
// They are structure-aware in three ways a blind walk cannot be:
//
//   - literal payloads (literalValueKey) are instance data, not
//     schemas, and are never descended into;
//   - name maps (nameMapKey) flip the walk into name position, where
//     every value is a schema whatever its key spells;
//   - a subschema carrying its own $id is a separate resource:
//     references inside it resolve against that resource, so the
//     subtree is never re-based (rewrite returns it verbatim);
//   - plain-name fragments ("#T") are not JSON Pointers: they name
//     $anchors, and a scoped copy namespaces every anchor the
//     document defines (see scopedSchemaMemberTaken), rewriting
//     references to locally defined anchors to the namespaced
//     name; a fragment naming no local anchor is left alone.

// hasLocalRef reports whether a decoded schema document contains a
// $ref or $dynamicRef pointing inside itself.
func hasLocalRef(v any) bool {
	switch t := v.(type) {
	case map[string]any:
		for k, val := range t {
			if k == "$ref" || k == "$dynamicRef" {
				if ref, ok := val.(string); ok && strings.HasPrefix(ref, "#") {
					return true
				}
			}
			switch {
			case literalValueKey(k):
				continue
			case nameMapKey(k):
				if hasLocalRefInNames(val) {
					return true
				}
			default:
				if hasLocalRef(val) {
					return true
				}
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

// hasLocalRefInNames walks a name -> schema map: every value is a
// schema, whatever its key spells.
func hasLocalRefInNames(v any) bool {
	t, ok := v.(map[string]any)
	if !ok {
		return false
	}
	for _, sub := range t {
		if hasLocalRef(sub) {
			return true
		}
	}
	return false
}

// rewriteLocalRefs returns a copy of a decoded schema document with
// every local root-pointer $ref/$dynamicRef re-based at base: "#"
// becomes base itself and "#/..." becomes base + "/...". $anchor
// and $dynamicAnchor definitions are renamed per anchors (the
// document's own anchors mapped to their namespaced names), and
// plain-name fragment references to a renamed anchor are rewritten
// to match. The inside of any $id-carrying subschema is returned
// unchanged (see the traversal note above).
func rewriteLocalRefs(v any, base string, anchors map[string]string) any {
	switch t := v.(type) {
	case map[string]any:
		if id, ok := t["$id"].(string); ok && id != "" {
			return v // a separate resource: its refs are its own
		}
		out := make(map[string]any, len(t))
		for k, val := range t {
			if k == "$anchor" || k == "$dynamicAnchor" {
				if s, ok := val.(string); ok {
					if renamed, ok := anchors[s]; ok {
						out[k] = renamed
						continue
					}
				}
			}
			if k == "$ref" || k == "$dynamicRef" {
				if ref, ok := val.(string); ok {
					switch {
					case ref == "#" || strings.HasPrefix(ref, "#/"):
						out[k] = base + strings.TrimPrefix(ref, "#")
						continue
					case strings.HasPrefix(ref, "#"):
						if renamed, ok := anchors[ref[1:]]; ok {
							out[k] = "#" + renamed
							continue
						}
					}
				}
			}
			switch {
			case literalValueKey(k):
				out[k] = val // literal instance data, not a schema
			case nameMapKey(k):
				out[k] = rewriteLocalRefsInNames(val, base, anchors)
			default:
				out[k] = rewriteLocalRefs(val, base, anchors)
			}
		}
		return out
	case []any:
		out := make([]any, len(t))
		for i, e := range t {
			out[i] = rewriteLocalRefs(e, base, anchors)
		}
		return out
	default:
		return v
	}
}

// rewriteLocalRefsInNames rewrites a name -> schema map: every value
// is a schema, whatever its key spells.
func rewriteLocalRefsInNames(v any, base string, anchors map[string]string) any {
	t, ok := v.(map[string]any)
	if !ok {
		return v
	}
	out := make(map[string]any, len(t))
	for name, sub := range t {
		out[name] = rewriteLocalRefs(sub, base, anchors)
	}
	return out
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

// projectPinnedRule returns the rule schema as the pinned view
// advertises it. At the rule's own instance scope, a pinned property
// is removed from required lists (the pin itself satisfies it: pins
// merge before enforcement) and a dependentRequired entry triggered
// by a pinned property is promoted -- the pin always inserts the
// trigger before the rule is enforced, so the trigger's remaining
// dependencies are unconditional requirements of the caller, and the
// advertised schema must say so or it understates what enforcement
// demands. The projection follows same-instance applicators (allOf,
// anyOf, oneOf, not, if/then/else, dependentSchemas) but never
// crosses into a nested instance scope (properties, items, $defs,
// ...): a property there that shares a pinned name is a different
// property, which the top-level pin does not satisfy (Codex P2s on
// PR #2976). A schema that does not decode is returned unchanged:
// the projection is presentation, and a malformed rule keeps its
// verbatim meaning rather than a guessed one.
func projectPinnedRule(raw json.RawMessage, pinned map[string]bool) json.RawMessage {
	dec := json.NewDecoder(bytes.NewReader(raw))
	dec.UseNumber()
	var doc any
	if err := dec.Decode(&doc); err != nil {
		return raw
	}
	out, err := json.Marshal(stripPinnedRequirements(doc, pinned))
	if err != nil {
		return raw
	}
	return out
}

// stripPinnedRequirements projects one schema at its own instance
// scope; see projectPinnedRule for the contract.
func stripPinnedRequirements(v any, pinned map[string]bool) any {
	m, ok := v.(map[string]any)
	if !ok {
		return v
	}
	out := make(map[string]any, len(m))
	var promoted []string
	for k, val := range m {
		switch k {
		case "required":
			if names, ok := val.([]any); ok {
				if kept := filterPinnedNames(names, pinned); len(kept) > 0 {
					out[k] = kept
				}
				continue
			}
			out[k] = val
		case "dependentRequired":
			deps, ok := val.(map[string]any)
			if !ok {
				out[k] = val
				continue
			}
			keptDeps := map[string]any{}
			triggers := make([]string, 0, len(deps))
			for prop := range deps {
				triggers = append(triggers, prop)
			}
			sort.Strings(triggers)
			for _, prop := range triggers {
				depVal := deps[prop]
				names, ok := depVal.([]any)
				if !ok {
					keptDeps[prop] = depVal
					continue
				}
				kept := filterPinnedNames(names, pinned)
				if pinned[prop] {
					// The pin always supplies the trigger, so
					// its remaining dependencies promote to
					// unconditional requirements at this scope.
					for _, n := range kept {
						if name, ok := n.(string); ok {
							promoted = append(promoted, name)
						}
					}
					continue
				}
				if len(kept) > 0 {
					keptDeps[prop] = kept
				}
			}
			if len(keptDeps) > 0 {
				out[k] = keptDeps
			}
		case "allOf", "anyOf", "oneOf":
			if subs, ok := val.([]any); ok {
				projected := make([]any, len(subs))
				for i, sub := range subs {
					projected[i] = stripPinnedRequirements(sub, pinned)
				}
				out[k] = projected
			} else {
				out[k] = val
			}
		case "not", "if", "then", "else":
			out[k] = stripPinnedRequirements(val, pinned)
		case "dependentSchemas":
			if subs, ok := val.(map[string]any); ok {
				projected := make(map[string]any, len(subs))
				for prop, sub := range subs {
					projected[prop] = stripPinnedRequirements(sub, pinned)
				}
				out[k] = projected
			} else {
				out[k] = val
			}
		default:
			// Any other key is either not a schema position or
			// opens a nested instance scope: the pinned names
			// have no meaning there, and the value keeps its
			// verbatim meaning.
			out[k] = val
		}
	}
	if len(promoted) > 0 {
		required, _ := out["required"].([]any)
		seen := map[string]bool{}
		for _, n := range required {
			if name, ok := n.(string); ok {
				seen[name] = true
			}
		}
		for _, name := range promoted {
			if !seen[name] {
				seen[name] = true
				required = append(required, name)
			}
		}
		out["required"] = required
	}
	return out
}

func filterPinnedNames(names []any, pinned map[string]bool) []any {
	kept := make([]any, 0, len(names))
	for _, n := range names {
		if name, ok := n.(string); ok && pinned[name] {
			continue
		}
		kept = append(kept, n)
	}
	return kept
}
