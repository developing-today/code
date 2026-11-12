package e2e_test

import (
	"strings"
	"testing"
)

// #6116: an unannotated tool is a possibly destructive write by the
// specification's defaults, so under the default policy (write=ask,
// confirmDestructive floor on) a headless caller -- nobody answers
// the gate's question -- could never run it. Declaring the tool
// readOnlyTools in the server's mcpx block is the operator's
// pre-authorization: the call runs end-to-end and returns its real
// result.
const declaredReadOnlyServer = `{
  "mcpServers": {
    "demo": { "command": "FAKE", "mcpx": { "sharing": "shared", "scope": "global", "readOnlyTools": ["structured"] } }
  }
}`

func TestDeclaredReadOnlyToolRunsHeadless(t *testing.T) {
	e := newEnv(t, declaredReadOnlyServer)
	e.setenv("MCPX_POLICY_WRITE=ask", "MCPX_ELICIT_CONFIRM_DESTRUCTIVE=true", "MCPX_ELICIT_ASK_TIMEOUT=2s")

	// structured publishes no annotations. Declared, it executes
	// and its genuine result comes back.
	if out := e.run("call", "demo.structured", `{}`); !strings.Contains(out, "42") {
		t.Fatalf("a declared read-only call should return its result:\n%s", out)
	}

	// wipe is not declared: the gate still fires, and the refusal
	// names the gate -- a typed refusal, not a fake result.
	out, err := e.try("call", "demo.wipe", `{}`)
	if err == nil {
		t.Fatalf("an undeclared destructive call should be refused:\n%s", out)
	}
	if !strings.Contains(out, "not confirmed") {
		t.Fatalf("the refusal should say it was not confirmed:\n%s", out)
	}
}

// The control: the same server, the same headless session, no
// declaration -- the unannotated tool is gated and never runs.
func TestUndeclaredUnannotatedToolStaysGatedHeadless(t *testing.T) {
	e := newEnv(t, oneServer)
	e.setenv("MCPX_POLICY_WRITE=ask", "MCPX_ELICIT_CONFIRM_DESTRUCTIVE=true", "MCPX_ELICIT_ASK_TIMEOUT=2s")

	out, err := e.try("call", "demo.structured", `{}`)
	if err == nil {
		t.Fatalf("an undeclared unannotated call should be gated:\n%s", out)
	}
	if !strings.Contains(out, "not confirmed") {
		t.Fatalf("the gate refusal should say it was not confirmed:\n%s", out)
	}
}
