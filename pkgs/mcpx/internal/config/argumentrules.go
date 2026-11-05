package config

import (
	"bytes"
	"encoding/json"
	"fmt"
	"regexp"
	"sort"
	"strconv"
	"strings"

	"github.com/developing-today/code/pkgs/mcpx/internal/schemaval"
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
	return s.effectiveSchema(tool, upstream, nil, nil)
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
// This compatibility form treats every pin as hidden; callers with
// view visibility metadata should use EffectiveSchemaPinnedWithHidden.
func (s *ArgumentRuleSet) EffectiveSchemaPinned(tool string, upstream json.RawMessage, pinned map[string]any) json.RawMessage {
	return s.effectiveSchema(tool, upstream, pinned, nil)
}

// EffectiveSchemaPinnedWithHidden is EffectiveSchemaPinned with the
// view's hidden pin names. A visible pin remains declared in a
// projected rule when adjacent additionalProperties would otherwise
// reject the caller's supplied value before the runtime merge replaces
// it; hidden pins stay absent from the caller-facing rule.
func (s *ArgumentRuleSet) EffectiveSchemaPinnedWithHidden(tool string, upstream json.RawMessage, pinned map[string]any, hidden []string) json.RawMessage {
	visible := make(map[string]bool, len(pinned))
	for name := range pinned {
		visible[name] = true
	}
	for _, name := range hidden {
		delete(visible, name)
	}
	return s.effectiveSchema(tool, upstream, pinned, visible)
}

func (s *ArgumentRuleSet) effectiveSchema(tool string, upstream json.RawMessage, pinned map[string]any, visiblePins map[string]bool) json.RawMessage {
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
			schema = projectPinnedRuleWithVisiblePins(schema, pinned, visiblePins)
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
// demands. The same promotion applies to dependentSchemas entries
// and, under a draft-07 dialect, to both forms of their predecessor
// keyword dependencies. The projection follows same-instance
// applicators (allOf, anyOf, oneOf, not, if/then/else,
// dependentSchemas) but never
// crosses into a nested instance scope (properties, items, $defs,
// ...): a property there that shares a pinned name is a different
// property, which the top-level pin does not satisfy (Codex P2s on
// PR #2976). A schema that does not decode is returned unchanged:
// the projection is presentation, and a malformed rule keeps its
// verbatim meaning rather than a guessed one.
func projectPinnedRule(raw json.RawMessage, pinned map[string]any) json.RawMessage {
	return projectPinnedRuleWithVisiblePins(raw, pinned, nil)
}

func projectPinnedRuleWithVisiblePins(raw json.RawMessage, pinned map[string]any, visiblePins map[string]bool) json.RawMessage {
	dec := json.NewDecoder(bytes.NewReader(raw))
	dec.UseNumber()
	var doc any
	if err := dec.Decode(&doc); err != nil {
		return raw
	}
	out, err := json.Marshal(stripPinnedRequirementsWithVisiblePins(doc, pinned, visiblePins))
	if err != nil {
		return raw
	}
	return out
}

func stripPinnedRequirementsWithVisiblePins(v any, pinned map[string]any, visiblePins map[string]bool) any {
	root, _ := v.(map[string]any)
	set := make(map[string]bool, len(pinned))
	for name := range pinned {
		set[name] = true
	}
	return stripPinnedScopeWithVisiblePins(v, set, pinned, visiblePins, root, map[string]bool{})
}

// conjoinRefProjection combines the projections of a $ref target and
// its sibling schema as conjuncts under allOf. $defs is inert without
// a reference and other schemas may still point into it by
// document-absolute pointer, so it stays at the level where the
// reference sat rather than inside the conjunction. The legacy
// definitions container rides along the same way: a modern-dialect
// rule may still declare one, and a reference like
// "#/definitions/options" is document-absolute wherever it sits --
// leaving the container inside the allOf member strands every such
// reference (Codex P2 on PR #2976). With no sibling constraints
// left, the target alone is the projection.
func conjoinRefProjection(target, siblings any) any {
	sm, ok := siblings.(map[string]any)
	if !ok {
		return target
	}
	defs, hasDefs := sm["$defs"]
	delete(sm, "$defs")
	legacy, hasLegacy := sm["definitions"]
	delete(sm, "definitions")
	tm, tok := target.(map[string]any)
	if len(sm) == 0 {
		if tok {
			if hasDefs {
				if _, clash := tm["$defs"]; !clash {
					tm["$defs"] = defs
				}
			}
			if hasLegacy {
				if _, clash := tm["definitions"]; !clash {
					tm["definitions"] = legacy
				}
			}
		}
		return target
	}
	out := map[string]any{"allOf": []any{target, sm}}
	if hasDefs {
		out["$defs"] = defs
	}
	if hasLegacy {
		out["definitions"] = legacy
	}
	return out
}

// stripPinnedScopeWithVisiblePins projects one scope with the rule
// document's root available for resolving local references and a set
// of references currently being resolved, so a self-referential
// definition cannot recurse forever. A local $ref applies its target
// at the instance scope where the reference sits -- a definition
// referenced from the root is a root constraint, not a nested scope
// -- so a resolvable reference is projected together with its siblings
// as a same-instance conjunction (Codex P2 on PR #2976).
func stripPinnedScopeWithVisiblePins(v any, pinned map[string]bool, pinVals map[string]any, visiblePins map[string]bool, root map[string]any, resolving map[string]bool) any {
	return stripPinnedScopeResAtWithVisiblePins(v, pinned, pinVals, visiblePins, root, root, resolving, true)
}

// stripPinnedScopeResWithVisiblePins also tracks the enclosing resource
// root separately: a subschema declaring its own
// $id is a separate resource, and fragment references inside it
// resolve against that resource's own definitions, not the outer
// document's (Codex P2 on PR #2976). resRoot is the root of the
// resource the traversal is currently inside; root remains the
// rule document's root for the dialect and verdict helpers, which
// decline $id-carrying shapes on their own.
func stripPinnedScopeResWithVisiblePins(v any, pinned map[string]bool, pinVals map[string]any, visiblePins map[string]bool, root map[string]any, resRoot map[string]any, resolving map[string]bool) any {
	return stripPinnedScopeResAtWithVisiblePins(v, pinned, pinVals, visiblePins, root, resRoot, resolving, false)
}

// stripPinnedScopeResAtWithVisiblePins adds vIsResRoot: whether v is the root of
// its own schema resource (the document root, or a subschema whose
// $id makes it one). Only at a resource root do fragment references
// like "#/properties/<name>" point at this scope's own properties,
// so only there can dropping a pinned declaration strand one.
func stripPinnedScopeResAtWithVisiblePins(v any, pinned map[string]bool, pinVals map[string]any, visiblePins map[string]bool, root map[string]any, resRoot map[string]any, resolving map[string]bool, vIsResRoot bool) any {
	m, ok := v.(map[string]any)
	if !ok {
		return v
	}
	atResRoot := vIsResRoot
	if id, isStr := m["$id"].(string); isStr && id != "" {
		resRoot = m
		atResRoot = true
	}
	// Verdicts about subschemas of this scope evaluate references
	// against the current resource (verdictRoot) while preserving
	// the rule document's dialect: a combinator inside an $id
	// resource must not have its branches decided against the outer
	// document's definitions (Codex P2 on PR #2976).
	verdictRoot := make(map[string]any, len(resRoot)+1)
	for key, value := range resRoot {
		verdictRoot[key] = value
	}
	if dialect, ok := root["$schema"]; ok {
		verdictRoot["$schema"] = dialect
	}
	resKey := func(ref string) string {
		if id, isStr := resRoot["$id"].(string); isStr && id != "" {
			return id + "|" + ref
		}
		return ref
	}
	if ref, ok := m["$ref"].(string); ok && strings.HasPrefix(ref, "#") && !resolving[resKey(ref)] {
		if target, ok := resolveLocalPointer(resRoot, ref); ok {
			if dialectIgnoresRefSiblings(root) {
				// Draft-07 and earlier: the reference replaces
				// this schema outright and its siblings are
				// ignored at enforcement, so the projection is
				// the projected target alone -- conjoining the
				// siblings advertises requirements enforcement
				// does not hold (Codex P2 on PR #2976). This
				// covers boolean targets too: stripPinnedScope
				// returns a non-object target unchanged. The
				// document's definition containers are not
				// constraints, though: the target's own nested
				// references resolve against them, so they ride
				// along verbatim when the projected target does
				// not carry its own (Codex P2 on PR #2976).
				resolving[resKey(ref)] = true
				projected := stripPinnedScopeResWithVisiblePins(target, pinned, pinVals, visiblePins, root, resRoot, resolving)
				delete(resolving, resKey(ref))
				if pm, isMap := projected.(map[string]any); isMap {
					for _, defsKey := range []string{"$defs", "definitions"} {
						if _, carried := pm[defsKey]; !carried {
							if defs, ok := m[defsKey]; ok {
								pm[defsKey] = defs
							}
						}
					}
					// The dialect rides along the same way: it is
					// not an assertion sibling draft-07 ignores,
					// it is the language the projected constraints
					// are written in. Dropping it leaves draft-07
					// keywords (dependencies) to be read under the
					// upstream schema's dialect, where they are
					// not keywords at all, so the advertised
					// schema accepts calls enforcement rejects
					// (Codex P2 on PR #2976).
					if _, carried := pm["$schema"]; !carried {
						if dialect, ok := m["$schema"]; ok {
							pm["$schema"] = dialect
						}
					}
				}
				return projected
			}
			if tm, ok := target.(map[string]any); ok {
				// A $ref and its siblings are a conjunction, not a
				// merge: copying sibling keys over the target's drops
				// constraints both sides carry (a sibling "required"
				// replacing the referenced list, which the projection
				// then strips as pinned, advertises a schema that
				// accepts calls enforcement rejects). Project each
				// side on its own and conjoin them (Codex P2 on
				// PR #2976).
				siblings := make(map[string]any, len(m)-1)
				for k, val := range m {
					if k != "$ref" {
						siblings[k] = val
					}
				}
				resolving[resKey(ref)] = true
				projectedTarget := stripPinnedScopeResWithVisiblePins(tm, pinned, pinVals, visiblePins, root, resRoot, resolving)
				projectedSiblings := stripPinnedScopeResWithVisiblePins(siblings, pinned, pinVals, visiblePins, root, resRoot, resolving)
				delete(resolving, resKey(ref))
				return conjoinRefProjection(projectedTarget, projectedSiblings)
			}
			// A non-object target (a boolean schema) has no scope of
			// its own to project, but its siblings are still
			// same-instance constraints: dropping them advertises a
			// schema enforcement does not hold. Conjoin them the
			// same way when any exist.
			siblings := make(map[string]any, len(m)-1)
			for k, val := range m {
				if k != "$ref" {
					siblings[k] = val
				}
			}
			if len(siblings) == 0 {
				return target
			}
			resolving[resKey(ref)] = true
			projectedSiblings := stripPinnedScopeResWithVisiblePins(siblings, pinned, pinVals, visiblePins, root, resRoot, resolving)
			delete(resolving, resKey(ref))
			return conjoinRefProjection(target, projectedSiblings)
		}
	}
	// A condition the pins decide outright must not survive as a
	// condition: projecting it by name-stripping alone removes the
	// pinned requirement that made it decidable, so the advertised
	// "if" evaluates against a caller who cannot supply the pinned
	// property and picks the branch enforcement -- which merges the
	// pin first -- does not (Codex P2 on PR #2976). Fold the decided
	// branch in as an unconditional conjunct instead.
	var preservedConditionals []preservedBranch
	if _, hasIf := m["if"]; hasIf {
		if branch, decided := pinDecidedBranch(m, pinned, pinVals, verdictRoot, resolving); decided {
			if atResRoot {
				for _, key := range []string{"if", "then", "else"} {
					if value, exists := m[key]; exists {
						pointer := "#/" + key
						if schemaPointerReferenced(m, pointer) {
							// Folding removes this original location. Keep its
							// schema in the resource definitions for other
							// references before the selected branch moves
							// into allOf (Codex P2 on PR #2976).
							preservedConditionals = append(preservedConditionals, preservedBranch{
								pointer: pointer,
								value:   cloneSchemaValue(value),
							})
						}
					}
				}
			}
			folded := make(map[string]any, len(m))
			for k, val := range m {
				if k != "if" && k != "then" && k != "else" {
					folded[k] = val
				}
			}
			if branch != nil {
				if subs, ok := folded["allOf"].([]any); ok {
					folded["allOf"] = append(append([]any{}, subs...), branch)
				} else {
					folded["allOf"] = []any{branch}
				}
			}
			m = folded
		}
	}
	// Every pin merges into the instance at this scope whether or
	// not the scope declares the property. Property-count
	// constraints are projected per pin below: a pin the caller
	// did not supply adds one to the merged count, while a pin
	// the caller supplied itself is overwritten by the merge and
	// adds nothing (Codex P2 on PR #2976).
	pinnedCount := len(pinned)
	out := make(map[string]any, len(m))
	var promoted []string
	var promotedSubs []any
	var preserved map[string]any
	preservedBranches := preservedConditionals
	var branchRemaps []branchRemap
	var unsatisfiable bool
	for k, val := range m {
		switch k {
		case "properties":
			props, ok := val.(map[string]any)
			if !ok {
				out[k] = val
				continue
			}
			kept := make(map[string]any, len(props))
			for name, sub := range props {
				if pinned[name] {
					predicate := map[string]any{"properties": map[string]any{name: sub}}
					if valid, decided := pinnedSchemaVerdict(predicate, pinned, pinVals, verdictRoot, resolving); decided {
						if !valid {
							unsatisfiable = true
						} else {
							if visiblePins[name] {
								if _, hasAdditionalProperties := m["additionalProperties"]; hasAdditionalProperties {
									// A visible pin remains caller-addressable. Keep
									// an unconstrained declaration beside
									// additionalProperties so the caller's raw
									// value is not rejected before the runtime
									// merge overwrites it with the validated pin.
									kept[name] = true
								}
							}
							if atResRoot && schemaPointerReferenced(m, "#/properties/"+escapePointerSegment(name)) {
								// Another schema in this resource
								// references this declaration (a
								// property whose schema is a $ref to
								// it): dropping it would leave the
								// advertised schema with an
								// unresolved reference, so the
								// declaration is preserved under
								// $defs below (Codex P2 on
								// PR #2976).
								if preserved == nil {
									preserved = map[string]any{}
								}
								preserved[name] = sub
							}
						}
						continue
					}
				}
				kept[name] = sub
			}
			if len(kept) > 0 {
				out[k] = kept
			}
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
			subs, ok := val.([]any)
			if !ok {
				out[k] = val
				continue
			}
			// A branch the pins decide outright must be counted by
			// its verdict, not name-stripped: stripping a oneOf
			// branch decided by a pinned discriminator leaves a
			// branch a caller who cannot supply the pin both
			// matches and cannot match as enforcement counts it,
			// so the advertised oneOf rejects calls the merged
			// arguments pass (Codex P2 on PR #2976). A decided
			// branch's truth depends only on the pins, so caller
			// input cannot change it: a valid one always matches,
			// an invalid one never does.
			kept := make([]any, 0, len(subs))
			keptIndices := make([]int, 0, len(subs))
			decidedValid := 0
			for i, sub := range subs {
				valid, decided := pinnedSchemaVerdict(sub, pinned, pinVals, verdictRoot, resolving)
				if !decided {
					// The branch's truth may still be
					// settled against the caller by its
					// pinned-only conjuncts: a oneOf
					// branch whose pinned discriminator
					// assertion fails can never match
					// the merged instance, even though
					// the branch also examines caller
					// properties the full verdict must
					// decline (Codex P2 on PR #2976).
					if cv, cd := pinnedConjunctVerdict(sub, pinned, pinVals, verdictRoot, resolving); cd && !cv {
						valid, decided = false, true
					}
				}
				if decided {
					if valid {
						decidedValid++
					} else if k == "allOf" {
						unsatisfiable = true
					}
					if atResRoot {
						pointer := "#/" + k + "/" + strconv.Itoa(i)
						if schemaPointerReferenced(m, pointer) {
							// Another schema in
							// this resource
							// references into the
							// discarded branch:
							// the branch is kept
							// under $defs below
							// and the references
							// are rewritten
							// there, instead of
							// being left dangling
							// (Codex P2 on
							// PR #2976).
							preservedBranches = append(preservedBranches, preservedBranch{pointer: pointer, value: sub})
						}
					}
					continue
				}
				if atResRoot && len(kept) != i {
					branchRemaps = append(branchRemaps, branchRemap{key: k, oldIndex: i, newIndex: len(kept)})
				}
				kept = append(kept, stripPinnedScopeResWithVisiblePins(sub, pinned, pinVals, visiblePins, root, resRoot, resolving))
				keptIndices = append(keptIndices, i)
			}
			switch k {
			case "allOf":
				if len(kept) > 0 {
					out[k] = kept
				}
			case "anyOf":
				if decidedValid > 0 {
					// One branch always matches: the disjunction
					// constrains nothing.
					// Keep projected alternatives referenced elsewhere
					// in this resource before collapsing the choice.
					if atResRoot {
						for i, sub := range kept {
							pointer := "#/anyOf/" + strconv.Itoa(keptIndices[i])
							if schemaPointerReferenced(m, pointer) {
								preservedBranches = append(preservedBranches, preservedBranch{pointer: pointer, value: sub})
							}
						}
					}
				} else if len(kept) == 0 {
					unsatisfiable = true
				} else {
					out[k] = kept
				}
			case "oneOf":
				if decidedValid > 1 {
					// Two branches always match: exactly-one can
					// never hold.
					unsatisfiable = true
				} else if decidedValid == 1 {
					// The decided branch is the one match, so the
					// caller must match none of the rest.
					if len(kept) > 0 {
						promotedSubs = append(promotedSubs, map[string]any{
							"not": map[string]any{"anyOf": kept},
						})
					}
				} else if len(kept) == 0 {
					unsatisfiable = true
				} else {
					out[k] = kept
				}
			}
		case "minProperties", "maxProperties":
			if pinnedCount == 0 {
				out[k] = val
				continue
			}
			n, ok := schemaCountValue(val)
			if !ok {
				out[k] = val
				continue
			}
			// The merged instance's property count is the
			// caller's count plus the pins the caller did
			// not supply: a pin is not necessarily hidden,
			// and one the caller supplies itself is
			// overwritten by the merge, not added (Codex
			// P2 on PR #2976). The count is therefore
			// projected as a conjunct branching on which
			// pinned names the caller supplied, not by
			// subtracting every pin from the bound.
			switch conjunct := pinnedCountConjunct(k, n, sortedPinnedNames(pinned)); {
			case conjunct == true:
				// Every caller instance satisfies the
				// merged count: no constraint survives.
			case conjunct == false:
				// The pins alone exceed the bound: no
				// caller input can satisfy this scope.
				unsatisfiable = true
			default:
				promotedSubs = append(promotedSubs, conjunct)
			}
		case "additionalProperties":
			// Rules run against the merged arguments. A pin that is
			// not covered by this scope's properties or
			// patternProperties is therefore tested by
			// additionalProperties even when it is hidden from the
			// caller-facing schema (Codex P2 on PR #2976).
			if len(pinVals) > 0 {
				for name, value := range pinVals {
					if scopeDeclaresProperty(m, name) {
						continue
					}
					if valid, decided := pinnedValueVerdict(val, value, root); decided && !valid {
						unsatisfiable = true
					}
				}
			}
			out[k] = val
		case "not":
			// A negation inverts the effect of the schemas inside
			// it: stripping a pinned requirement there does not
			// relax the advertised schema, it tightens it -- the
			// stripped inner schema succeeds vacuously for a
			// caller who cannot supply the hidden pin, so the
			// negation rejects calls enforcement accepts (Codex
			// P2 on PR #2976). Evaluate the pinned contribution
			// instead: when the pins decide the inner schema, the
			// negation's outcome is fixed -- drop it when it
			// always passes, keep it as not:true when it always
			// fails. When the pins do not decide it, project the
			// inner schema (below) rather than keeping it verbatim.
			if valid, decided := pinnedSchemaVerdict(val, pinned, pinVals, verdictRoot, resolving); decided {
				if valid {
					out[k] = true
				}
			} else {
				// The pins do not decide the negation, but their
				// contribution is still fixed: a mixed predicate
				// (a required list naming both a pinned and a
				// caller property) must have the pinned names
				// projected out, or the advertised negation
				// accepts caller arguments enforcement -- which
				// merges the pins first -- rejects (Codex P2 on
				// PR #2976). Project the inner schema with the
				// same equivalence-preserving projection used
				// elsewhere; parts the projection cannot decide
				// keep their verbatim meaning inside it.
				out[k] = stripPinnedScopeResWithVisiblePins(val, pinned, pinVals, visiblePins, root, resRoot, resolving)
			}
		case "if", "then", "else":
			out[k] = stripPinnedScopeResWithVisiblePins(val, pinned, pinVals, visiblePins, root, resRoot, resolving)
		case "dependencies":
			// The draft-07 predecessor of dependentRequired /
			// dependentSchemas: under a pre-2019-09 dialect it
			// constrains the instance in both of its forms -- an
			// array of names (the dependentRequired form) or a
			// subschema (the dependentSchemas form) -- and the
			// projection must treat a pinned trigger exactly like
			// its modern successors, promoting what the pin always
			// activates to unconditional constraints at this
			// scope. Leaving it verbatim advertises {} as valid
			// for a rule whose pinned trigger demands more, while
			// enforcement inserts the pin and rejects the call
			// (Codex P2 on PR #2976). Under a modern dialect the
			// keyword does not exist, so the value is inert data
			// and keeps its verbatim meaning.
			if !dialectIgnoresRefSiblings(root) {
				out[k] = val
				continue
			}
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
				switch dep := depVal.(type) {
				case []any:
					kept := filterPinnedNames(dep, pinned)
					if pinned[prop] {
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
				case map[string]any, bool:
					sub := stripPinnedScopeResWithVisiblePins(depVal, pinned, pinVals, visiblePins, root, resRoot, resolving)
					if pinned[prop] {
						if pm, isMap := sub.(map[string]any); isMap && len(pm) == 0 {
							continue
						}
						if b, isBool := sub.(bool); isBool && b {
							continue
						}
						promotedSubs = append(promotedSubs, sub)
						continue
					}
					keptDeps[prop] = sub
				default:
					keptDeps[prop] = depVal
				}
			}
			if len(keptDeps) > 0 {
				out[k] = keptDeps
			}
		case "dependentSchemas":
			if subs, ok := val.(map[string]any); ok {
				projected := make(map[string]any, len(subs))
				triggers := make([]string, 0, len(subs))
				for prop := range subs {
					triggers = append(triggers, prop)
				}
				sort.Strings(triggers)
				for _, prop := range triggers {
					sub := stripPinnedScopeResWithVisiblePins(subs[prop], pinned, pinVals, visiblePins, root, resRoot, resolving)
					if pinned[prop] {
						// The pin always supplies the trigger, so
						// the projected dependent schema applies
						// unconditionally at this scope: promote it
						// into allOf, analogous to the pinned
						// dependentRequired promotion above. An
						// empty or true projection constrains
						// nothing and is dropped.
						if pm, isMap := sub.(map[string]any); isMap && len(pm) == 0 {
							continue
						}
						if b, isBool := sub.(bool); isBool && b {
							continue
						}
						promotedSubs = append(promotedSubs, sub)
						continue
					}
					projected[prop] = sub
				}
				if len(projected) > 0 {
					out[k] = projected
				}
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
	if len(promotedSubs) > 0 {
		switch existing := out["allOf"].(type) {
		case []any:
			out["allOf"] = append(existing, promotedSubs...)
		case nil:
			out["allOf"] = promotedSubs
		default:
			wrapped := make([]any, 0, len(promotedSubs)+1)
			wrapped = append(wrapped, existing)
			out["allOf"] = append(wrapped, promotedSubs...)
		}
	}
	if len(preserved) > 0 || len(preservedBranches) > 0 {
		defs := map[string]any{}
		usable := true
		if existing, present := out["$defs"]; present {
			em, isMap := existing.(map[string]any)
			if !isMap {
				usable = false
			} else {
				for k2, v2 := range em {
					defs[k2] = v2
				}
			}
		}
		if usable {
			names := make([]string, 0, len(preserved))
			for name := range preserved {
				names = append(names, name)
			}
			sort.Strings(names)
			type refRewrite struct{ oldRef, newRef string }
			var rewrites []refRewrite
			for _, name := range names {
				defName := "_mcpx_pinned_" + name
				for i := 1; ; i++ {
					if _, exists := defs[defName]; !exists {
						break
					}
					defName = fmt.Sprintf("_mcpx_pinned_%s_%d", name, i)
				}
				defs[defName] = preserved[name]
				rewrites = append(rewrites, refRewrite{
					"#/properties/" + escapePointerSegment(name),
					"#/$defs/" + escapePointerSegment(defName),
				})
			}
			out["$defs"] = defs
			for _, pb := range preservedBranches {
				base := "_mcpx_branch_" + strings.ReplaceAll(strings.TrimPrefix(pb.pointer, "#/"), "/", "_")
				defName := base
				for i := 1; ; i++ {
					if _, exists := defs[defName]; !exists {
						break
					}
					defName = fmt.Sprintf("%s_%d", base, i)
				}
				defs[defName] = pb.value
				rewrites = append(rewrites, refRewrite{pb.pointer, "#/$defs/" + escapePointerSegment(defName)})
			}
			for _, rw := range rewrites {
				rewriteArgumentRefs(out, true, rw.oldRef, rw.newRef)
			}
		}
	}
	// References to kept applicator branches use their original
	// indices; dropping a decided branch shifts the survivors, so
	// surviving references are remapped to the new positions.
	// Remaps run in ascending original-index order: a remap's
	// target index is always vacated (by an earlier drop or an
	// earlier remap) before anything is rewritten into it.
	for _, rm := range branchRemaps {
		rewriteArgumentRefs(out, true, "#/"+rm.key+"/"+strconv.Itoa(rm.oldIndex), "#/"+rm.key+"/"+strconv.Itoa(rm.newIndex))
	}
	if unsatisfiable {
		// A pin-decided branch already settles this schema against
		// every caller: advertise that verdict directly instead of
		// a combinator no caller input can satisfy.
		out["not"] = true
	}
	return out
}

// schemaCountValue reads a property-count keyword value as written:
// the rule decoder produces json.Number, and a float64 from a
// hand-built document is accepted too.
func schemaCountValue(v any) (int64, bool) {
	switch n := v.(type) {
	case json.Number:
		i, err := n.Int64()
		return i, err == nil
	case float64:
		return int64(n), true
	}
	return 0, false
}

func escapePointerSegment(s string) string {
	s = strings.ReplaceAll(s, "~", "~0")
	return strings.ReplaceAll(s, "/", "~1")
}

// schemaPointerReferenced reports whether the resource rooted at m
// contains a local reference pointing at or below pointer. Nested
// resources are not entered: their fragments resolve against
// themselves.
func schemaPointerReferenced(m map[string]any, pointer string) bool {
	return schemaPointerReferencedAt(m, pointer, true)
}

func schemaPointerReferencedAt(v any, pointer string, isResourceRoot bool) bool {
	return schemaPointerReferencedPos(v, pointer, isResourceRoot, false)
}

// schemaPointerReferencedPos adds nameMap: inside a name map every
// key is a name and every value a schema, so the literal-payload
// skip does not apply to its entries.
func schemaPointerReferencedPos(v any, pointer string, isResourceRoot, nameMap bool) bool {
	switch v := v.(type) {
	case map[string]any:
		if !nameMap {
			if !isResourceRoot {
				if id, ok := v["$id"].(string); ok && id != "" {
					return false
				}
			}
			if ref, ok := v["$ref"].(string); ok && (ref == pointer || strings.HasPrefix(ref, pointer+"/")) {
				return true
			}
		}
		for k2, val := range v {
			if !nameMap && literalValueKey(k2) {
				continue
			}
			if schemaPointerReferencedPos(val, pointer, false, !nameMap && nameMapKey(k2)) {
				return true
			}
		}
	case []any:
		for _, val := range v {
			if schemaPointerReferencedPos(val, pointer, false, false) {
				return true
			}
		}
	}
	return false
}

// rewriteArgumentRefs rewrites every local reference in the
// resource rooted at v that points at or below oldRef so it points
// at or below newRef instead. Nested resources are not entered.
func rewriteArgumentRefs(v any, isResourceRoot bool, oldRef, newRef string) {
	rewriteArgumentRefsPos(v, isResourceRoot, false, oldRef, newRef)
}

// rewriteArgumentRefsPos adds nameMap: see schemaPointerReferencedPos.
func rewriteArgumentRefsPos(v any, isResourceRoot, nameMap bool, oldRef, newRef string) {
	switch v := v.(type) {
	case map[string]any:
		if !nameMap {
			if !isResourceRoot {
				if id, ok := v["$id"].(string); ok && id != "" {
					return
				}
			}
			if ref, ok := v["$ref"].(string); ok {
				if ref == oldRef {
					v["$ref"] = newRef
				} else if strings.HasPrefix(ref, oldRef+"/") {
					v["$ref"] = newRef + ref[len(oldRef):]
				}
			}
		}
		for k2, val := range v {
			if !nameMap && literalValueKey(k2) {
				continue
			}
			rewriteArgumentRefsPos(val, false, !nameMap && nameMapKey(k2), oldRef, newRef)
		}
	case []any:
		for _, val := range v {
			rewriteArgumentRefsPos(val, false, false, oldRef, newRef)
		}
	}
}

// pinDecidedBranch reports whether the map's "if" condition is
// decided by the pinned values alone, and if so returns the branch
// ("then" for true, "else" for false; nil when that branch is absent)
// the condition selects at enforcement time. A condition is decidable
// only when every property it examines is pinned -- the pin merge
// fixes those properties' values before enforcement, so the
// condition's truth is already determined -- and the condition
// evaluates cleanly against an instance carrying just the pins.
// Anything else (a condition reading a caller-supplied property, a
// whole-instance or name-pattern keyword, an unresolvable or
// recursive reference, a condition that fails to compile) is left
// for the ordinary name-stripping projection.
func pinDecidedBranch(m map[string]any, pinned map[string]bool, pinVals map[string]any, root map[string]any, resolving map[string]bool) (any, bool) {
	cond, ok := m["if"]
	if !ok {
		return nil, false
	}
	if b, isBool := cond.(bool); isBool {
		if b {
			return m["then"], true
		}
		return m["else"], true
	}
	valid, decided := pinnedSchemaVerdict(cond, pinned, pinVals, root, resolving)
	if !decided {
		return nil, false
	}
	if valid {
		return m["then"], true
	}
	return m["else"], true
}

// pinnedSchemaVerdict reports whether a schema's truth at its own
// instance scope is decided by the pinned values alone -- every
// property it examines is pinned and its shape is one whose truth
// depends only on those properties (see collectConditionNames) --
// and if so, whether an instance carrying the pins satisfies it.
// Pins merge before enforcement, so a decided verdict is the
// verdict enforcement reaches on the merged instance, whatever the
// caller adds.
func pinnedSchemaVerdict(schema any, pinned map[string]bool, pinVals map[string]any, root map[string]any, resolving map[string]bool) (bool, bool) {
	if b, isBool := schema.(bool); isBool {
		return b, true
	}
	names := map[string]bool{}
	if !collectConditionNames(schema, root, resolving, names) || len(names) == 0 {
		return false, false
	}
	inst := make(map[string]any, len(names))
	for name := range names {
		if !pinned[name] {
			return false, false
		}
		inst[name] = pinVals[name]
	}
	inlined, ok := inlineLocalRefs(schema, root, resolving, dialectIgnoresRefSiblings(root))
	if !ok {
		return false, false
	}
	// The detached condition compiles under the enclosing
	// document's dialect: a branch written for draft-07 uses
	// keywords later dialects dropped (dependencies is the
	// concrete case -- Codex P2 on PR #2976), and compiling it
	// as the default 2020-12 would silently ignore them and
	// report a verdict enforcement does not reach. A condition
	// declaring its own $schema keeps it.
	if im, isMap := inlined.(map[string]any); isMap {
		if _, declares := im["$schema"]; !declares {
			if dialect, isStr := root["$schema"].(string); isStr && dialect != "" {
				withDialect := make(map[string]any, len(im)+1)
				for k, val := range im {
					withDialect[k] = val
				}
				withDialect["$schema"] = dialect
				inlined = withDialect
			}
		}
	}
	condRaw, err := json.Marshal(inlined)
	if err != nil {
		return false, false
	}
	compiled, err := schemaval.Compile(condRaw)
	if err != nil {
		return false, false
	}
	instRaw, err := json.Marshal(inst)
	if err != nil {
		return false, false
	}
	dec, err := schemaval.DecodeInstance(instRaw)
	if err != nil {
		return false, false
	}
	return compiled.Validate(dec) == nil, true
}

// scopeDeclaresProperty mirrors the declaration check performed by
// additionalProperties at one schema object: only its adjacent
// properties and matching patternProperties exempt a name.
func scopeDeclaresProperty(scope map[string]any, name string) bool {
	if props, ok := scope["properties"].(map[string]any); ok {
		if _, exists := props[name]; exists {
			return true
		}
	}
	patterns, _ := scope["patternProperties"].(map[string]any)
	for pattern := range patterns {
		re, err := regexp.Compile(pattern)
		if err == nil && re.MatchString(name) {
			return true
		}
	}
	return false
}

// pinnedValueVerdict validates one fixed pin value against a schema
// applied to that property's value, resolving rule-local references
// and retaining the enclosing dialect where necessary.
func pinnedValueVerdict(schema, value any, root map[string]any) (bool, bool) {
	if b, ok := schema.(bool); ok {
		return b, true
	}
	inlined, ok := inlineLocalRefs(schema, root, map[string]bool{}, dialectIgnoresRefSiblings(root))
	if !ok {
		return false, false
	}
	if im, ok := inlined.(map[string]any); ok {
		if _, declares := im["$schema"]; !declares {
			if dialect, isString := root["$schema"].(string); isString && dialect != "" {
				withDialect := make(map[string]any, len(im)+1)
				for k, v := range im {
					withDialect[k] = v
				}
				withDialect["$schema"] = dialect
				inlined = withDialect
			}
		}
	}
	rawSchema, err := json.Marshal(inlined)
	if err != nil {
		return false, false
	}
	compiled, err := schemaval.Compile(rawSchema)
	if err != nil {
		return false, false
	}
	rawValue, err := json.Marshal(value)
	if err != nil {
		return false, false
	}
	instance, err := schemaval.DecodeInstance(rawValue)
	if err != nil {
		return false, false
	}
	return compiled.Validate(instance) == nil, true
}

// collectConditionNames gathers the property names a condition schema
// examines at its own instance scope: the keys of its properties, its
// required entries, and the triggers and dependencies of its
// dependent* keywords (and their draft-07 predecessor, dependencies),
// followed through the same-instance applicators
// (allOf/anyOf/oneOf/not/if/then/else, dependentSchemas values, local
// references). It reports false for shapes whose truth can depend on
// more than the examined properties' values -- whole-instance
// comparisons (const/enum), property counting, name patterns, or a
// reference that cannot be resolved -- so the caller treats those as
// undecidable. Nested instance scopes (a property's own subschema)
// are not entered: the validator judges those against the pin value
// itself.
func collectConditionNames(v any, root map[string]any, resolving map[string]bool, names map[string]bool) bool {
	m, ok := v.(map[string]any)
	if !ok {
		return true // a boolean subschema examines nothing
	}
	for k, val := range m {
		switch k {
		case "properties":
			props, ok := val.(map[string]any)
			if !ok {
				return false
			}
			for name := range props {
				names[name] = true
			}
		case "required":
			list, ok := val.([]any)
			if !ok {
				return false
			}
			for _, e := range list {
				name, ok := e.(string)
				if !ok {
					return false
				}
				names[name] = true
			}
		case "dependentRequired":
			deps, ok := val.(map[string]any)
			if !ok {
				return false
			}
			for trigger, depVal := range deps {
				names[trigger] = true
				list, ok := depVal.([]any)
				if !ok {
					return false
				}
				for _, e := range list {
					name, ok := e.(string)
					if !ok {
						return false
					}
					names[name] = true
				}
			}
		case "dependencies":
			// The draft-07 predecessor of dependentRequired /
			// dependentSchemas: under a draft-07 (or earlier)
			// dialect it constrains the instance exactly the
			// same way, so a condition using it examines the
			// trigger, the listed names (property form), or
			// whatever the schema form examines. Ignoring it
			// let a branch carrying a live dependency be
			// treated as decided by the pins alone and
			// discarded, advertising calls enforcement
			// rejects (Codex P2 on PR #2976).
			deps, ok := val.(map[string]any)
			if !ok {
				return false
			}
			for trigger, depVal := range deps {
				names[trigger] = true
				switch dep := depVal.(type) {
				case []any:
					for _, e := range dep {
						name, ok := e.(string)
						if !ok {
							return false
						}
						names[name] = true
					}
				case map[string]any:
					if !collectConditionNames(dep, root, resolving, names) {
						return false
					}
				case bool:
					// A boolean schema dependency examines
					// nothing beyond the trigger.
				default:
					return false
				}
			}
		case "dependentSchemas":
			subs, ok := val.(map[string]any)
			if !ok {
				return false
			}
			for trigger, sub := range subs {
				names[trigger] = true
				if !collectConditionNames(sub, root, resolving, names) {
					return false
				}
			}
		case "allOf", "anyOf", "oneOf":
			subs, ok := val.([]any)
			if !ok {
				return false
			}
			for _, sub := range subs {
				if !collectConditionNames(sub, root, resolving, names) {
					return false
				}
			}
		case "not", "if", "then", "else":
			if !collectConditionNames(val, root, resolving, names) {
				return false
			}
		case "$ref":
			ref, ok := val.(string)
			if !ok || resolving[ref] {
				return false
			}
			target, ok := resolveLocalPointer(root, ref)
			if !ok {
				return false
			}
			resolving[ref] = true
			ok = collectConditionNames(target, root, resolving, names)
			delete(resolving, ref)
			if !ok {
				return false
			}
		case "additionalProperties":
			if b, isBool := val.(bool); !isBool || !b {
				return false
			}
		case "patternProperties", "propertyNames", "unevaluatedProperties",
			"const", "enum", "minProperties", "maxProperties", "$dynamicRef":
			return false
		}
	}
	return true
}

// dialectIgnoresRefSiblings reports whether the document's
// declared dialect predates 2019-09: in draft-07 and earlier a
// $ref replaces its containing schema outright and its siblings
// are ignored, where 2019-09 and later evaluate the siblings
// alongside the reference. No $schema (or an unrecognized one)
// takes the modern reading.
func dialectIgnoresRefSiblings(root map[string]any) bool {
	dialect, ok := root["$schema"].(string)
	if !ok {
		return false
	}
	for _, old := range []string{"draft-04", "draft-06", "draft-07", "draft/4", "draft/6", "draft/7"} {
		if strings.Contains(dialect, old) {
			return true
		}
	}
	return false
}

// inlineLocalRefs returns a copy of a schema with every resolvable
// local $ref replaced by the conjunction of its target and its
// siblings, recursively, so the result stands alone for compilation.
// Under a dialect that ignores $ref siblings (draft-07 and
// earlier), the target alone replaces the reference, matching what
// enforcement evaluates (Codex P2 on PR #2976). It reports false
// when a reference cannot be resolved or is recursive.
func inlineLocalRefs(v any, root map[string]any, resolving map[string]bool, ignoreSiblings bool) (any, bool) {
	switch t := v.(type) {
	case map[string]any:
		// A subschema carrying its own $id is a separate
		// resource: references inside it resolve against that
		// resource, not the outer rule document this walk
		// resolves against. Substituting the outer document's
		// same-named definitions would evaluate a condition
		// with different semantics than enforcement, so the
		// shape is declined (the caller treats the condition
		// as undecided) — the same boundary rewriteLocalRefs
		// honours (Codex P2 on PR #2976).
		if id, ok := t["$id"].(string); ok && id != "" {
			return nil, false
		}
		if ref, ok := t["$ref"].(string); ok && strings.HasPrefix(ref, "#") {
			if resolving[ref] {
				return nil, false
			}
			target, ok := resolveLocalPointer(root, ref)
			if !ok {
				return nil, false
			}
			resolving[ref] = true
			inTarget, okT := inlineLocalRefs(target, root, resolving, ignoreSiblings)
			if ignoreSiblings {
				// The dialect ignores the siblings: the
				// reference alone is the schema enforcement
				// evaluates, so inlining the siblings as
				// conjuncts would judge a different --
				// stricter -- condition.
				delete(resolving, ref)
				if !okT {
					return nil, false
				}
				return inTarget, true
			}
			siblings := make(map[string]any, len(t))
			for k, val := range t {
				if k != "$ref" {
					siblings[k] = val
				}
			}
			inSiblings, okS := inlineLocalRefs(siblings, root, resolving, ignoreSiblings)
			delete(resolving, ref)
			if !okT || !okS {
				return nil, false
			}
			if sm, isMap := inSiblings.(map[string]any); isMap && len(sm) == 0 {
				return inTarget, true
			}
			return map[string]any{"allOf": []any{inTarget, inSiblings}}, true
		}
		out := make(map[string]any, len(t))
		for k, val := range t {
			// Only schema-valued positions are traversed: a
			// literal payload (const/enum/default/examples) is
			// instance data, and a "$ref" key inside it names
			// data, not a reference -- inlining there rewrites
			// the very value a condition compares against, so
			// the projection evaluates a different condition
			// than enforcement does (Codex P2 on PR #2976).
			// Name maps flip the walk into name position, where
			// every value is a schema whatever its key spells.
			switch {
			case literalValueKey(k):
				out[k] = val
			case nameMapKey(k):
				inlined, ok := inlineLocalRefsInNames(val, root, resolving, ignoreSiblings)
				if !ok {
					return nil, false
				}
				out[k] = inlined
			default:
				inlined, ok := inlineLocalRefs(val, root, resolving, ignoreSiblings)
				if !ok {
					return nil, false
				}
				out[k] = inlined
			}
		}
		return out, true
	case []any:
		out := make([]any, len(t))
		for i, e := range t {
			inlined, ok := inlineLocalRefs(e, root, resolving, ignoreSiblings)
			if !ok {
				return nil, false
			}
			out[i] = inlined
		}
		return out, true
	default:
		return v, true
	}
}

// inlineLocalRefsInNames inlines a name -> schema map: every value
// is a schema, whatever its key spells.
func inlineLocalRefsInNames(v any, root map[string]any, resolving map[string]bool, ignoreSiblings bool) (any, bool) {
	t, ok := v.(map[string]any)
	if !ok {
		return v, true
	}
	out := make(map[string]any, len(t))
	for name, sub := range t {
		inlined, ok := inlineLocalRefs(sub, root, resolving, ignoreSiblings)
		if !ok {
			return nil, false
		}
		out[name] = inlined
	}
	return out, true
}

// resolveLocalPointer resolves a local JSON Pointer reference ("#"
// or "#/a/b") against the document root. Plain-name fragments name
// $anchors, not pointers, and do not resolve here.
func resolveLocalPointer(root map[string]any, ref string) (any, bool) {
	if ref == "#" {
		return root, true
	}
	if !strings.HasPrefix(ref, "#/") {
		return nil, false
	}
	var cur any = root
	for _, seg := range strings.Split(ref[2:], "/") {
		seg = strings.ReplaceAll(strings.ReplaceAll(seg, "~1", "/"), "~0", "~")
		m, ok := cur.(map[string]any)
		if !ok {
			return nil, false
		}
		cur, ok = m[seg]
		if !ok {
			return nil, false
		}
	}
	return cur, true
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

// PinnedBranchVerdict exposes pinnedSchemaVerdict for the daemon's
// upstream projector, which must decide applicator branches against
// the same pin semantics before projecting them: a branch whose
// truth depends only on pinned properties (a oneOf discriminator on
// a pinned property, say) is decided by the pins, and projecting its
// assertions away would change how many branches a caller matches.
func PinnedBranchVerdict(schema any, pinVals map[string]any, root map[string]any) (bool, bool) {
	set := make(map[string]bool, len(pinVals))
	for name := range pinVals {
		set[name] = true
	}
	return pinnedSchemaVerdict(schema, set, pinVals, root, map[string]bool{})
}

// preservedBranch records an applicator branch the projection
// discarded (its truth was decided by the pins) that another schema
// in the same resource still references into: the branch is kept
// under $defs and the references rewritten to it.
type preservedBranch struct {
	pointer string
	value   any
}

// cloneSchemaValue detaches a preserved reference target from the
// projected branch that may be mutated during traversal.
func cloneSchemaValue(value any) any {
	switch value := value.(type) {
	case map[string]any:
		clone := make(map[string]any, len(value))
		for key, child := range value {
			clone[key] = cloneSchemaValue(child)
		}
		return clone
	case []any:
		clone := make([]any, len(value))
		for i, child := range value {
			clone[i] = cloneSchemaValue(child)
		}
		return clone
	default:
		return value
	}
}

// branchRemap records a kept applicator branch whose index in its
// applicator changed when an earlier branch was discarded, so
// references written against the original index can be rewritten.
type branchRemap struct {
	key      string
	oldIndex int
	newIndex int
}

func sortedPinnedNames(pinned map[string]bool) []string {
	names := make([]string, 0, len(pinned))
	for name := range pinned {
		names = append(names, name)
	}
	sort.Strings(names)
	return names
}

// pinnedCountConjunct returns a schema a caller instance satisfies
// exactly when the merged instance (the caller's arguments with
// every pin merged over them) satisfies the property-count keyword
// kind with bound n. A pin the caller also supplies adds nothing
// when it merges -- the merge overwrites it -- so the bound is
// projected according to the overlap between the pinned names and
// the caller's own properties: for each pin, the present branch
// counts it in the caller's total and the absent branch counts it
// as the merge's contribution (Codex P2 on PR #2976).
func pinnedCountConjunct(kind string, n int64, pins []string) any {
	if len(pins) == 0 {
		switch kind {
		case "minProperties":
			if n <= 0 {
				return true
			}
		case "maxProperties":
			if n < 0 {
				return false
			}
		}
		return map[string]any{kind: json.Number(strconv.FormatInt(n, 10))}
	}
	present := conjoinCountSchemas(
		map[string]any{"required": []any{pins[0]}},
		pinnedCountConjunct(kind, n, pins[1:]))
	absent := conjoinCountSchemas(
		map[string]any{"not": map[string]any{"required": []any{pins[0]}}},
		pinnedCountConjunct(kind, n-1, pins[1:]))
	return map[string]any{"anyOf": []any{present, absent}}
}

// conjoinCountSchemas conjoins a small keyword map with a count
// conjunct under construction: both must hold. A false conjunct
// falsifies the whole; a true one drops out.
func conjoinCountSchemas(a map[string]any, b any) any {
	bm, ok := b.(map[string]any)
	if !ok {
		if bb, isBool := b.(bool); isBool && bb {
			return a
		}
		return false
	}
	out := make(map[string]any, len(a)+len(bm))
	for k, v := range a {
		out[k] = v
	}
	for k, v := range bm {
		if k == "required" {
			if existing, ok := out[k].([]any); ok {
				if names, ok := v.([]any); ok {
					out[k] = append(existing, names...)
					continue
				}
			}
		}
		if _, clash := out[k]; clash {
			return map[string]any{"allOf": []any{a, bm}}
		}
		out[k] = v
	}
	return out
}

// conditionNamesAllPinned reports whether a condition schema's
// truth at its own instance scope depends only on pinned
// properties (see collectConditionNames).
func conditionNamesAllPinned(v any, pinned map[string]bool, root map[string]any, resolving map[string]bool) bool {
	names := map[string]bool{}
	if !collectConditionNames(v, root, resolving, names) {
		return false
	}
	for name := range names {
		if !pinned[name] {
			return false
		}
	}
	return true
}

// pinnedConjunctSchema extracts the sub-schema formed by v's
// pinned-only conjuncts: the assertions at v's own instance scope
// whose truth depends only on pinned properties -- a properties
// entry for a pinned name, a required entry naming only pinned
// names, a dependent* entry whose trigger and demands are all
// pinned, an allOf member's own pinned conjuncts, a negation the
// pins decide. It reports false when v carries no such conjunct.
// The extraction conjoins parts of v, so when the pins decide it
// false, v itself can never match the merged instance even though
// the rest of v depends on caller-supplied properties the full
// verdict must decline (Codex P2 on PR #2976).
func pinnedConjunctSchema(v any, pinned map[string]bool, pinVals map[string]any, root map[string]any, resolving map[string]bool) (any, bool) {
	if b, isBool := v.(bool); isBool {
		return b, true
	}
	m, ok := v.(map[string]any)
	if !ok {
		return nil, false
	}
	if ref, isStr := m["$ref"].(string); isStr && strings.HasPrefix(ref, "#") && dialectIgnoresRefSiblings(root) {
		// Draft-07 and earlier: the reference replaces the
		// schema outright, so its siblings are not conjuncts
		// of the branch at all.
		return nil, false
	}
	out := map[string]any{}
	for k, val := range m {
		switch k {
		case "properties":
			props, ok := val.(map[string]any)
			if !ok {
				continue
			}
			kept := map[string]any{}
			for name, sub := range props {
				if pinned[name] {
					kept[name] = sub
				}
			}
			if len(kept) > 0 {
				out[k] = kept
			}
		case "required":
			names, ok := val.([]any)
			if !ok {
				continue
			}
			kept := make([]any, 0, len(names))
			for _, e := range names {
				if name, isStr := e.(string); isStr && pinned[name] {
					kept = append(kept, e)
				}
			}
			if len(kept) > 0 {
				out[k] = kept
			}
		case "dependentRequired", "dependencies":
			deps, ok := val.(map[string]any)
			if !ok {
				continue
			}
			kept := map[string]any{}
			for trigger, depVal := range deps {
				if !pinned[trigger] {
					continue
				}
				switch dep := depVal.(type) {
				case []any:
					all := true
					for _, e := range dep {
						if name, isStr := e.(string); !isStr || !pinned[name] {
							all = false
							break
						}
					}
					if all {
						kept[trigger] = depVal
					}
				case map[string]any, bool:
					if conditionNamesAllPinned(depVal, pinned, root, resolving) {
						kept[trigger] = depVal
					}
				}
			}
			if len(kept) > 0 {
				out[k] = kept
			}
		case "dependentSchemas":
			subs, ok := val.(map[string]any)
			if !ok {
				continue
			}
			kept := map[string]any{}
			for trigger, sub := range subs {
				if pinned[trigger] && conditionNamesAllPinned(sub, pinned, root, resolving) {
					kept[trigger] = sub
				}
			}
			if len(kept) > 0 {
				out[k] = kept
			}
		case "allOf":
			subs, ok := val.([]any)
			if !ok {
				continue
			}
			kept := make([]any, 0, len(subs))
			for _, sub := range subs {
				if ex, has := pinnedConjunctSchema(sub, pinned, pinVals, root, resolving); has {
					kept = append(kept, ex)
				}
			}
			if len(kept) > 0 {
				out[k] = kept
			}
		case "not":
			// A negation the pins decide is itself a
			// pinned-only conjunct: decided valid inside
			// makes the conjunct false, decided invalid
			// makes it true (it drops out).
			if valid, decided := pinnedSchemaVerdict(val, pinned, pinVals, root, resolving); decided && valid {
				out[k] = val
			}
		}
	}
	if len(out) == 0 {
		return nil, false
	}
	return out, true
}

// pinnedConjunctVerdict reports whether the pinned-only conjuncts
// of a schema are decided by the pinned values alone, and if so
// whether they hold: a decided-false conjunct settles the whole
// schema against every caller, whatever its caller-dependent
// remainder says.
func pinnedConjunctVerdict(schema any, pinned map[string]bool, pinVals map[string]any, root map[string]any, resolving map[string]bool) (bool, bool) {
	ex, has := pinnedConjunctSchema(schema, pinned, pinVals, root, resolving)
	if !has {
		return false, false
	}
	return pinnedSchemaVerdict(ex, pinned, pinVals, root, resolving)
}

// PinnedConjunctVerdict exposes pinnedConjunctVerdict for the
// daemon's upstream projector, which must give a caller-dependent
// applicator branch the same partial evaluation before projecting
// it: projecting a branch whose pinned conjuncts already fail
// strips the very assertions that distinguished it from its
// siblings (Codex P2 on PR #2976).
func PinnedConjunctVerdict(schema any, pinVals map[string]any, root map[string]any) (bool, bool) {
	set := make(map[string]bool, len(pinVals))
	for name := range pinVals {
		set[name] = true
	}
	return pinnedConjunctVerdict(schema, set, pinVals, root, map[string]bool{})
}
