package cli

import (
	"context"
	"os"
	"path/filepath"
	"strconv"
	"strings"
	"testing"

	mcpx "github.com/dezren39/mcpx"
)

// firstDifference describes where two strings first disagree, for a test
// failure message that is useful instead of two multi-kilobyte dumps.
func firstDifference(a, b string) string {
	n := len(a)
	if len(b) < n {
		n = len(b)
	}
	for i := 0; i < n; i++ {
		if a[i] != b[i] {
			return "first difference at byte " + strconv.Itoa(i)
		}
	}
	return "one is a prefix of the other"
}

// TestHelpAgentsPrintsEmbeddedDocsVerbatim is the in-binary half of #1947:
// `mcpx help agents` prints the embedded AGENTS.md exactly -- no heading,
// no wrapping, no trailing additions -- so an agent holding only the
// binary reads the same document as one holding the repository.
func TestHelpAgentsPrintsEmbeddedDocsVerbatim(t *testing.T) {
	out, err := captureStdout(t, func() error {
		return (&App{}).CmdHelp(context.Background(), []string{"agents"})
	})
	if err != nil {
		t.Fatalf("CmdHelp(agents): %v", err)
	}
	if out != mcpx.AgentDocs {
		t.Errorf("help agents output (%d bytes) is not the embedded AGENTS.md (%d bytes): %s",
			len(out), len(mcpx.AgentDocs), firstDifference(out, mcpx.AgentDocs))
	}
	// The embedded bytes are the repository's AGENTS.md itself, not a
	// second copy that can drift: in a source checkout the two must agree
	// byte for byte.
	onDisk, err := os.ReadFile(filepath.Join("..", "..", "AGENTS.md"))
	if err != nil {
		t.Fatal(err)
	}
	if out != string(onDisk) {
		t.Errorf("help agents output (%d bytes) differs from pkgs/mcpx/AGENTS.md on disk (%d bytes): %s",
			len(out), len(onDisk), firstDifference(out, string(onDisk)))
	}
}

// TestUsageListsTheAgentsTopic: `mcpx help` names the topic, or an agent
// that only has the binary never learns it exists.
func TestUsageListsTheAgentsTopic(t *testing.T) {
	if !strings.Contains(Usage(), "mcpx help agents") {
		t.Error("Usage() does not list the `help agents` topic")
	}
}

// TestAgentsIsATopicNotACommand pins the shape of the feature: agents is
// reached through help, so it appears in no command table, dispatch map,
// man page or completion -- those are for commands, and `mcpx agents`
// (running the documentation) would be a different, unwanted thing.
func TestAgentsIsATopicNotACommand(t *testing.T) {
	for _, c := range Commands() {
		if c.Name == "agents" {
			t.Error("Commands() declares an agents command; agents is a help topic")
		}
	}
	if _, ok := (&App{}).Handlers()["agents"]; ok {
		t.Error("Handlers() dispatches an agents command; agents is a help topic")
	}
}
