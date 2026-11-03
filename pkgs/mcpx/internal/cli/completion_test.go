package cli

import (
	"strings"
	"testing"
)

// TestZshCompletionCommandsAreQuoted is the regression test for issue #272:
// `mcpx completion zsh | zsh -n` failed with "parse error near `;;'".
// Summaries carry apostrophes, parens and glob brackets, so every entry in
// the _describe commands array must be single-quoted; an unquoted '\” escape
// is only valid inside quotes.
func TestZshCompletionCommandsAreQuoted(t *testing.T) {
	out, err := Completion("zsh", "0.0.0-test")
	if err != nil {
		t.Fatal(err)
	}
	start := strings.Index(out, "  commands=(\n")
	if start < 0 {
		t.Fatal("commands array not found in zsh completion")
	}
	rest := out[start+len("  commands=(\n"):]
	end := strings.Index(rest, "\n  )\n")
	if end < 0 {
		t.Fatal("commands array not terminated in zsh completion")
	}
	lines := strings.Split(rest[:end], "\n")
	if len(lines) == 0 {
		t.Fatal("commands array is empty")
	}
	for _, l := range lines {
		if !strings.HasPrefix(l, "    '") || !strings.HasSuffix(l, "'") {
			t.Errorf("unquoted commands entry: %q", l)
		}
	}
	// At least one summary really does contain an apostrophe, so this test
	// guards the case that broke, not a vacuous shape.
	if !strings.Contains(out, `server'\''s prompts`) {
		t.Error("expected an apostrophe-bearing entry (server's prompts) in the commands array")
	}
}

// TestZshDescribeEscapesColonsAndQuotes pins zshDescribe: the entry is
// single-quoted for the shell, and _describe's own colon separator inside the
// description is backslash-escaped.
func TestZshDescribeEscapesColonsAndQuotes(t *testing.T) {
	got := zshDescribe("list a server's prompts: verbosely")
	want := `list a server'\''s prompts\: verbosely`
	if got != want {
		t.Errorf("zshDescribe = %q, want %q", got, want)
	}
}

// --- Alias coverage (#1661) ---
//
// The word-1 lists offer every spelling (Name + Aliases), but the flag
// arms used to key on the canonical Name only: an alias was offered
// and then refused its flags (bash fell back to filename completion,
// fish never offered the alias as a subcommand at all). Every spelling
// must carry the command's flags in all three shells.

// allSpellings is the test-local Name+Aliases expansion (kept local
// so these tests fail on assertions, not compilation, against the
// pre-fix generators).
func allSpellings(c Command) []string {
	out := []string{c.Name}
	return append(out, c.Aliases...)
}

func aliasedCommands(t *testing.T) []Command {
	t.Helper()
	var out []Command
	for _, c := range Commands() {
		if len(c.Aliases) > 0 {
			out = append(out, c)
		}
	}
	if len(out) == 0 {
		t.Fatal("no aliased commands in the table; the alias tests would be vacuous")
	}
	return out
}

// bashArmLabels maps each spelling in a bash case-arm label (`a|b)`)
// to the arm's flag list.
func bashArmLabels(t *testing.T, out string) map[string]string {
	t.Helper()
	arms := map[string]string{}
	for _, line := range strings.Split(out, "\n") {
		line = strings.TrimSpace(line)
		i := strings.Index(line, ") COMPREPLY=")
		if i < 0 {
			continue
		}
		for _, spelling := range strings.Split(line[:i], "|") {
			arms[spelling] = line
		}
	}
	return arms
}

func TestBashCompletionArmsCoverEverySpelling(t *testing.T) {
	out, err := Completion("bash", "0.0.0-test")
	if err != nil {
		t.Fatal(err)
	}
	// The equality guard: the word-1 spelling list and the case-arm
	// set must have the same membership — the invariant whose
	// violation is #1661.
	var word1 []string
	for _, line := range strings.Split(out, "\n") {
		if strings.HasPrefix(line, "  local commands=") {
			word1 = strings.Fields(strings.Trim(line[len("  local commands="):], `"`))
		}
	}
	if len(word1) == 0 {
		t.Fatal("bash word-1 commands list not found")
	}
	arms := bashArmLabels(t, out)
	if len(arms) != len(word1) {
		t.Errorf("bash arm set (%d) != word-1 list (%d)", len(arms), len(word1))
	}
	for _, w := range word1 {
		if _, ok := arms[w]; !ok {
			t.Errorf("spelling %q is offered at word 1 but has no flag arm", w)
		}
	}
	// Every alias shares its canonical command's arm (same flags).
	for _, c := range aliasedCommands(t) {
		for _, a := range c.Aliases {
			if arms[a] != arms[c.Name] {
				t.Errorf("alias %q does not share %q's bash arm", a, c.Name)
			}
		}
	}
}

func TestZshCompletionArmsCoverEverySpelling(t *testing.T) {
	out, err := Completion("zsh", "0.0.0-test")
	if err != nil {
		t.Fatal(err)
	}
	// Direct shape assertion per aliased command: its spellings appear
	// together as one case-arm label.
	for _, c := range aliasedCommands(t) {
		label := "    " + c.Name
		for _, a := range c.Aliases {
			label += "|" + a
		}
		label += ")\n      _arguments"
		if !strings.Contains(out, label) {
			t.Errorf("zsh completion has no _arguments arm labelled %q", label)
		}
	}
}

func TestFishCompletionCoversEverySpelling(t *testing.T) {
	out, err := Completion("fish", "0.0.0-test")
	if err != nil {
		t.Fatal(err)
	}
	for _, c := range aliasedCommands(t) {
		for _, spelling := range allSpellings(c) {
			if !strings.Contains(out, "-n __fish_use_subcommand -a "+spelling+" -d ") {
				t.Errorf("fish does not offer %q as a subcommand candidate", spelling)
			}
		}
		// Where the canonical name appears in a seen_subcommand_from
		// condition, every spelling must appear in that same
		// condition, or the flag rules never fire under the alias.
		for _, line := range strings.Split(out, "\n") {
			i := strings.Index(line, "__fish_seen_subcommand_from ")
			if i < 0 {
				continue
			}
			rest := line[i+len("__fish_seen_subcommand_from "):]
			cond := rest[:strings.Index(rest, "'")]
			fields := strings.Fields(cond)
			has := map[string]bool{}
			for _, f := range fields {
				has[f] = true
			}
			if has[c.Name] {
				for _, spelling := range allSpellings(c) {
					if !has[spelling] {
						t.Errorf("fish seen condition %q lacks spelling %q", cond, spelling)
					}
				}
			}
		}
	}
}

// The defect also hits generated op-group aliases (tasks, artifacts),
// not only the hand-written ls/log aliases: at least one operations
// command must be aliased, and its aliases must be armed in bash.
func TestCompletionCoversOpGroupAliases(t *testing.T) {
	out, err := Completion("bash", "0.0.0-test")
	if err != nil {
		t.Fatal(err)
	}
	arms := bashArmLabels(t, out)
	found := false
	for _, c := range Commands() {
		if c.Group != "operations" || len(c.Aliases) == 0 {
			continue
		}
		found = true
		for _, a := range c.Aliases {
			if arms[a] == "" || arms[a] != arms[c.Name] {
				t.Errorf("op-group alias %q of %q lacks its bash flag arm", a, c.Name)
			}
		}
	}
	if !found {
		t.Fatal("no aliased operations (op-group) command found; the op-group case would be vacuous")
	}
}
