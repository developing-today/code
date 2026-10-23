package cli

import (
	"os"
	"regexp"
	"strconv"
	"strings"
	"testing"
)

const (
	storyPath  = "../../docs/story.md"
	agentsPath = "../../AGENTS.md"
)

// TestStoryDocumentIsCurrent fails when docs/story.md's claims about
// AGENTS.md drift from the file itself: the line counts it states, or the
// block it presents as a quote of AGENTS.md. The quote is abridged, but
// every line it does quote must be a line of AGENTS.md verbatim, and the
// counts it states ("the full file is N lines", "N lines today") must be
// the file's real length. #365: both had silently rotted once already
// (109 stated, 120 actual), the same failure TestParityDocumentIsCurrent
// guards docs/parity.md against.
func TestStoryDocumentIsCurrent(t *testing.T) {
	agents, err := os.ReadFile(agentsPath)
	if err != nil {
		t.Fatal(err)
	}
	story, err := os.ReadFile(storyPath)
	if err != nil {
		t.Fatal(err)
	}

	agentLines := strings.Split(strings.TrimSuffix(string(agents), "\n"), "\n")
	inAgents := make(map[string]bool, len(agentLines))
	for _, l := range agentLines {
		inAgents[l] = true
	}

	// The quoted block: the first ```markdown fence in story.md.
	storyLines := strings.Split(string(story), "\n")
	start := -1
	for i, l := range storyLines {
		if l == "```markdown" {
			start = i + 1
			break
		}
	}
	if start < 0 {
		t.Fatal("docs/story.md has no ```markdown quote of AGENTS.md")
	}
	quoted := 0
	closed := false
	for _, l := range storyLines[start:] {
		if l == "```" {
			closed = true
			break
		}
		if strings.TrimSpace(l) == "" {
			continue
		}
		quoted++
		if !inAgents[l] {
			t.Errorf("docs/story.md quotes AGENTS.md as %q, which is not a line of AGENTS.md; re-sync the quote with the file", l)
		}
	}
	if !closed {
		t.Error("docs/story.md's AGENTS.md quote is never closed")
	}
	if quoted == 0 {
		t.Error("docs/story.md's AGENTS.md quote is empty")
	}

	// The stated line counts.
	for _, claim := range []struct {
		name string
		re   *regexp.Regexp
	}{
		{"the full file is", regexp.MustCompile(`the full file is (\d+) lines`)},
		{"lines today", regexp.MustCompile(`(\d+) lines today`)},
	} {
		for _, m := range claim.re.FindAllStringSubmatch(string(story), -1) {
			n, err := strconv.Atoi(m[1])
			if err != nil {
				t.Fatal(err)
			}
			if n != len(agentLines) {
				t.Errorf("docs/story.md says AGENTS.md is %d lines (%q claim); it is %d lines", n, claim.name, len(agentLines))
			}
		}
	}
}
