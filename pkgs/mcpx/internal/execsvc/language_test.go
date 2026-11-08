package execsvc

import (
	"encoding/json"
	"testing"
)

// TestExecOptionsLanguageWireName pins the /v1/exec spelling of the
// explicit script-language selection (#1917 remainder): the JSON field
// is "language", matching script.language and --language.
func TestExecOptionsLanguageWireName(t *testing.T) {
	var opts Options
	if err := json.Unmarshal([]byte(`{"language":"python"}`), &opts); err != nil {
		t.Fatal(err)
	}
	if opts.Language != "python" {
		t.Fatalf("Options.Language = %q, want python", opts.Language)
	}
	out, err := json.Marshal(Options{Language: "python"})
	if err != nil {
		t.Fatal(err)
	}
	if string(out) != `{"language":"python"}` {
		t.Fatalf("marshalled options = %s, want the language field only", out)
	}
}
