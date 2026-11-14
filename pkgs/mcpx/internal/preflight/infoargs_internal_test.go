package preflight

import (
	"strings"
	"testing"
)

// #8457: --cached-only must never reach `deno info` verbatim
// (info rejects it); it is translated to --no-remote, its
// no-fetch equivalent on the info surface, without disturbing
// the other forwarded flags or duplicating --no-remote.
func TestInfoArgsTranslatesCachedOnlyForDenoInfo(t *testing.T) {
	in := []string{"--cached-only", "--config=deno.json", "--frozen"}
	want := []string{"--no-remote", "--config=deno.json", "--frozen"}
	if got := infoArgs(in); strings.Join(got, " ") != strings.Join(want, " ") {
		t.Fatalf("infoArgs = %v, want %v", got, want)
	}
	both := []string{"--cached-only", "--no-remote"}
	if got := infoArgs(both); len(got) != 1 || got[0] != "--no-remote" {
		t.Fatalf("infoArgs(both spellings) = %v, want a single --no-remote", got)
	}
	plain := []string{"--frozen"}
	if got := infoArgs(plain); len(got) != 1 || got[0] != "--frozen" {
		t.Fatalf("infoArgs(plain) = %v, want [--frozen]", got)
	}
}
