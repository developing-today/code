package cli

import (
	"encoding/json"
	"testing"

	"github.com/developing-today/code/pkgs/mcpx/internal/execsvc"
	"github.com/developing-today/code/pkgs/mcpx/internal/logging"
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

// The --json envelope's results are the service's emits, which interception
// has already rewritten, not the values collected live before it ran: an
// intercepted image must reach the caller as a link, not its base64 body.
func TestRunEnvelopeResultsAreTheRewrittenEmits(t *testing.T) {
	raw := json.RawMessage(`{"type":"image","data":"iVBORw0KGgo=","mimeType":"image/png"}`)
	link := json.RawMessage(`{"type":"resource_link","uri":"mcpx://artifacts/a"}`)
	res := &execsvc.Result{Emits: []json.RawMessage{link}}
	env := runEnvelope(res, "", nil, []logging.Streamed{{Value: raw}})
	got, _ := json.Marshal(env["results"])
	if string(got) != `[`+string(link)+`]` {
		t.Errorf("results = %s, want the rewritten emit %s", got, link)
	}
}
