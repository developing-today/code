package config

import (
	"encoding/json"
	"strings"
	"testing"

	"github.com/dezren39/mcpx/internal/schemaval"
)

// The four worked examples from #1986, as one server's rule set.
func exampleRules(t *testing.T) *ArgumentRuleSet {
	t.Helper()
	set, err := CompileArgumentRules([]ArgumentRuleSource{
		{Key: "create_repository", Origin: "test", Schema: json.RawMessage(`{
			"required": ["private", "organization"],
			"properties": {
				"private": {"const": true},
				"organization": {"pattern": "^(my-org|my-org-sandbox)$"}
			}}`)},
		{Key: "/^(push_files|create_or_update_file|delete_file)$/", Origin: "test", Schema: json.RawMessage(`{
			"required": ["branch"],
			"properties": {"branch": {"not": {"enum": ["main", "master"]}}}}`)},
		{Key: "*", Origin: "test", Schema: json.RawMessage(`{
			"properties": {"owner": {"pattern": "^(my-org|my-user)$"}}}`)},
		{Key: "execute_datadog_workflow", Origin: "test", Schema: json.RawMessage(`{
			"required": ["workflow_id"],
			"properties": {"workflow_id": {"enum": ["id-1", "id-2"]}}}`)},
	})
	if err != nil {
		t.Fatalf("CompileArgumentRules: %v", err)
	}
	return set
}

func TestArgumentRulesWorkedExamples(t *testing.T) {
	set := exampleRules(t)

	// create_repository: private in a known org passes; public, or a
	// foreign org, or no organization at all is refused.
	if v := set.Check("create_repository", map[string]any{"private": true, "organization": "my-org"}); v != nil {
		t.Errorf("conforming create_repository refused: %v", v)
	}
	for name, args := range map[string]map[string]any{
		"public":      {"private": false, "organization": "my-org"},
		"foreign org": {"private": true, "organization": "evil-corp"},
		"no org":      {"private": true},
	} {
		v := set.Check("create_repository", args)
		if v == nil {
			t.Errorf("create_repository %s: allowed, want refusal", name)
			continue
		}
		if v.Rule.Key != "create_repository" {
			t.Errorf("create_repository %s: refused by rule %q, want create_repository", name, v.Rule.Key)
		}
	}

	// Branch rule, keyed by regexp across three tools: main/master
	// refused, a feature branch allowed, on every matched tool.
	for _, tool := range []string{"push_files", "create_or_update_file", "delete_file"} {
		if v := set.Check(tool, map[string]any{"branch": "main"}); v == nil {
			t.Errorf("%s to main: allowed, want refusal", tool)
		} else if !strings.Contains(v.Error(), "$.branch") || !strings.Contains(v.Error(), `"main"`) {
			t.Errorf("%s refusal should name path and value, got: %v", tool, v)
		}
		if v := set.Check(tool, map[string]any{"branch": "feat/x"}); v != nil {
			t.Errorf("%s to feat/x refused: %v", tool, v)
		}
	}
	// The regexp does not reach other tools.
	if v := set.Check("get_file_contents", map[string]any{"branch": "main"}); v != nil {
		t.Errorf("unmatched tool refused by branch rule: %v", v)
	}

	// Owner rule, keyed "*" per-argument: applies only when owner is
	// present, to any tool that carries it.
	if v := set.Check("list_repos", map[string]any{"owner": "someone-else"}); v == nil {
		t.Error("foreign owner allowed, want refusal")
	}
	if v := set.Check("list_repos", map[string]any{"owner": "my-user"}); v != nil {
		t.Errorf("own owner refused: %v", v)
	}
	if v := set.Check("list_repos", map[string]any{"page": 1}); v != nil {
		t.Errorf("tool without owner refused: %v", v)
	}

	// Workflow ids: only the reviewed enum.
	if v := set.Check("execute_datadog_workflow", map[string]any{"workflow_id": "id-9"}); v == nil {
		t.Error("unreviewed workflow id allowed, want refusal")
	}
	if v := set.Check("execute_datadog_workflow", map[string]any{"workflow_id": "id-2"}); v != nil {
		t.Errorf("reviewed workflow id refused: %v", v)
	}
}

func TestArgumentRuleViolationNamesOrigin(t *testing.T) {
	set := exampleRules(t)
	v := set.Check("create_repository", map[string]any{"private": false, "organization": "my-org"})
	if v == nil {
		t.Fatal("want violation")
	}
	msg := v.Error()
	for _, want := range []string{"$.private", "false", "create_repository", "test"} {
		if !strings.Contains(msg, want) {
			t.Errorf("violation %q does not name %q", msg, want)
		}
	}
}

func TestArgumentRulesLoadTimeErrors(t *testing.T) {
	// A bad schema is a load-time error.
	if _, err := CompileArgumentRules([]ArgumentRuleSource{
		{Key: "x", Origin: "test", Schema: json.RawMessage(`{"properties": {"a": {"type": "nonesuch"}}}`)},
	}); err == nil {
		t.Error("invalid schema compiled, want load-time error")
	}
	// A non-RE2 tool pattern key is a load-time error.
	if _, err := CompileArgumentRules([]ArgumentRuleSource{
		{Key: "/(?=delete)purge/", Origin: "test", Schema: json.RawMessage(`{}`)},
	}); err == nil {
		t.Error("lookahead tool pattern compiled, want load-time error")
	}
	// A schema pattern outside the validator's dialect (lookahead) is
	// a load-time error, not a rule that silently never matches.
	if _, err := CompileArgumentRules([]ArgumentRuleSource{
		{Key: "x", Origin: "test", Schema: json.RawMessage(`{"properties": {"a": {"pattern": "^(?=x)y"}}}`)},
	}); err == nil {
		t.Error("lookahead schema pattern compiled, want load-time error")
	}
}

func TestArgumentRulesComposeAcrossPoolServerAndLayers(t *testing.T) {
	// Pool says owner must be my-org; the server adds branch-not-main.
	// Both apply to one call (allOf composition through Resolve).
	cfg := &Config{
		Path: "base.json",
		Pool: Extras{ArgumentRules: map[string]json.RawMessage{
			"*": json.RawMessage(`{"properties": {"owner": {"pattern": "^my-org$"}}}`),
		}},
		MCPServers: map[string]*Server{
			"gh": {Command: "demo", Mcpx: &Extras{ArgumentRules: map[string]json.RawMessage{
				"push_files": json.RawMessage(`{"properties": {"branch": {"not": {"enum": ["main"]}}}}`),
			}}},
		},
	}
	r, err := cfg.Resolve("gh")
	if err != nil {
		t.Fatalf("Resolve: %v", err)
	}
	if v := r.ArgumentRules.Check("push_files", map[string]any{"owner": "other", "branch": "feat"}); v == nil {
		t.Error("pool rule not applied through Resolve")
	}
	if v := r.ArgumentRules.Check("push_files", map[string]any{"owner": "my-org", "branch": "main"}); v == nil {
		t.Error("server rule not applied through Resolve")
	}
	if v := r.ArgumentRules.Check("push_files", map[string]any{"owner": "my-org", "branch": "feat"}); v != nil {
		t.Errorf("conforming call refused: %v", v)
	}

	// Layered files: the far file's rule survives the merge, and a key
	// both files use composes instead of the near file loosening it.
	near := Extras{ArgumentRules: map[string]json.RawMessage{
		"*": json.RawMessage(`{"properties": {"branch": {"not": {"enum": ["main"]}}}}`),
	}}
	far := Extras{ArgumentRules: map[string]json.RawMessage{
		"*":      json.RawMessage(`{"properties": {"owner": {"pattern": "^my-org$"}}}`),
		"deploy": json.RawMessage(`{"properties": {"env": {"const": "staging"}}}`),
	}}
	merged := mergeExtras(near, far)
	set, err := CompileArgumentRules(argumentRuleSources(merged.ArgumentRules, "test"))
	if err != nil {
		t.Fatalf("compile merged: %v", err)
	}
	if v := set.Check("anything", map[string]any{"owner": "other"}); v == nil {
		t.Error("far file's owner rule lost in layered merge")
	}
	if v := set.Check("anything", map[string]any{"branch": "main"}); v == nil {
		t.Error("near file's branch rule lost in layered merge")
	}
	if v := set.Check("deploy", map[string]any{"env": "prod"}); v == nil {
		t.Error("far file's deploy rule lost in layered merge")
	}
}

func TestArgumentRulesDeadAndEffectiveSchema(t *testing.T) {
	set := exampleRules(t)
	dead := set.DeadRules([]string{"create_repository", "push_files"})
	// "*" and the branch regexp are alive; the datadog rule is dead
	// against this catalog.
	found := map[string]bool{}
	for _, r := range dead {
		found[r.Key] = true
	}
	if !found["execute_datadog_workflow"] {
		t.Errorf("dead rules = %v, want execute_datadog_workflow among them", dead)
	}
	if found["*"] || found["create_repository"] {
		t.Errorf("live rules reported dead: %v", dead)
	}

	upstream := json.RawMessage(`{"type": "object", "properties": {"private": {"type": "boolean"}}}`)
	eff := set.EffectiveSchema("create_repository", upstream)
	// The upstream schema stays at the root -- the MCP schemas
	// require inputSchema.type == "object" at the root itself, not
	// inside an allOf member -- and the matching rules (the
	// create_repository rule + the "*" rule) join as allOf members.
	var doc struct {
		Type       string                     `json:"type"`
		Properties map[string]json.RawMessage `json:"properties"`
		AllOf      []json.RawMessage          `json:"allOf"`
	}
	if err := json.Unmarshal(eff, &doc); err != nil {
		t.Fatalf("effective schema does not parse: %v", err)
	}
	if doc.Type != "object" {
		t.Errorf("effective schema root type = %q, want object: %s", doc.Type, eff)
	}
	if _, ok := doc.Properties["private"]; !ok {
		t.Errorf("upstream properties lost from the root: %s", eff)
	}
	if len(doc.AllOf) != 2 {
		t.Fatalf("effective schema = %s, want allOf of 2 rules", eff)
	}
	// A tool only the "*" rule matches gets just that one member.
	got := set.EffectiveSchema("unrelated_tool_xyz", upstream)
	var d2 struct {
		AllOf []json.RawMessage `json:"allOf"`
	}
	if err := json.Unmarshal(got, &d2); err != nil || len(d2.AllOf) != 1 {
		t.Fatalf("effective schema for other tool = %s, want allOf of 1 (* rule)", got)
	}
}

// A call with no arguments at all ("args": null on the wire, a nil
// map in the daemon) is judged as the empty object the client will
// actually send upstream -- never as JSON null, which object keywords
// like required do not reject.
func TestArgumentRuleNilArgsJudgedAsEmptyObject(t *testing.T) {
	set := exampleRules(t)
	if v := set.Check("create_repository", nil); v == nil {
		t.Error("nil args allowed past a rule requiring private+organization; want refusal")
	}
	// A rule that only constrains a present property still passes an
	// argument-less call.
	if v := set.Check("list_repos", nil); v != nil {
		t.Errorf("nil args refused by a properties-only rule: %v", v)
	}
}

// The advertised schema keeps the upstream document at the root --
// its type: object declaration and its x-mcp-header property
// annotations stay where clients look for them -- and a rule whose
// schema references its own $defs still resolves after composition.
func TestEffectiveSchemaPreservesRootAndRuleRefs(t *testing.T) {
	set, err := CompileArgumentRules([]ArgumentRuleSource{
		{Key: "read_file", Origin: "test", Schema: json.RawMessage(`{
			"$defs": {"nonEmpty": {"type": "string", "minLength": 1}},
			"properties": {"path": {"$ref": "#/$defs/nonEmpty"}}}`)},
	})
	if err != nil {
		t.Fatalf("CompileArgumentRules: %v", err)
	}
	upstream := json.RawMessage(`{"type": "object", "properties": {"path": {"type": "string", "x-mcp-header": "X-Path"}}}`)
	eff := set.EffectiveSchema("read_file", upstream)
	var root map[string]any
	if err := json.Unmarshal(eff, &root); err != nil {
		t.Fatalf("effective schema does not parse: %v", err)
	}
	if root["type"] != "object" {
		t.Errorf("root type = %v, want object: %s", root["type"], eff)
	}
	props, _ := root["properties"].(map[string]any)
	path, _ := props["path"].(map[string]any)
	if path["x-mcp-header"] != "X-Path" {
		t.Errorf("x-mcp-header annotation not reachable at the root properties: %s", eff)
	}
	// The composed document must itself compile and judge: the
	// rule's $defs reference survived composition.
	compiled, err := schemaval.Compile(eff)
	if err != nil {
		t.Fatalf("effective schema does not compile: %v (%s)", err, eff)
	}
	inst, err := schemaval.DecodeInstance(json.RawMessage(`{"path": ""}`))
	if err != nil {
		t.Fatal(err)
	}
	if err := compiled.Validate(inst); err == nil {
		t.Error("empty path validated against the composed schema; want the rule's minLength to bite")
	}
	inst, err = schemaval.DecodeInstance(json.RawMessage(`{"path": "x"}`))
	if err != nil {
		t.Fatal(err)
	}
	if err := compiled.Validate(inst); err != nil {
		t.Errorf("valid path refused by the composed schema: %v", err)
	}
}

// Two layers keying a rule on the same pattern compose with allOf;
// when both schemas carry local $defs references the merged rule
// still compiles and both sides still bite.
func TestMergedLayerRulesWithLocalRefsCompile(t *testing.T) {
	near := Extras{ArgumentRules: map[string]json.RawMessage{
		"*": json.RawMessage(`{"$defs": {"ok": {"type": "string", "pattern": "^near-"}}, "properties": {"branch": {"$ref": "#/$defs/ok"}}}`),
	}}
	far := Extras{ArgumentRules: map[string]json.RawMessage{
		"*": json.RawMessage(`{"$defs": {"ok": {"type": "string", "pattern": "^far-"}}, "properties": {"owner": {"$ref": "#/$defs/ok"}}}`),
	}}
	merged := mergeExtras(near, far)
	set, err := CompileArgumentRules(argumentRuleSources(merged.ArgumentRules, "test"))
	if err != nil {
		t.Fatalf("merged rules with local refs do not compile: %v", err)
	}
	// Branch can satisfy at most one side's pattern prefix per value;
	// use values that isolate each side's bite via the other property.
	if v := set.Check("anything", map[string]any{"branch": "zzz", "owner": "far-org"}); v == nil {
		t.Error("near layer's branch rule lost in the merge")
	}
	if v := set.Check("anything", map[string]any{"branch": "near-x", "owner": "zzz"}); v == nil {
		t.Error("far layer's owner rule lost in the merge")
	}
	if v := set.Check("anything", map[string]any{"branch": "near-x", "owner": "far-org"}); v != nil {
		t.Errorf("call satisfying both layers refused: %v", v)
	}
}

// Literal instance data is not a schema: a rule whose const/enum
// payload is an object carrying a "$ref" key permits THAT object.
// Layered merging must not rewrite the literal while re-basing real
// schema references, or the merged rule would forbid the permitted
// value and permit a rewritten one nobody wrote.
func TestMergedRulesPreserveLiteralRefValues(t *testing.T) {
	far := Extras{ArgumentRules: map[string]json.RawMessage{
		"*": json.RawMessage(`{"properties": {
			"payload": {"const": {"$ref": "#/ok"}},
			"mode": {"enum": [{"$ref": "#/ok"}, "plain"]}
		}}`),
	}}
	near := Extras{ArgumentRules: map[string]json.RawMessage{
		"*": json.RawMessage(`{}`),
	}}
	merged := mergeExtras(near, far)
	set, err := CompileArgumentRules(argumentRuleSources(merged.ArgumentRules, "test"))
	if err != nil {
		t.Fatalf("CompileArgumentRules: %v", err)
	}
	good := map[string]any{
		"payload": map[string]any{"$ref": "#/ok"},
		"mode":    map[string]any{"$ref": "#/ok"},
	}
	if v := set.Check("anything", good); v != nil {
		t.Errorf("call carrying the literal values the rule permits refused after merge: %v", v)
	}
	rewritten := map[string]any{
		"payload": map[string]any{"$ref": "#/$defs/far$doc/ok"},
		"mode":    "plain",
	}
	if v := set.Check("anything", rewritten); v == nil {
		t.Error("call carrying the rewritten literal the merge invented was permitted; the literal was treated as a schema reference")
	}
}

// Property and definition NAMES are not keywords: a rule constraining
// an argument named "default" (or "enum") writes that name as a key of
// the properties map, and its value is a schema like any other. The
// ref traversal must walk it in name position, or the schema's $ref
// is left pointing at the combined root and loading fails.
func TestMergedRulesTraversePropertyNamesNotKeywords(t *testing.T) {
	far := Extras{ArgumentRules: map[string]json.RawMessage{
		"*": json.RawMessage(`{
			"$defs": {"T": {"type": "string", "minLength": 2}},
			"properties": {
				"default": {"$ref": "#/$defs/T"},
				"enum": {"$ref": "#/$defs/T"}
			}}`),
	}}
	near := Extras{ArgumentRules: map[string]json.RawMessage{
		"*": json.RawMessage(`{}`),
	}}
	merged := mergeExtras(near, far)
	set, err := CompileArgumentRules(argumentRuleSources(merged.ArgumentRules, "test"))
	if err != nil {
		t.Fatalf("merged rule with arguments named default/enum does not compile: %v", err)
	}
	if v := set.Check("anything", map[string]any{"default": "ok", "enum": "ok"}); v != nil {
		t.Errorf("conforming call refused after merge: %v", v)
	}
	if v := set.Check("anything", map[string]any{"default": "x"}); v == nil {
		t.Error("rule behind the argument named default lost its bite in the merge")
	}
	if v := set.Check("anything", map[string]any{"enum": 42}); v == nil {
		t.Error("rule behind the argument named enum lost its bite in the merge")
	}
}

// A rule carrying its own absolute $id is a separate resource: its
// internal references resolve against that resource, so composition
// must copy the document without re-basing the references inside it.
func TestMergedRulesRespectSchemaIDBoundaries(t *testing.T) {
	far := Extras{ArgumentRules: map[string]json.RawMessage{
		"*": json.RawMessage(`{
			"$id": "https://example.com/rules/arg.json",
			"$defs": {"T": {"type": "string", "minLength": 2}},
			"properties": {"name": {"$ref": "#/$defs/T"}}}`),
	}}
	near := Extras{ArgumentRules: map[string]json.RawMessage{
		"*": json.RawMessage(`{"properties": {"other": {"type": "boolean"}}}`),
	}}
	merged := mergeExtras(near, far)
	set, err := CompileArgumentRules(argumentRuleSources(merged.ArgumentRules, "test"))
	if err != nil {
		t.Fatalf("merged rule with its own $id does not compile: %v", err)
	}
	if v := set.Check("anything", map[string]any{"name": "ok", "other": true}); v != nil {
		t.Errorf("conforming call refused after merge: %v", v)
	}
	if v := set.Check("anything", map[string]any{"name": "x"}); v == nil {
		t.Error("$id rule's referenced constraint lost its bite in the merge")
	}
}

// A plain-name fragment ("#T") references an $anchor, not a JSON
// Pointer into the document root: prefixing it produces a pointer to
// a nonexistent definition. Composition must leave anchor references
// (and their targets) alone.
func TestMergedRulesPreserveAnchorReferences(t *testing.T) {
	far := Extras{ArgumentRules: map[string]json.RawMessage{
		"*": json.RawMessage(`{
			"$defs": {"T": {"$anchor": "T", "type": "string", "minLength": 2}},
			"properties": {"name": {"$ref": "#T"}}}`),
	}}
	near := Extras{ArgumentRules: map[string]json.RawMessage{
		"*": json.RawMessage(`{}`),
	}}
	merged := mergeExtras(near, far)
	set, err := CompileArgumentRules(argumentRuleSources(merged.ArgumentRules, "test"))
	if err != nil {
		t.Fatalf("merged rule with an anchor reference does not compile: %v", err)
	}
	if v := set.Check("anything", map[string]any{"name": "ok"}); v != nil {
		t.Errorf("conforming call refused after merge: %v", v)
	}
	if v := set.Check("anything", map[string]any{"name": "x"}); v == nil {
		t.Error("anchor-referenced constraint lost its bite in the merge")
	}
}

// A nearer config file that redefines a server without restating its
// argumentRules must not drop the farther file's server-level rules:
// rules only tighten, across layers as within one file.
func TestServerArgumentRulesSurviveLayeredMerge(t *testing.T) {
	far := &Config{
		MCPServers: map[string]*Server{
			"gh": {Command: "demo", Mcpx: &Extras{ArgumentRules: map[string]json.RawMessage{
				"push_files": json.RawMessage(`{"properties": {"branch": {"not": {"enum": ["main"]}}}}`),
			}}},
		},
		Origin: map[string]string{},
	}
	near := &Config{
		MCPServers: map[string]*Server{
			"gh": {Command: "demo-near"},
		},
		Origin: map[string]string{},
	}
	mergeInto(near, far, "far.json")
	got := near.MCPServers["gh"]
	if got.Command != "demo-near" {
		t.Fatalf("nearer server definition should win, got command %q", got.Command)
	}
	if got.Mcpx == nil || len(got.Mcpx.ArgumentRules) != 1 {
		t.Fatalf("farther server's argumentRules lost in mergeInto: %+v", got.Mcpx)
	}
	set, err := CompileArgumentRules(argumentRuleSources(got.Mcpx.ArgumentRules, "test"))
	if err != nil {
		t.Fatalf("compile merged server rules: %v", err)
	}
	if v := set.Check("push_files", map[string]any{"branch": "main"}); v == nil {
		t.Error("inherited server rule does not bite after the layered merge")
	}
}

// A rule's scoped copy must never overwrite an upstream $defs entry
// that happens to carry the generated name: upstream references to
// that entry would silently resolve to the rule document instead of
// their original target (Codex P2 on PR #2855, round 5).
func TestEffectiveSchemaDoesNotOverwriteUpstreamDefs(t *testing.T) {
	set, err := CompileArgumentRules([]ArgumentRuleSource{
		{Key: "create_repository", Origin: "test", Schema: json.RawMessage(`{
			"$defs": {"s": {"type": "string", "pattern": "^ok-"}},
			"properties": {"name": {"$ref": "#/$defs/s"}}}`)},
	})
	if err != nil {
		t.Fatalf("CompileArgumentRules: %v", err)
	}
	upstream := json.RawMessage(`{"type": "object",
		"$defs": {"argumentrule0$doc": {"type": "string"}},
		"properties": {
			"path": {"$ref": "#/$defs/argumentrule0$doc"},
			"name": {"type": "string"}}}`)
	eff := set.EffectiveSchema("create_repository", upstream)
	var root map[string]any
	if err := json.Unmarshal(eff, &root); err != nil {
		t.Fatalf("effective schema does not parse: %v", err)
	}
	defs, _ := root["$defs"].(map[string]any)
	up, _ := defs["argumentrule0$doc"].(map[string]any)
	if up["type"] != "string" || len(up) != 1 {
		t.Fatalf("upstream $defs entry overwritten by rule composition: %s", eff)
	}
	if len(defs) != 2 {
		t.Fatalf("rule copy missing from $defs: %s", eff)
	}
	compiled, err := schemaval.Compile(eff)
	if err != nil {
		t.Fatalf("effective schema does not compile: %v (%s)", err, eff)
	}
	for inst, wantErr := range map[string]bool{
		`{"path": "p", "name": "ok-1"}`:      false,
		`{"path": {"x": 1}, "name": "ok-1"}`: true,
		`{"path": "p", "name": "nope"}`:      true,
	} {
		dec, err := schemaval.DecodeInstance(json.RawMessage(inst))
		if err != nil {
			t.Fatal(err)
		}
		if gotErr := compiled.Validate(dec) != nil; gotErr != wantErr {
			t.Errorf("validate %s: error = %v, want %v (schema %s)", inst, gotErr, wantErr, eff)
		}
	}
}

// Two layered rules may each define the same $anchor name for
// different targets: anchors are document-scoped, so copying both
// documents into one composed root without isolating their anchors
// collides them -- a duplicate-anchor compile error, or one rule's
// "#T" resolving to the other rule's target (Codex P2 on PR #2855,
// round 6). Composition must namespace each document's anchors.
func TestMergedRulesAnchorCollisionAcrossLayers(t *testing.T) {
	far := Extras{ArgumentRules: map[string]json.RawMessage{
		"*": json.RawMessage(`{
			"$defs": {"T": {"$anchor": "T", "type": "string"}},
			"properties": {"a": {"$ref": "#T"}}}`),
	}}
	near := Extras{ArgumentRules: map[string]json.RawMessage{
		"*": json.RawMessage(`{
			"$defs": {"T": {"$anchor": "T", "type": "integer"}},
			"properties": {"b": {"$ref": "#T"}}}`),
	}}
	merged := mergeExtras(near, far)
	set, err := CompileArgumentRules(argumentRuleSources(merged.ArgumentRules, "test"))
	if err != nil {
		t.Fatalf("merged rules with colliding anchors do not compile: %v", err)
	}
	if v := set.Check("anything", map[string]any{"a": "yes", "b": 1}); v != nil {
		t.Errorf("conforming call refused after merge: %v", v)
	}
	if v := set.Check("anything", map[string]any{"a": 1, "b": 1}); v == nil {
		t.Error("far rule's string anchor lost its bite in the merge")
	}
	if v := set.Check("anything", map[string]any{"a": "yes", "b": "x"}); v == nil {
		t.Error("near rule's integer anchor lost its bite in the merge")
	}
}

// A $dynamicRef is a local reference exactly like $ref for scoping
// purposes: a rule whose only local reference is dynamic must still
// be scoped, or the reference re-bases onto the composed root and
// its target definition is lost (Codex P2 on PR #2855, round 6).
func TestMergedRulesDynamicRefScoped(t *testing.T) {
	far := Extras{ArgumentRules: map[string]json.RawMessage{
		"*": json.RawMessage(`{
			"$defs": {"T": {"type": "string", "minLength": 2}},
			"properties": {"name": {"$dynamicRef": "#/$defs/T"}}}`),
	}}
	near := Extras{ArgumentRules: map[string]json.RawMessage{
		"*": json.RawMessage(`{}`),
	}}
	merged := mergeExtras(near, far)
	set, err := CompileArgumentRules(argumentRuleSources(merged.ArgumentRules, "test"))
	if err != nil {
		t.Fatalf("merged rule with a $dynamicRef does not compile: %v", err)
	}
	if v := set.Check("anything", map[string]any{"name": "ok"}); v != nil {
		t.Errorf("conforming call refused after merge: %v", v)
	}
	if v := set.Check("anything", map[string]any{"name": "x"}); v == nil {
		t.Error("$dynamicRef-referenced constraint lost its bite in the merge")
	}
}

// Two rules matching the same tool may define the same anchor name
// for different targets; the advertised effective schema composes
// both into one root, where their anchors must not collide.
func TestEffectiveSchemaAnchorCollisionAcrossRules(t *testing.T) {
	set, err := CompileArgumentRules([]ArgumentRuleSource{
		{Key: "*", Origin: "test", Schema: json.RawMessage(`{
			"$defs": {"T": {"$anchor": "T", "type": "string", "minLength": 2}},
			"properties": {"a": {"$ref": "#T"}}}`)},
		{Key: "do_thing", Origin: "test", Schema: json.RawMessage(`{
			"$defs": {"T": {"$anchor": "T", "type": "integer"}},
			"properties": {"b": {"$ref": "#T"}}}`)},
	})
	if err != nil {
		t.Fatalf("CompileArgumentRules: %v", err)
	}
	upstream := json.RawMessage(`{"type": "object", "properties": {"a": {"type": "string"}, "b": {"type": "integer"}}}`)
	eff := set.EffectiveSchema("do_thing", upstream)
	compiled, err := schemaval.Compile(eff)
	if err != nil {
		t.Fatalf("effective schema with colliding rule anchors does not compile: %v (%s)", err, eff)
	}
	for inst, wantErr := range map[string]bool{
		`{"a": "ok", "b": 1}`:   false,
		`{"a": "x", "b": 1}`:    true,
		`{"a": "ok", "b": "s"}`: true,
	} {
		dec, err := schemaval.DecodeInstance(json.RawMessage(inst))
		if err != nil {
			t.Fatal(err)
		}
		if gotErr := compiled.Validate(dec) != nil; gotErr != wantErr {
			t.Errorf("validate %s: error = %v, want %v (schema %s)", inst, gotErr, wantErr, eff)
		}
	}
}

// An upstream unevaluatedProperties: false forbids every property
// the upstream schema itself does not evaluate. Folding a rule into
// the same root under allOf must not make the properties the rule
// mentions count as evaluated for that keyword -- the advertised
// schema would accept arguments the upstream schema forbids
// (Codex P2 on PR #2855, round 6).
func TestEffectiveSchemaPreservesUnevaluatedProperties(t *testing.T) {
	set, err := CompileArgumentRules([]ArgumentRuleSource{
		{Key: "*", Origin: "test", Schema: json.RawMessage(`{"properties": {"private": {"const": true}}}`)},
	})
	if err != nil {
		t.Fatalf("CompileArgumentRules: %v", err)
	}
	upstream := json.RawMessage(`{"type": "object",
		"properties": {"path": {"type": "string"}},
		"unevaluatedProperties": false}`)
	eff := set.EffectiveSchema("anything", upstream)
	var root map[string]any
	if err := json.Unmarshal(eff, &root); err != nil {
		t.Fatalf("effective schema does not parse: %v", err)
	}
	if _, ok := root["properties"]; !ok {
		t.Errorf("upstream properties no longer advertised at the root: %s", eff)
	}
	compiled, err := schemaval.Compile(eff)
	if err != nil {
		t.Fatalf("effective schema does not compile: %v (%s)", err, eff)
	}
	for inst, wantErr := range map[string]bool{
		`{"path": "p"}`:                  false,
		`{"path": "p", "private": true}`: true,
		`{"path": "p", "other": 1}`:      true,
		`{"path": 1}`:                    true,
	} {
		dec, err := schemaval.DecodeInstance(json.RawMessage(inst))
		if err != nil {
			t.Fatal(err)
		}
		if gotErr := compiled.Validate(dec) != nil; gotErr != wantErr {
			t.Errorf("validate %s: error = %v, want %v (schema %s)", inst, gotErr, wantErr, eff)
		}
	}
}

// dependentSchemas subschemas are upstream applicators too: the
// properties they evaluate count for the upstream root's
// unevaluatedProperties, so the boundary member must carry them or
// the advertised schema forbids arguments the upstream schema
// accepts (Codex P2 on PR #2855, round 7).
func TestEffectiveSchemaUnevaluatedDependentSchemas(t *testing.T) {
	set, err := CompileArgumentRules([]ArgumentRuleSource{
		{Key: "*", Origin: "test", Schema: json.RawMessage(`{}`)},
	})
	if err != nil {
		t.Fatalf("CompileArgumentRules: %v", err)
	}
	upstream := json.RawMessage(`{"type": "object",
		"properties": {"trigger": {}},
		"dependentSchemas": {"trigger": {"properties": {"other": {}}}},
		"unevaluatedProperties": false}`)
	eff := set.EffectiveSchema("anything", upstream)
	compiled, err := schemaval.Compile(eff)
	if err != nil {
		t.Fatalf("effective schema does not compile: %v (%s)", err, eff)
	}
	for inst, wantErr := range map[string]bool{
		`{"trigger": true, "other": "ok"}`:         false,
		`{"trigger": true}`:                        false,
		`{"trigger": true, "other": "ok", "x": 1}`: true,
		`{"other": "ok"}`:                          true,
	} {
		dec, err := schemaval.DecodeInstance(json.RawMessage(inst))
		if err != nil {
			t.Fatal(err)
		}
		if gotErr := compiled.Validate(dec) != nil; gotErr != wantErr {
			t.Errorf("validate %s: error = %v, want %v (schema %s)", inst, gotErr, wantErr, eff)
		}
	}
}

// EffectiveSchemaPinned: a rule requirement on a property the view
// pins is satisfied by the pin (pins merge before enforcement), so
// the advertised projection must not demand it from the caller —
// with the property hidden and additionalProperties:false upstream,
// the verbatim rule would advertise an unsatisfiable schema (Codex
// P2 on PR #2976).
func TestEffectiveSchemaPinnedDropsPinnedRequirements(t *testing.T) {
	set, err := CompileArgumentRules([]ArgumentRuleSource{
		{Key: "list_devices", Origin: "test", Schema: json.RawMessage(`{"required":["tailnet","limit"],"dependentRequired":{"verbose":["tailnet"]},"allOf":[{"required":["tailnet","limit"]}]}`)},
	})
	if err != nil {
		t.Fatalf("CompileArgumentRules: %v", err)
	}
	upstream := json.RawMessage(`{"type":"object","additionalProperties":false,"properties":{"limit":{"type":"number"}}}`)
	got := string(set.EffectiveSchemaPinned("list_devices", upstream, map[string]any{"tailnet": "-"}))
	if strings.Contains(got, "tailnet") {
		t.Errorf("pinned property must not appear in the advertised projection: %s", got)
	}
	if !strings.Contains(got, `"required":["limit"]`) {
		t.Errorf("unpinned requirements must still be advertised: %s", got)
	}
	// The un-projected path is untouched: enforcement-facing schema
	// generation still carries the full rule.
	full := string(set.EffectiveSchema("list_devices", upstream))
	if !strings.Contains(full, "tailnet") {
		t.Errorf("EffectiveSchema must keep the verbatim rule: %s", full)
	}
}

// Codex P2 on PR #2976: the pinned projection must respect JSON
// Schema instance scope. A nested object's property that shares a
// pinned name is a different property -- merging the top-level pin
// does not populate it -- so its requirement must survive the
// projection verbatim, or the advertised schema accepts calls
// enforcement rejects.
func TestEffectiveSchemaPinnedKeepsNestedScopeRequirements(t *testing.T) {
	set, err := CompileArgumentRules([]ArgumentRuleSource{
		{Key: "list_devices", Origin: "test", Schema: json.RawMessage(`{"properties":{"options":{"type":"object","required":["tailnet"]}}}`)},
	})
	if err != nil {
		t.Fatalf("CompileArgumentRules: %v", err)
	}
	upstream := json.RawMessage(`{"type":"object","properties":{"options":{"type":"object"}}}`)
	got := string(set.EffectiveSchemaPinned("list_devices", upstream, map[string]any{"tailnet": "-"}))
	if !strings.Contains(got, `"required":["tailnet"]`) {
		t.Errorf("nested-scope requirement on a same-named property must survive projection: %s", got)
	}
}

// Codex P2 on PR #2976: pins merge before rules are enforced, so a
// dependentRequired entry triggered by a pinned property always
// fires at enforcement time. The advertised projection must promote
// the trigger's remaining dependencies to unconditional requirements,
// or it accepts calls ({} for the issue's example) that enforcement
// rejects for a missing dependency.
func TestEffectiveSchemaPinnedPromotesPinnedTriggerDependencies(t *testing.T) {
	set, err := CompileArgumentRules([]ArgumentRuleSource{
		{Key: "list_devices", Origin: "test", Schema: json.RawMessage(`{"dependentRequired":{"tailnet":["limit"],"verbose":["tailnet","format"]}}`)},
	})
	if err != nil {
		t.Fatalf("CompileArgumentRules: %v", err)
	}
	upstream := json.RawMessage(`{"type":"object","properties":{"limit":{"type":"number"},"format":{"type":"string"}}}`)
	got := string(set.EffectiveSchemaPinned("list_devices", upstream, map[string]any{"tailnet": "-"}))
	if !strings.Contains(got, `"required":["limit"]`) {
		t.Errorf("dependencies of a pinned trigger must promote to required: %s", got)
	}
	if !strings.Contains(got, `"dependentRequired":{"verbose":["format"]}`) {
		t.Errorf("unpinned triggers keep their entry with pinned names filtered: %s", got)
	}
	if strings.Contains(got, "tailnet") {
		t.Errorf("pinned property must not appear in the advertised projection: %s", got)
	}
}

// Codex P2 on PR #2976: a dependentSchemas entry triggered by a
// pinned property always fires at enforcement (the pin is merged
// before rules run), so the advertised projection must promote the
// dependent schema to an unconditional same-instance constraint --
// the dependentRequired promotion alone leaves callers seeing {} as
// valid while enforcement rejects it for the missing dependency.
func TestEffectiveSchemaPinnedPromotesPinnedDependentSchemas(t *testing.T) {
	set, err := CompileArgumentRules([]ArgumentRuleSource{
		{Key: "list_devices", Origin: "test", Schema: json.RawMessage(`{"dependentSchemas":{"tailnet":{"required":["limit"]},"verbose":{"required":["format"]}}}`)},
	})
	if err != nil {
		t.Fatalf("CompileArgumentRules: %v", err)
	}
	upstream := json.RawMessage(`{"type":"object","properties":{"limit":{"type":"number"},"format":{"type":"string"}}}`)
	got := string(set.EffectiveSchemaPinned("list_devices", upstream, map[string]any{"tailnet": "-"}))
	if !strings.Contains(got, `"allOf":[{"required":["limit"]}]`) {
		t.Errorf("pinned trigger's dependent schema must promote to allOf: %s", got)
	}
	if strings.Contains(got, `"dependentSchemas":{"tailnet"`) {
		t.Errorf("pinned trigger must not stay conditional in the projection: %s", got)
	}
	if !strings.Contains(got, `"dependentSchemas":{"verbose":{"required":["format"]}}`) {
		t.Errorf("unpinned triggers keep their dependent schema: %s", got)
	}
}

// Codex P2 on PR #2976: a requirement reached through a local
// reference sits at the instance scope the reference occupies -- a
// root $ref is a root constraint -- so the pinned projection must
// resolve it instead of leaving the referenced requirement in place
// (which, against a hidden pin and additionalProperties:false,
// advertised an unsatisfiable schema).
func TestEffectiveSchemaPinnedResolvesLocalRefRequirements(t *testing.T) {
	set, err := CompileArgumentRules([]ArgumentRuleSource{
		{Key: "list_devices", Origin: "test", Schema: json.RawMessage(`{"$defs":{"need":{"required":["tailnet","limit"]}},"$ref":"#/$defs/need"}`)},
	})
	if err != nil {
		t.Fatalf("CompileArgumentRules: %v", err)
	}
	upstream := json.RawMessage(`{"type":"object","additionalProperties":false,"properties":{"limit":{"type":"number"}}}`)
	got := string(set.EffectiveSchemaPinned("list_devices", upstream, map[string]any{"tailnet": "-"}))
	if !strings.Contains(got, `"required":["limit"]`) {
		t.Errorf("referenced requirement must be projected at the root scope: %s", got)
	}
	if strings.Contains(got, `"required":["tailnet","limit"]`) && !strings.Contains(got, `"$defs"`) {
		t.Errorf("unprojected referenced requirement must not survive outside $defs: %s", got)
	}
}

// Codex P2 on PR #2976: a condition the pins decide must be projected
// by its decided branch, not by name-stripping. With tailnet pinned to
// "other", enforcement merges the pin, the rule's if (tailnet const
// "prod") is false, and the else branch requires cursor. Stripping
// the pinned name from the if's required made the advertised if
// succeed for a caller who omits tailnet, advertising the then branch
// (limit required) instead -- advertised-valid calls were rejected
// and runtime-valid calls failed advertised validation.
func TestEffectiveSchemaPinnedDecidesPinDeterminedConditions(t *testing.T) {
	rule := `{"if":{"properties":{"tailnet":{"const":"prod"}},"required":["tailnet"]},"then":{"required":["limit"]},"else":{"required":["cursor"]}}`
	upstream := json.RawMessage(`{"type":"object","properties":{"limit":{"type":"number"},"cursor":{"type":"string"}}}`)
	for _, tc := range []struct {
		pin       string
		valid     []string
		invalid   []string
		branchReq string
	}{
		// Pin "other": the condition is false at enforcement, so the
		// else requirement (cursor) is what callers must satisfy.
		{"other", []string{`{"cursor":"c"}`}, []string{`{}`, `{"limit":1}`}, `"required":["cursor"]`},
		// Pin "prod": the condition holds, so the then requirement
		// (limit) applies and cursor alone no longer suffices.
		{"prod", []string{`{"limit":1}`}, []string{`{}`, `{"cursor":"c"}`}, `"required":["limit"]`},
	} {
		set, err := CompileArgumentRules([]ArgumentRuleSource{
			{Key: "list_devices", Origin: "test", Schema: json.RawMessage(rule)},
		})
		if err != nil {
			t.Fatalf("CompileArgumentRules: %v", err)
		}
		eff := set.EffectiveSchemaPinned("list_devices", upstream, map[string]any{"tailnet": tc.pin})
		if strings.Contains(string(eff), `"if"`) {
			t.Errorf("pin %q: the decided condition must not survive in the projection: %s", tc.pin, eff)
		}
		if !strings.Contains(string(eff), tc.branchReq) {
			t.Errorf("pin %q: decided branch requirement %s must be advertised: %s", tc.pin, tc.branchReq, eff)
		}
		compiled, err := schemaval.Compile(eff)
		if err != nil {
			t.Fatalf("pin %q: effective schema does not compile: %v (%s)", tc.pin, err, eff)
		}
		for _, inst := range tc.valid {
			dec, err := schemaval.DecodeInstance(json.RawMessage(inst))
			if err != nil {
				t.Fatal(err)
			}
			if err := compiled.Validate(dec); err != nil {
				t.Errorf("pin %q: %s must be advertised-valid: %v (schema %s)", tc.pin, inst, err, eff)
			}
		}
		for _, inst := range tc.invalid {
			dec, err := schemaval.DecodeInstance(json.RawMessage(inst))
			if err != nil {
				t.Fatal(err)
			}
			if err := compiled.Validate(dec); err == nil {
				t.Errorf("pin %q: %s must be advertised-invalid (schema %s)", tc.pin, inst, eff)
			}
		}
	}
}

// A condition that also reads a caller-supplied property is NOT
// decided by the pins: it stays a condition in the projection (with
// the pinned names stripped as before), because the caller still
// controls its outcome.
func TestEffectiveSchemaPinnedKeepsUndecidedConditions(t *testing.T) {
	set, err := CompileArgumentRules([]ArgumentRuleSource{
		{Key: "list_devices", Origin: "test", Schema: json.RawMessage(`{"if":{"properties":{"tailnet":{"const":"prod"},"verbose":{"const":true}},"required":["tailnet","verbose"]},"then":{"required":["limit"]}}`)},
	})
	if err != nil {
		t.Fatalf("CompileArgumentRules: %v", err)
	}
	upstream := json.RawMessage(`{"type":"object","properties":{"limit":{"type":"number"},"verbose":{"type":"boolean"}}}`)
	got := string(set.EffectiveSchemaPinned("list_devices", upstream, map[string]any{"tailnet": "prod"}))
	if !strings.Contains(got, `"if"`) {
		t.Errorf("a condition reading unpinned properties must stay conditional: %s", got)
	}
}

// Codex P2 on PR #2976: reference inlining for a pin-decided
// condition must traverse only schema-valued positions. A literal
// $ref inside a const payload is instance data, not a reference:
// inlining it rewrites the constant, so the projection evaluates
// the condition against a different value than enforcement does
// and advertises the wrong branch.
func TestEffectiveSchemaPinnedKeepsLiteralRefConstants(t *testing.T) {
	rule := `{"$defs":{"value":{"type":"string"}},"if":{"properties":{"selector":{"const":{"$ref":"#/$defs/value"}}},"required":["selector"]},"then":{"required":["limit"]},"else":{"required":["cursor"]}}`
	set, err := CompileArgumentRules([]ArgumentRuleSource{
		{Key: "list_devices", Origin: "test", Schema: json.RawMessage(rule)},
	})
	if err != nil {
		t.Fatalf("CompileArgumentRules: %v", err)
	}
	upstream := json.RawMessage(`{"type":"object","properties":{"limit":{"type":"number"},"cursor":{"type":"string"}}}`)
	pin := map[string]any{"selector": map[string]any{"$ref": "#/$defs/value"}}
	eff := set.EffectiveSchemaPinned("list_devices", upstream, pin)
	if !strings.Contains(string(eff), `"required":["limit"]`) {
		t.Errorf("condition is true at enforcement (pin equals the const), so the then requirement must be advertised: %s", eff)
	}
	compiled, err := schemaval.Compile(eff)
	if err != nil {
		t.Fatalf("effective schema does not compile: %v (%s)", err, eff)
	}
	dec, err := schemaval.DecodeInstance(json.RawMessage(`{"limit":1}`))
	if err != nil {
		t.Fatal(err)
	}
	if err := compiled.Validate(dec); err != nil {
		t.Errorf("{\"limit\":1} must be advertised-valid: %v (schema %s)", err, eff)
	}
	dec, err = schemaval.DecodeInstance(json.RawMessage(`{}`))
	if err != nil {
		t.Fatal(err)
	}
	if err := compiled.Validate(dec); err == nil {
		t.Errorf("{} must be advertised-invalid (schema %s)", eff)
	}
}

// Codex P2 on PR #2976: pinned constraints under a negation must be
// evaluated against the pin, not stripped. Stripping the pinned
// requirement inside "not" leaves a properties-only schema that a
// caller instance satisfies vacuously, so the advertised schema
// rejected every valid call (pin tailnet="other": the negated
// const "prod" never matches the merged pin, so the negation
// always passes at enforcement).
func TestEffectiveSchemaPinnedEvaluatesNegatedPins(t *testing.T) {
	rule := `{"not":{"properties":{"tailnet":{"const":"prod"}},"required":["tailnet"]}}`
	upstream := json.RawMessage(`{"type":"object","properties":{"limit":{"type":"number"}}}`)
	for _, tc := range []struct {
		pin     string
		wantErr bool // for the {} instance against the advertised schema
	}{
		{"other", false}, // negation always passes: {} advertised-valid
		{"prod", true},   // negation always fails: {} advertised-invalid
	} {
		set, err := CompileArgumentRules([]ArgumentRuleSource{
			{Key: "list_devices", Origin: "test", Schema: json.RawMessage(rule)},
		})
		if err != nil {
			t.Fatalf("CompileArgumentRules: %v", err)
		}
		eff := set.EffectiveSchemaPinned("list_devices", upstream, map[string]any{"tailnet": tc.pin})
		compiled, err := schemaval.Compile(eff)
		if err != nil {
			t.Fatalf("pin %q: effective schema does not compile: %v (%s)", tc.pin, err, eff)
		}
		dec, err := schemaval.DecodeInstance(json.RawMessage(`{}`))
		if err != nil {
			t.Fatal(err)
		}
		if gotErr := compiled.Validate(dec) != nil; gotErr != tc.wantErr {
			t.Errorf("pin %q: validate {} error = %v, want %v (schema %s)", tc.pin, gotErr, tc.wantErr, eff)
		}
	}
	// A negation reading only unpinned properties is not decided by
	// the pins: it survives verbatim (stripping under a negation
	// would invert the pinned names' effect).
	set, err := CompileArgumentRules([]ArgumentRuleSource{
		{Key: "list_devices", Origin: "test", Schema: json.RawMessage(`{"not":{"properties":{"verbose":{"const":true}},"required":["verbose"]}}`)},
	})
	if err != nil {
		t.Fatalf("CompileArgumentRules: %v", err)
	}
	got := string(set.EffectiveSchemaPinned("list_devices", upstream, map[string]any{"tailnet": "prod"}))
	if !strings.Contains(got, `"not":{"properties":{"verbose":{"const":true}},"required":["verbose"]}`) {
		t.Errorf("undecided negation must survive verbatim: %s", got)
	}
}

// Codex P2 on PR #2976: a subschema carrying its own $id is a
// separate resource — references inside it resolve against that
// resource, not the outer rule document. The inliner must decline
// pin-decided evaluation for such shapes rather than substituting
// the outer document's same-named definition and selecting the
// branch enforcement does not.
func TestEffectiveSchemaPinnedDeclinesIDCarryingConditions(t *testing.T) {
	rule := `{"$defs":{"value":{"const":"other"}},"if":{"$id":"https://example.com/cond","$defs":{"value":{"const":"prod"}},"properties":{"tailnet":{"$ref":"#/$defs/value"}},"required":["tailnet"]},"then":{"required":["limit"]},"else":{"required":["cursor"]}}`
	set, err := CompileArgumentRules([]ArgumentRuleSource{
		{Key: "list_devices", Origin: "test", Schema: json.RawMessage(rule)},
	})
	if err != nil {
		t.Fatalf("CompileArgumentRules: %v", err)
	}
	upstream := json.RawMessage(`{"type":"object","properties":{"limit":{"type":"number"},"cursor":{"type":"string"}}}`)
	got := string(set.EffectiveSchemaPinned("list_devices", upstream, map[string]any{"tailnet": "prod"}))
	if !strings.Contains(got, `"if"`) {
		t.Errorf("a condition inside an $id-carrying resource must stay undecided, not be evaluated against the outer document: %s", got)
	}
}

// Codex P2 on PR #2976: a oneOf whose branches are discriminated by
// a pinned property must be projected by branch verdict, not by
// name-stripping. Stripping the pinned requirement from every branch
// left branches a caller matches vacuously (the pinned property is
// hidden), so the advertised oneOf counted two matches where
// enforcement -- which merges the pin first -- counts exactly one,
// and rejected calls that are valid at runtime.
func TestEffectiveSchemaPinnedDecidesOneOfBranches(t *testing.T) {
	upstream := json.RawMessage(`{"type":"object","properties":{"limit":{"type":"number"}}}`)
	check := func(t *testing.T, rule, pin string, valid, invalid []string) {
		t.Helper()
		set, err := CompileArgumentRules([]ArgumentRuleSource{
			{Key: "list_devices", Origin: "test", Schema: json.RawMessage(rule)},
		})
		if err != nil {
			t.Fatalf("CompileArgumentRules: %v", err)
		}
		eff := set.EffectiveSchemaPinned("list_devices", upstream, map[string]any{"tailnet": pin})
		compiled, err := schemaval.Compile(eff)
		if err != nil {
			t.Fatalf("effective schema does not compile: %v (%s)", err, eff)
		}
		for _, inst := range valid {
			dec, err := schemaval.DecodeInstance(json.RawMessage(inst))
			if err != nil {
				t.Fatal(err)
			}
			if err := compiled.Validate(dec); err != nil {
				t.Errorf("pin %q rule %s: %s must be advertised-valid: %v (schema %s)", pin, rule, inst, err, eff)
			}
		}
		for _, inst := range invalid {
			dec, err := schemaval.DecodeInstance(json.RawMessage(inst))
			if err != nil {
				t.Fatal(err)
			}
			if err := compiled.Validate(dec); err == nil {
				t.Errorf("pin %q rule %s: %s must be advertised-invalid (schema %s)", pin, rule, inst, eff)
			}
		}
	}

	// The Codex example: two branches discriminated by the pinned
	// tailnet. Exactly one matches after the pin merges, for either
	// pin value, whatever else the caller sends.
	discriminated := `{"oneOf":[{"properties":{"tailnet":{"const":"prod"}},"required":["tailnet"]},{"properties":{"tailnet":{"const":"other"}},"required":["tailnet"]}]}`
	for _, pin := range []string{"prod", "other"} {
		check(t, discriminated, pin, []string{`{}`, `{"limit":1}`}, nil)
	}
	// One decided branch plus one caller-decided branch: the caller
	// must not also match the undecided branch.
	mixed := `{"oneOf":[{"properties":{"tailnet":{"const":"prod"}},"required":["tailnet"]},{"required":["limit"]}]}`
	check(t, mixed, "prod", []string{`{}`}, []string{`{"limit":1}`})
	// Two branches the pin satisfies: exactly-one can never hold.
	check(t, `{"oneOf":[{"properties":{"tailnet":{"const":"prod"}},"required":["tailnet"]},{"required":["tailnet"]}]}`, "prod", nil, []string{`{}`})
	// No branch the pin satisfies: oneOf and anyOf both fail, and an
	// allOf with a decided-false branch fails with it.
	check(t, discriminated, "third", nil, []string{`{}`})
	check(t, `{"anyOf":[{"properties":{"tailnet":{"const":"other"}},"required":["tailnet"]}]}`, "prod", nil, []string{`{}`})
	check(t, `{"allOf":[{"properties":{"tailnet":{"const":"other"}},"required":["tailnet"]}]}`, "prod", nil, []string{`{}`})
	// An anyOf with a pin-satisfied branch constrains nothing.
	check(t, `{"anyOf":[{"properties":{"tailnet":{"const":"prod"}},"required":["tailnet"]},{"required":["limit"]}]}`, "prod", []string{`{}`, `{"limit":1}`}, nil)
}

// A $ref to a non-object schema (here a boolean definition) still
// carries its siblings as same-instance constraints: returning the
// target alone dropped a required the projection must keep (with
// pinned names filtered as usual).
func TestEffectiveSchemaPinnedKeepsNonObjectRefSiblings(t *testing.T) {
	set, err := CompileArgumentRules([]ArgumentRuleSource{
		{Key: "list_devices", Origin: "test", Schema: json.RawMessage(`{"$defs":{"t":true},"$ref":"#/$defs/t","required":["tailnet","limit"]}`)},
	})
	if err != nil {
		t.Fatalf("CompileArgumentRules: %v", err)
	}
	upstream := json.RawMessage(`{"type":"object","properties":{"limit":{"type":"number"}}}`)
	got := string(set.EffectiveSchemaPinned("list_devices", upstream, map[string]any{"tailnet": "-"}))
	if !strings.Contains(got, `"required":["limit"]`) {
		t.Errorf("siblings of a non-object $ref target must survive projection: %s", got)
	}
	if strings.Contains(got, "tailnet") {
		t.Errorf("pinned property must not appear in the advertised projection: %s", got)
	}
}
