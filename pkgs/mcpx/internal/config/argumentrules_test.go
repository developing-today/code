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
