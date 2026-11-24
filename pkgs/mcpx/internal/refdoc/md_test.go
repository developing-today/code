package refdoc_test

import (
	"strings"
	"testing"

	"github.com/developing-today/code/pkgs/mcpx/internal/refdoc"
)

func TestTableEscapesPipesAndNewlines(t *testing.T) {
	got := refdoc.Table(
		[]string{"setting", "governs"},
		[][]string{
			{"`a.b`", "x | y"},
			{"`c.d`", "line one\nline two"},
		},
	)
	want := "| setting | governs |\n" +
		"| --- | --- |\n" +
		"| `a.b` | x \\| y |\n" +
		"| `c.d` | line one<br>line two |\n"
	if got != want {
		t.Errorf("Table mismatch:\n got: %q\nwant: %q", got, want)
	}
}

func TestTablePadsShortRows(t *testing.T) {
	got := refdoc.Table([]string{"a", "b"}, [][]string{{"1"}})
	if !strings.Contains(got, "| 1 |  |\n") {
		t.Errorf("short row not padded: %q", got)
	}
}

func TestCodeFenceOutrunsBackticksInContent(t *testing.T) {
	got := refdoc.Code("before\n```\ninside\n```\nafter")
	if !strings.HasPrefix(got, "````\n") || !strings.HasSuffix(got, "\n````\n") {
		t.Errorf("fence did not outrun the content's fence: %q", got)
	}
	plain := refdoc.Code("plain")
	if plain != "```\nplain\n```\n" {
		t.Errorf("plain code block wrong: %q", plain)
	}
}

func TestRegionWrapsInAnchorComments(t *testing.T) {
	got := refdoc.Region("daemon", "### daemon\n\nbody\n\n")
	want := "<!-- ANCHOR: daemon -->\n### daemon\n\nbody\n<!-- ANCHOR_END: daemon -->\n"
	if got != want {
		t.Errorf("Region mismatch:\n got: %q\nwant: %q", got, want)
	}
}

func TestRenderingIsDeterministic(t *testing.T) {
	render := func() string {
		return refdoc.Region("x", refdoc.Table([]string{"a"}, [][]string{{"1"}, {"2"}})) +
			refdoc.Code("tick ` here")
	}
	first, second := render(), render()
	if first != second {
		t.Error("same input rendered different bytes across runs")
	}
}

func TestTableEscapesAnglePlaceholdersInProse(t *testing.T) {
	got := refdoc.Table(
		[]string{"setting", "governs"},
		[][]string{
			{"`a.b`", "visible via mcpx elicit result <id>"},
			{"`c.d`", "host, or `flake:<ref>` for a Nix flake devshell"},
		},
	)
	want := "| setting | governs |\n" +
		"| --- | --- |\n" +
		"| `a.b` | visible via mcpx elicit result &lt;id&gt; |\n" +
		"| `c.d` | host, or `flake:<ref>` for a Nix flake devshell |\n"
	if got != want {
		t.Errorf("Table mismatch:\n got: %q\nwant: %q", got, want)
	}
}

func TestEscapeAngles(t *testing.T) {
	cases := []struct{ in, want string }{
		{"plain text", "plain text"},
		{"a <server> b", "a &lt;server&gt; b"},
		{"`<server>` in code", "`<server>` in code"},
		{"``a ` <b>`` and <c>", "``a ` <b>`` and &lt;c&gt;"},
		{"already &lt;escaped&gt;", "already &lt;escaped&gt;"},
		{"a lone ` backtick, then <x>", "a lone ` backtick, then &lt;x&gt;"},
	}
	for _, c := range cases {
		if got := refdoc.EscapeAngles(c.in); got != c.want {
			t.Errorf("EscapeAngles(%q) = %q, want %q", c.in, got, c.want)
		}
	}
}
