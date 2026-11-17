package pool_test

import (
	"encoding/json"
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/developing-today/code/pkgs/mcpx/internal/config"
	"github.com/developing-today/code/pkgs/mcpx/internal/mcpauth"
	"github.com/developing-today/code/pkgs/mcpx/internal/pool"
	"github.com/developing-today/code/pkgs/mcpx/internal/testsupport"
)

// Oracle tests for the #8017 daemon call-path wiring.
//
// The oracle is a scripted fake whose answer depends on the
// credential its process was spawned with (the GH_TOKEN env the
// failover auth resolution hands it):
//
//   - spawned with token-one (identity "first") the tool "ghcall"
//     answers an error result carrying a GitHub secondary-limit
//     soft block, and "ghhard" answers a marker-less 403 hard block;
//   - spawned with token-two (identity "second") both answer
//     "served-by-second".
//
// A correct wiring therefore shows: ghcall succeeds with
// "served-by-second" (the soft block was translated, the identity
// rotated, the instance respawned with the rotated env, and the
// call retried invisibly), while ghhard returns its hard block
// unchanged and the persisted rotation state still names "first".

func failoverPool(t *testing.T) (*pool.Pool, *config.Resolved, string) {
	t.Helper()
	bin := testsupport.FakeMCPBinary(t)
	dir := t.TempDir()
	statePath := filepath.Join(dir, "failover-state.json")
	scriptPath := filepath.Join(dir, "fake-script.json")
	script := `{"tools":[
		{"name":"ghcall","description":"soft-blocks on the first identity","inputSchema":{"type":"object","properties":{}},
		 "response":{"ifEnv":{"name":"GH_TOKEN","equals":"token-one"},
		             "isError":true,
		             "text":"failed: POST https://api.github.com/repos/o/r/issues: 403 You have exceeded a secondary rate limit. submitted too quickly, please retry later",
		             "else":{"ifEnv":{"name":"GH_TOKEN","equals":"token-two"},
		                     "text":"served-by-second",
		                     "else":{"text":"served-by-other"}}}},
		{"name":"ghhard","description":"hard-blocks on the first identity","inputSchema":{"type":"object","properties":{}},
		 "response":{"ifEnv":{"name":"GH_TOKEN","equals":"token-one"},
		             "isError":true,
		             "text":"403 Forbidden: resource not accessible by integration",
		             "else":{"text":"served-by-second"}}}
	]}`
	if err := os.WriteFile(scriptPath, []byte(script), 0o600); err != nil {
		t.Fatal(err)
	}
	t.Setenv("MCPX_TEST_ID1_SECRET", "token-one")
	t.Setenv("MCPX_TEST_ID2_SECRET", "token-two")
	cfg := &config.Config{MCPServers: map[string]*config.Server{
		"fake": {
			Name:    "fake",
			Command: bin,
			Env:     map[string]string{"FAKEMCP_SCRIPT_FILE": scriptPath},
			Auth: &mcpauth.Auth{
				Type: "env",
				Env:  map[string]string{"GH_TOKEN": ""},
				Failover: &mcpauth.Failover{
					Identities: []mcpauth.Identity{
						{Name: "first", Kind: "token", Secret: "MCPX_TEST_ID1_SECRET"},
						{Name: "second", Kind: "token", Secret: "MCPX_TEST_ID2_SECRET"},
					},
					StatePath:       statePath,
					CooldownSeconds: 60,
				},
			},
			Mcpx: &config.Extras{Sharing: config.SharingShared, Scope: config.ScopeGlobal},
		},
	}}
	r, err := cfg.Resolve("fake")
	if err != nil {
		t.Fatalf("resolve: %v", err)
	}
	p := pool.New(r)
	t.Cleanup(p.Close)
	return p, r, statePath
}

func rotationCurrent(t *testing.T, statePath string) string {
	t.Helper()
	b, err := os.ReadFile(statePath)
	if err != nil {
		t.Fatalf("read rotation state: %v", err)
	}
	var s struct {
		Current int `json:"current"`
	}
	if err := json.Unmarshal(b, &s); err != nil {
		t.Fatalf("decode rotation state: %v (%s)", err, b)
	}
	return []string{"first", "second"}[s.Current]
}

func TestFailoverCallSoftBlockRotatesRespawnsAndRetries(t *testing.T) {
	p, r, statePath := failoverPool(t)
	res, err := call(t, p, r, config.CallContext{SessionID: "s1", CallID: "c1"}, "ghcall", map[string]any{})
	if err != nil {
		t.Fatalf("call: %v", err)
	}
	if got := textOf(t, res); got != "served-by-second" {
		t.Fatalf("oracle: result = %q, want %q -- the soft block must rotate, respawn and retry invisibly", got, "served-by-second")
	}
	if got := rotationCurrent(t, statePath); got != "second" {
		t.Errorf("oracle: persisted rotation current = %q, want %q", got, "second")
	}
}

func TestFailoverCallHardBlockSurfacesUnchanged(t *testing.T) {
	p, r, statePath := failoverPool(t)
	res, err := call(t, p, r, config.CallContext{SessionID: "s1", CallID: "c1"}, "ghhard", map[string]any{})
	if err != nil {
		t.Fatalf("call: %v", err)
	}
	text, isError, ok := mcpauth.ToolResultText(res)
	if !ok || !isError {
		t.Fatalf("oracle: hard block result = %s, want the original isError result returned unchanged", res)
	}
	if !strings.Contains(text, "resource not accessible by integration") {
		t.Errorf("oracle: hard block text = %q, want the upstream text unchanged", text)
	}
	if _, err := os.Stat(statePath); !os.IsNotExist(err) {
		b, _ := os.ReadFile(statePath)
		var s struct {
			Current int `json:"current"`
		}
		_ = json.Unmarshal(b, &s)
		if s.Current != 0 {
			t.Errorf("oracle: hard block rotated the identity (current=%d); want no rotation", s.Current)
		}
	}
}
