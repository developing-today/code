package e2e_test

// Integration-test exercises for the CLI running group (#8465):
// embeddings, feedback, proxy, scripts, skills. Each test drives the
// real command through the harness so the feature-coverage ledger
// gains a row for it, which is what lets the matching line come out
// of feature_coverage_exemptions.txt (the guard fails on an
// exemption for a covered feature, so the deletions are
// self-verifying).
//
// Careful: an exercise must not record ledger rows for features that
// still carry exemptions of their own. The feedback test therefore
// reads its trace id through tryUnrecorded — `interactions` belongs
// to the #8463 inspection group and stays exempt until that group
// burns down. The `call` it rates is already covered elsewhere.

import (
	"os"
	"path/filepath"
	"strings"
	"testing"
)

func TestCoverageEmbeddingsTestCommand(t *testing.T) {
	e := newEnv(t, oneServer)
	out := e.run("embeddings", "test", "hello coverage")
	if !strings.Contains(out, "Vector Dimension:") {
		t.Fatalf("embeddings test did not report a vector:\n%s", out)
	}
	if !strings.Contains(out, "Backend Model:") {
		t.Fatalf("embeddings test did not report its backend:\n%s", out)
	}
}

func TestCoverageProxyCommand(t *testing.T) {
	e := newEnv(t, oneServer)
	out := e.run("proxy")
	if !strings.Contains(out, "OpenAI-compatible Base URL:") {
		t.Fatalf("proxy did not report its base URL:\n%s", out)
	}
	if !strings.Contains(out, "/chat/completions") {
		t.Fatalf("proxy did not report its chat completions endpoint:\n%s", out)
	}
}

func TestCoverageScriptsListsDiscoveredScript(t *testing.T) {
	e := newEnv(t, oneServer)
	// Script discovery walks up from the working directory (the env
	// dir for harness invocations) and reads <dir>/.config/mcpx/scripts.
	dir := filepath.Join(e.dir, ".config", "mcpx", "scripts")
	if err := os.MkdirAll(dir, 0o755); err != nil {
		t.Fatal(err)
	}
	script := "// coverage probe for the scripts command\n"
	if err := os.WriteFile(filepath.Join(dir, "coverageprobe.ts"), []byte(script), 0o644); err != nil {
		t.Fatal(err)
	}
	out := e.run("scripts")
	if !strings.Contains(out, "coverageprobe") {
		t.Fatalf("scripts did not list the discovered script:\n%s", out)
	}
}

func TestCoverageSkillsListAndInstall(t *testing.T) {
	e := newEnv(t, oneServer)
	out := e.run("skills")
	if !strings.Contains(out, "SKILL") {
		t.Fatalf("skills list did not render its table:\n%s", out)
	}
	dest := filepath.Join(e.dir, "installed-skills")
	e.run("skills", "install", "--to", dest)
	found := 0
	err := filepath.WalkDir(dest, func(path string, d os.DirEntry, err error) error {
		if err == nil && !d.IsDir() && d.Name() == "SKILL.md" {
			found++
		}
		return nil
	})
	if err != nil {
		t.Fatalf("walking installed skills: %v", err)
	}
	if found == 0 {
		t.Fatalf("skills install wrote no SKILL.md under %s", dest)
	}
}

func TestCoverageFeedbackRatesTrace(t *testing.T) {
	e := newEnv(t, oneServer)
	// The feedback store rates the trace it is given: an existing
	// interaction is updated in place, and a trace the daemon has not
	// recorded yet gets a stub row (logstore.ProvideFeedback) — which
	// is the flow this exercise drives end to end. The rating is then
	// read back through `interactions --show`, unrecorded on purpose:
	// `interactions` is #8463's exemption, not this group's.
	const traceID = "e2e-feedback-8465"
	// Flags precede the trace id: the command's flag set stops
	// parsing at the first positional argument, and the distinctive
	// 0.9 score below proves the flags — not the 1.0 default — landed.
	out := e.run("feedback", "--score", "0.9", "--target", "execution",
		"--notes", "e2e coverage exercise #8465", traceID)
	if !strings.Contains(out, "Feedback recorded successfully") {
		t.Fatalf("feedback did not confirm recording:\n%s", out)
	}
	if !strings.Contains(out, "score: 0.90") {
		t.Fatalf("feedback did not record the given score:\n%s", out)
	}

	shown, err := e.tryUnrecorded("interactions", "--show", traceID)
	if err != nil {
		t.Fatalf("interactions --show failed: %v\n%s", err, shown)
	}
	if !strings.Contains(shown, "Score:        0.90") {
		t.Fatalf("rated trace does not show its score:\n%s", shown)
	}
	if !strings.Contains(shown, "Target:       execution") {
		t.Fatalf("rated trace does not show its target:\n%s", shown)
	}
	if !strings.Contains(shown, "e2e coverage exercise #8465") {
		t.Fatalf("rated trace does not show its feedback notes:\n%s", shown)
	}
}
