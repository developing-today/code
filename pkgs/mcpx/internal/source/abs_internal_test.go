package source

import "testing"

func TestAbsExpandsOnlyBareTildeAndTildeSlash(t *testing.T) {
	// #1702 / #2988, rule pinned by triage task 1921 and mirrored from
	// searchpath's TestOnlyBareTildeAndTildeSlashExpandHome: only a bare
	// ~ and ~/... name the caller's home. A ~name path is not expanded --
	// it resolves literally under Dir -- because resolving it under the
	// caller's home silently reads a lookalike directory (and invents
	// one for users that do not exist), while a passwd lookup would
	// answer differently for the CLI user and the daemon user.
	t.Setenv("HOME", "/home/testhome")
	cases := []struct{ in, want string }{
		{"~", "/home/testhome"},
		{"~/x", "/home/testhome/x"},
		{"~alice/x", "/w/~alice/x"},
		{"~nosuchuser/x", "/w/~nosuchuser/x"},
		{"~scratch", "/w/~scratch"},
	}
	for _, c := range cases {
		if got := abs(c.in, "/w"); got != c.want {
			t.Errorf("abs(%q) = %q, want %q", c.in, got, c.want)
		}
	}
}
