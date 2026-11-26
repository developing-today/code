package daemon_test

import (
	"os"
	"os/exec"
	"path/filepath"
	"testing"
)

// Oracle: running real WriteInfo and Server.Listen fixtures in another test
// process must leave the caller's daemon-record directory byte-for-byte intact.
// These are the /tmp/sock pid=42 and /tmp/mxh*/d.sock writers seen by e2e's
// independent before/after inventory guard in #14332.
func TestDaemonSuiteLeavesCallerStateUntouched(t *testing.T) {
	for _, fixture := range []string{
		"TestInfoFileNamesBothDigests",
		"TestUpgradeWithAMissingBinaryKeepsServing",
	} {
		t.Run(fixture, func(t *testing.T) {
			root := t.TempDir()
			state := filepath.Join(root, "mcpx")
			if err := os.MkdirAll(state, 0700); err != nil {
				t.Fatal(err)
			}
			sentinel := []byte("caller-owned record\n")
			path := filepath.Join(state, "daemon-caller.json")
			if err := os.WriteFile(path, sentinel, 0600); err != nil {
				t.Fatal(err)
			}
			cmd := exec.Command(os.Args[0], "-test.run=^"+fixture+"$", "-test.count=1")
			cmd.Env = append(os.Environ(), "XDG_STATE_HOME="+root, "MCPX_STATE_DIR="+state)
			if output, err := cmd.CombinedOutput(); err != nil {
				t.Fatalf("fixture failed: %v\n%s", err, output)
			}
			entries, err := os.ReadDir(state)
			if err != nil {
				t.Fatal(err)
			}
			if len(entries) != 1 || entries[0].Name() != filepath.Base(path) {
				t.Fatalf("daemon fixture polluted caller inventory: %v", entries)
			}
			got, err := os.ReadFile(path)
			if err != nil || string(got) != string(sentinel) {
				t.Fatalf("caller record changed: %q, %v", got, err)
			}
		})
	}
}
