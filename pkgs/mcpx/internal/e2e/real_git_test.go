//go:build realsoftware

package e2e_test

import (
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"testing"

	"github.com/developing-today/code/pkgs/mcpx/internal/testsupport"
)

// Real mcp-server-git (#6903): the pinned server must answer tools/list
// through mcpx and read a real repository created for the test — a
// vanished or broken git binary in the flake set must fail this subset
// (MCPX_TEST_REQUIRE=realsoftware), not pass silently behind the
// nightly's unasserted `ls`.
func TestRealGitServerTools(t *testing.T) {
	server := testsupport.RealServer(t, "mcp-server-git")

	repo := t.TempDir()
	git := func(args ...string) {
		t.Helper()
		cmd := exec.Command("git", args...)
		cmd.Dir = repo
		if out, err := cmd.CombinedOutput(); err != nil {
			t.Fatalf("git %v: %v\n%s", args, err, out)
		}
	}
	git("init")
	git("config", "user.email", "realsoftware@example.invalid")
	git("config", "user.name", "mcpx realsoftware subset")
	probe := filepath.Join(repo, "probe.txt")
	if err := os.WriteFile(probe, []byte("real software, pinned by the flake\n"), 0o644); err != nil {
		t.Fatal(err)
	}
	git("add", "probe.txt")
	git("commit", "-m", "realsoftware probe commit")

	e := newEnv(t, realConfig(server, "git", "the real mcp-server-git, pinned by the flake", "--repository", repo))
	out := e.run("ls")
	if !strings.Contains(out, "git") {
		t.Fatalf("namespace missing from ls:\n%s", out)
	}
	out = e.run("types", "git")
	for _, want := range []string{"git_status", "git_log"} {
		if !strings.Contains(out, want) {
			t.Errorf("types output missing real tool %q:\n%s", want, out)
		}
	}
	out = e.run("call", "git.git_log", `{"repo_path":`+jsonString(repo)+`}`)
	if !strings.Contains(out, "realsoftware probe commit") {
		t.Fatalf("git_log did not see the commit created for the test:\n%s", out)
	}
	out = e.run("call", "git.git_status", `{"repo_path":`+jsonString(repo)+`}`)
	if !strings.Contains(out, "probe.txt") && !strings.Contains(strings.ToLower(out), "clean") {
		t.Fatalf("git_status output does not describe the test repository:\n%s", out)
	}
}
