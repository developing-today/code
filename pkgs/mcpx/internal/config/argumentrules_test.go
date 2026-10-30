package config

import (
	"encoding/json"
	"strings"
	"testing"
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
	var doc struct {
		AllOf []json.RawMessage `json:"allOf"`
	}
	if err := json.Unmarshal(eff, &doc); err != nil || len(doc.AllOf) != 3 {
		// Upstream + create_repository rule + "*" rule.
		t.Fatalf("effective schema = %s, want allOf of 3", eff)
	}
	if got := set.EffectiveSchema("unrelated_tool_xyz", upstream); string(got) != string(upstream) {
		// Only the "*" rule matches any tool; a tool with no owner
		// argument still gets the "*" rule folded in, which is the
		// point of a per-argument rule.
		var d2 struct {
			AllOf []json.RawMessage `json:"allOf"`
		}
		if err := json.Unmarshal(got, &d2); err != nil || len(d2.AllOf) != 2 {
			t.Fatalf("effective schema for other tool = %s, want allOf of 2 (upstream + * rule)", got)
		}
	}
}
