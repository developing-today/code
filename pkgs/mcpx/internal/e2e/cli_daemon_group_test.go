package e2e_test

import (
	"encoding/json"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

// TestReloadPicksUpAServerAddedOnDisk exercises `mcpx reload` (#8461). With
// the config watcher off, a server written into the config file is not
// seen until reload re-reads it; reload then names it as added and ls
// lists it beside the server that was already running.
func TestReloadPicksUpAServerAddedOnDisk(t *testing.T) {
	e := newEnv(t, oneServer)
	e.setenv("MCPX_DAEMON_WATCH_CONFIG=false")
	if out := e.run("ls"); strings.Contains(out, "beta") {
		t.Fatalf("beta is listed before it is configured:\n%s", out)
	}
	// beta differs from demo in its environment, so it is a distinct child
	// rather than a second name for demo's.
	cfg := strings.ReplaceAll(`{"mcpServers":{`+
		`"demo":{"command":"FAKE","mcpx":{"sharing":"shared","scope":"global","description":"a fake server"}},`+
		`"beta":{"command":"FAKE","env":{"MCPX_E2E_BETA":"1"}}}}`, "FAKE", e.fake)
	if err := os.WriteFile(filepath.Join(e.dir, ".mcpx.json"), []byte(cfg), 0o644); err != nil {
		t.Fatal(err)
	}
	if out := e.run("ls"); strings.Contains(out, "beta") {
		t.Fatalf("with the watcher off, beta should wait for reload:\n%s", out)
	}
	var res struct {
		Added   []string `json:"added"`
		Removed []string `json:"removed"`
	}
	out := e.run("--json", "reload")
	if err := json.Unmarshal([]byte(out), &res); err != nil {
		t.Fatalf("reload --json is not a reload result (%v):\n%s", err, out)
	}
	if len(res.Added) != 1 || res.Added[0] != "beta" || len(res.Removed) != 0 {
		t.Fatalf("reload should add beta and remove nothing, got added %v removed %v", res.Added, res.Removed)
	}
	if out := e.run("ls"); !strings.Contains(out, "beta") || !strings.Contains(out, "demo") {
		t.Fatalf("after reload both servers should be listed:\n%s", out)
	}
}

// TestUpgradeToTheRunningBinaryIsAlreadyCurrent exercises `mcpx upgrade`
// (#8461) from the binary the daemon is already running: there is nothing
// to hand over, so it says the daemon is current and succeeds.
func TestUpgradeToTheRunningBinaryIsAlreadyCurrent(t *testing.T) {
	e := newEnv(t, oneServer)
	e.run("ls")
	if out := e.run("upgrade"); !strings.Contains(out, "already current") {
		t.Fatalf("upgrading to the running binary should report it current:\n%s", out)
	}
	if out := e.run("ls"); !strings.Contains(out, "demo") {
		t.Fatalf("the daemon should still serve after a no-op upgrade:\n%s", out)
	}
}
