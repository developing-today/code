package execsvc

import "testing"

// PM-12 (#1942): scriptEnv must mark a run ephemeral when the caller did
// not name a session, so the generated client sends x-mcpx-ephemeral and
// session-scoped instances are released at the end of a /v1/exec run
// instead of waiting for idle reaping. On the pre-fix code the key is
// absent entirely and every assertion below fails.
func TestScriptEnvMarksEphemeralUnlessSessionNamed(t *testing.T) {
	s := &Service{}
	env := s.scriptEnv("run-1", "exec-run-1", Options{})
	if got := env["MCPX_EPHEMERAL"]; got != "1" {
		t.Errorf("unnamed session: MCPX_EPHEMERAL = %q, want \"1\"", got)
	}
	env = s.scriptEnv("run-2", "named", Options{Session: "named"})
	if got := env["MCPX_EPHEMERAL"]; got != "0" {
		t.Errorf("named session: MCPX_EPHEMERAL = %q, want \"0\" (a named session may be shared)", got)
	}
	// An explicit caller value in Options.Env wins over the default.
	env = s.scriptEnv("run-3", "exec-run-3", Options{Env: map[string]string{"MCPX_EPHEMERAL": "0"}})
	if got := env["MCPX_EPHEMERAL"]; got != "0" {
		t.Errorf("explicit env: MCPX_EPHEMERAL = %q, want \"0\"", got)
	}
}
