package conformance

import "testing"

// The matrix's gap and n/a details are prose written in the coverage
// files, and they name commands with placeholder arguments
// ("mcpx elicit result <id>"). mdEscape must turn those placeholders
// into entities the book renderer shows instead of eating them as
// HTML tags (#13422), while leaving code spans — the house style for
// a placeholder that must stay literal — untouched.
func TestMdEscapeAngles(t *testing.T) {
	cases := []struct{ in, want string }{
		{"visible via mcpx elicit result <id>", "visible via mcpx elicit result &lt;id&gt;"},
		{"host, or `flake:<ref>` for a devshell", "host, or `flake:<ref>` for a devshell"},
		{"pipes | stay escaped", `pipes \| stay escaped`},
	}
	for _, c := range cases {
		if got := mdEscape(c.in); got != c.want {
			t.Errorf("mdEscape(%q) = %q, want %q", c.in, got, c.want)
		}
	}
}
