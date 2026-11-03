package conformance_test

import (
	"os"
	"path/filepath"
	"testing"

	"github.com/developing-today/code/pkgs/mcpx/internal/conformance"
)

// The #1950 OAuth decision, expressed per row: a catalogue note that waives
// an authorization requirement by choice must survive derivation (it used
// to be flattened into the blanket gap), and an unwaived authorization gap
// must point at the live issue for its side — never at #253, the issue's
// pre-relocation dezren39/nix number.
func TestAuthorizationApplicabilityFollowsTheRecordedDecision(t *testing.T) {
	md := `# Catalogue fixture

| id | revs | side | level | area | quote | url | server note | client note | extra |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| auth-waived | 2025-03-26 | both | SHOULD | authorization | "q" | 2025-03-26/basic/authorization | not-implemented-by-choice (no OAuth; loopback only) | not-implemented-by-choice (no OAuth to upstreams) | |
| auth-open | 2025-03-26 | both | MUST | authorization | "q" | 2025-03-26/basic/authorization | yes | no | |
| auth-na | 2025-03-26 | both | MAY | authorization | "q" | 2025-03-26/basic/authorization | n/a | n/a (stdio only) | |
`
	path := filepath.Join(t.TempDir(), "requirements.md")
	if err := os.WriteFile(path, []byte(md), 0o644); err != nil {
		t.Fatal(err)
	}
	reqs, err := conformance.Parse(path)
	if err != nil {
		t.Fatal(err)
	}
	if len(reqs) != 3 {
		t.Fatalf("parsed %d requirements, want 3", len(reqs))
	}
	byID := map[string]conformance.Requirement{}
	for _, r := range reqs {
		byID[r.ID] = r
	}
	waived := "n/a:optional feature mcpx does not implement"
	if got := byID["auth-waived"].Applicability[conformance.Server]; got != waived {
		t.Errorf("server waiver = %q, want %q", got, waived)
	}
	if got := byID["auth-waived"].Applicability[conformance.Client]; got != waived {
		t.Errorf("client waiver = %q, want %q", got, waived)
	}
	if got := byID["auth-open"].Applicability[conformance.Server]; got != "not-implemented:#1975" {
		t.Errorf("server gap = %q, want not-implemented:#1975 (the /mcp resource-server issue)", got)
	}
	if got := byID["auth-open"].Applicability[conformance.Client]; got != "not-implemented:#1950" {
		t.Errorf("client gap = %q, want not-implemented:#1950 (the deferred client OAuth umbrella)", got)
	}
	if got := byID["auth-na"].Applicability[conformance.Server]; got != "n/a: catalogue marks it n/a" && got[:4] != "n/a:" {
		t.Errorf("server n/a note = %q, want an n/a applicability", got)
	}
	for id, r := range byID {
		for side, a := range r.Applicability {
			if a == "not-implemented:#253" {
				t.Errorf("%s %s still points at the stale nix number #253", id, side)
			}
		}
	}
}
