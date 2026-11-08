package api_test

import (
	"encoding/json"
	"testing"

	"github.com/developing-today/code/pkgs/mcpx/internal/api"
)

// TestExecOptionsSchemaDeclaresDeps is the Codex finding on PR #2589
// (ops_exec.go): the decoder accepted options.deps, but the published
// execOptionsSchema omitted it and sets additionalProperties:false, so
// schema-validating clients rejected dependency-bearing requests.
// The public schema must declare deps and its transported fields.
func TestExecOptionsSchemaDeclaresDeps(t *testing.T) {
	o, ok := api.ByName("exec")
	if !ok {
		t.Fatal("no exec operation")
	}
	var doc map[string]any
	if err := json.Unmarshal(o.InputSchema(), &doc); err != nil {
		t.Fatal(err)
	}
	props, _ := doc["properties"].(map[string]any)
	options, _ := props["options"].(map[string]any)
	optProps, _ := options["properties"].(map[string]any)
	depsProp, ok := optProps["deps"].(map[string]any)
	if !ok {
		t.Fatalf("published exec options schema does not declare deps: %v", optProps)
	}
	if depsProp["additionalProperties"] != false {
		t.Error("deps schema must set additionalProperties:false like its siblings")
	}
	depsProps, _ := depsProp["properties"].(map[string]any)
	for _, field := range []string{"packages", "manifest", "manifestName", "manifestDir", "packageDirs", "lockfile", "lockfileName", "lockfiles", "npmrc", "source", "autoInstall"} {
		if _, ok := depsProps[field]; !ok {
			t.Errorf("deps schema is missing transported field %q", field)
		}
	}
}
