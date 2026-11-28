package cli

import (
	"strings"
	"testing"
)

// TestManPageDateFollowsSourceDateEpoch is the reproducibility guard for
// #1895: the page dates itself from SOURCE_DATE_EPOCH, so a pinned build
// reproduces it byte for byte instead of stamping the wall clock.
func TestManPageDateFollowsSourceDateEpoch(t *testing.T) {
	pages := map[string]string{}
	for _, c := range []struct{ epoch, want string }{
		{"1000000000", "2001-09-09"},
		{"1700000000", "2023-11-14"},
	} {
		t.Setenv("SOURCE_DATE_EPOCH", c.epoch)
		page := ManPage("1.2.3")
		want := `.TH MCPX 1 "` + c.want + `" "mcpx 1.2.3"`
		if !strings.HasPrefix(page, want) {
			t.Errorf("SOURCE_DATE_EPOCH=%s: man page header is %q, want prefix %q",
				c.epoch, headerLine(page), want)
		}
		pages[c.epoch] = page
	}
	if pages["1000000000"] == pages["1700000000"] {
		t.Error("man pages for two different SOURCE_DATE_EPOCH values are identical; the date is not taken from the epoch")
	}
}

// TestManPageDateWithoutEpochIsFixed covers plain `mcpx man` outside a
// reproducible build: the date is a fixed release date, never today.
func TestManPageDateWithoutEpochIsFixed(t *testing.T) {
	want := `.TH MCPX 1 "` + manDateFallback + `" "mcpx 1.2.3"`
	t.Setenv("SOURCE_DATE_EPOCH", "")
	if page := ManPage("1.2.3"); !strings.HasPrefix(page, want) {
		t.Errorf("man page header is %q, want prefix %q", headerLine(page), want)
	}
	// An unparseable epoch must not fall back to the clock either.
	t.Setenv("SOURCE_DATE_EPOCH", "not-a-number")
	if page := ManPage("1.2.3"); !strings.HasPrefix(page, want) {
		t.Errorf("with an unparseable SOURCE_DATE_EPOCH the header is %q, want prefix %q", headerLine(page), want)
	}
}

// TestGeneratedArtefactsCarryTheVersion proves the man page and all three
// completion scripts are non-empty and name the version they were generated
// for, which is what lets the build guard compare them with --version.
func TestGeneratedArtefactsCarryTheVersion(t *testing.T) {
	const version = "9.9.9-test"
	page := ManPage(version)
	if len(page) < 1000 {
		t.Fatalf("man page is %d bytes; expected a full page", len(page))
	}
	if !strings.Contains(page, "mcpx "+version) {
		t.Errorf("man page does not name version %q", version)
	}
	for _, shell := range []string{"bash", "zsh", "fish"} {
		out, err := Completion(shell, version)
		if err != nil {
			t.Fatalf("Completion(%q): %v", shell, err)
		}
		if len(out) < 100 {
			t.Errorf("%s completion is %d bytes; expected a full script", shell, len(out))
		}
		if !strings.Contains(out, version) {
			t.Errorf("%s completion does not name version %q", shell, version)
		}
	}
}

// TestEveryCommandAppearsInGeneratedArtefacts is the presence guard the
// issue asks for: both generators read Commands(), so a command missing from
// the table is missing from the man page and from every completion, and
// nothing else would notice (#1895, #159).
func TestEveryCommandAppearsInGeneratedArtefacts(t *testing.T) {
	cmds := Commands()
	if len(cmds) == 0 {
		t.Fatal("Commands() is empty; the generators would produce nothing")
	}
	page := ManPage("1.2.3")
	for _, c := range cmds {
		if !strings.Contains(page, "mcpx "+c.Name) {
			t.Errorf("man page has no entry for command %q", c.Name)
		}
		for _, shell := range []string{"bash", "zsh", "fish"} {
			out, err := Completion(shell, "1.2.3")
			if err != nil {
				t.Fatalf("Completion(%q): %v", shell, err)
			}
			if !strings.Contains(out, c.Name) {
				t.Errorf("%s completion does not offer command %q", shell, c.Name)
			}
		}
	}
}

func headerLine(s string) string {
	if i := strings.IndexByte(s, '\n'); i >= 0 {
		return s[:i]
	}
	return s
}
