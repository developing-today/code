package execsvc

import "testing"

// #1975 (Codex P2 on PR #2711): a daemon that resolved its bearer token
// from a config file or flag must hand that token to the scripts it
// hosts, or every call the generated client makes back to the daemon
// is a 401. scriptEnv is where that hand-off happens.
func TestScriptEnvCarriesBearerTokenWhenConfigured(t *testing.T) {
	const tok = "0123456789abcdef0123456789abcdef"
	s := &Service{BearerToken: tok}
	env := s.scriptEnv("run-1", "exec-run-1", Options{})
	if got := env["MCPX_TRANSPORT_BEARER_TOKEN"]; got != tok {
		t.Errorf("configured: MCPX_TRANSPORT_BEARER_TOKEN = %q, want the resolved token", got)
	}
	// An explicit caller value in Options.Env wins, as with MCPX_EPHEMERAL.
	env = s.scriptEnv("run-2", "exec-run-2", Options{Env: map[string]string{"MCPX_TRANSPORT_BEARER_TOKEN": "caller"}})
	if got := env["MCPX_TRANSPORT_BEARER_TOKEN"]; got != "caller" {
		t.Errorf("explicit env: MCPX_TRANSPORT_BEARER_TOKEN = %q, want %q", got, "caller")
	}
	// No token configured: the key stays absent, so an exported
	// environment value (or its absence) is untouched.
	s = &Service{}
	env = s.scriptEnv("run-3", "exec-run-3", Options{})
	if _, ok := env["MCPX_TRANSPORT_BEARER_TOKEN"]; ok {
		t.Errorf("unconfigured: MCPX_TRANSPORT_BEARER_TOKEN present, want absent")
	}
}
