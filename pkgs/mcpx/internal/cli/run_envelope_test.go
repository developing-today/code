package cli

import (
	"testing"

	"github.com/developing-today/code/pkgs/mcpx/internal/execsvc"
)

// CODE-30 (#1937; Codex review, PR #9205): the --json run envelope
// carries the typed failure kind beside the error, so a CLI consumer
// can branch on it exactly like an HTTP caller reading the Result.
func TestRunEnvelopeCarriesFailureKind(t *testing.T) {
	res := &execsvc.Result{ExitCode: 1, Error: "boom", ErrorKind: execsvc.FailureExecution}
	env := runEnvelope(res, "", nil, nil)
	if env["error"] != "boom" {
		t.Errorf("envelope error = %v, want %q", env["error"], "boom")
	}
	if env["errorKind"] != execsvc.FailureExecution {
		t.Errorf("envelope errorKind = %v, want %q", env["errorKind"], execsvc.FailureExecution)
	}

	clean := runEnvelope(&execsvc.Result{}, "", nil, nil)
	if _, ok := clean["error"]; ok {
		t.Errorf("clean envelope carries error: %v", clean["error"])
	}
	if _, ok := clean["errorKind"]; ok {
		t.Errorf("clean envelope carries errorKind: %v", clean["errorKind"])
	}
}
