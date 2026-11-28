package tui

import (
	"strings"
	"testing"
	"unicode/utf8"

	"github.com/mattn/go-runewidth"
)

// Regression tests for #1663: the tui helpers measured in bytes, so
// truncation could split a multi-byte rune (invalid UTF-8 on the
// terminal) and wide-rune rows misaligned every table. Widths are
// terminal cells now.

func TestTruncateMultiByteStaysValidUTF8(t *testing.T) {
	s := "héllo wörld, this is a long line"
	for n := 0; n <= len(s); n++ {
		out := truncate(s, n)
		if !utf8.ValidString(out) {
			t.Fatalf("truncate(%q, %d) = %q: invalid UTF-8", s, n, out)
		}
		if w := runewidth.StringWidth(out); w > n {
			t.Fatalf("truncate(%q, %d) = %q: %d cells exceeds budget", s, n, out, w)
		}
	}
}

func TestTruncateWideRuneDroppedWhole(t *testing.T) {
	// Each CJK rune is 2 cells; a budget of 4 leaves 3 for content after
	// the ellipsis, so exactly one wide rune fits and the second is
	// dropped whole rather than halved.
	out := truncate("你好世界", 4)
	if out != "你…" {
		t.Fatalf("truncate wide = %q, want %q", out, "你…")
	}
	if !utf8.ValidString(out) {
		t.Fatalf("truncate wide = %q: invalid UTF-8", out)
	}
}

func TestTruncateControls(t *testing.T) {
	if got := truncate("short", 10); got != "short" {
		t.Fatalf("under-budget truncate changed the string: %q", got)
	}
	if got := truncate("a longer string", 6); got != "a lon…" {
		t.Fatalf("ascii truncate = %q, want %q", got, "a lon…")
	}
	if got := truncate("abc", 1); got != "…" {
		t.Fatalf("budget-1 truncate = %q, want ellipsis", got)
	}
	if got := truncate("abc", 0); got != "" {
		t.Fatalf("budget-0 truncate = %q, want empty", got)
	}
}

func TestPadCountsCellsNotBytes(t *testing.T) {
	if got := pad("你好", 6); got != "你好  " { // 4 cells + 2 spaces
		t.Fatalf("pad wide = %q, want %q", got, "你好  ")
	}
	if got := pad("héllo", 8); got != "héllo   " { // 5 cells + 3 spaces
		t.Fatalf("pad accented = %q, want %q", got, "héllo   ")
	}
	if got := pad("already wide enough", 4); got != "already wide enough" {
		t.Fatalf("pad over-budget changed the string: %q", got)
	}
}

func TestRenderMixedWidthRowsAlign(t *testing.T) {
	tbl := Table{
		Columns: []string{"Name", "Tool"},
		Rows: [][]string{
			{"ascii", "read"},
			{"héllo", "écriture"},
			{"你好世界", "工具工具"},
		},
	}
	plain := func(s string) string { return s }
	out := tbl.Render(80, -1, plain, plain)
	lines := strings.Split(strings.TrimRight(out, "\n"), "\n")
	if len(lines) != 4 {
		t.Fatalf("rendered %d lines, want 4:\n%s", len(lines), out)
	}
	want := runewidth.StringWidth(lines[0])
	for i, line := range lines[1:] {
		if w := runewidth.StringWidth(line); w != want {
			t.Fatalf("row %d is %d cells, header is %d: %q", i, w, want, line)
		}
		if !utf8.ValidString(line) {
			t.Fatalf("row %d is invalid UTF-8: %q", i, line)
		}
	}
}

func TestRenderTruncationStaysValidUTF8(t *testing.T) {
	tbl := Table{
		Columns: []string{"Name"},
		Rows:    [][]string{{"你好世界你好世界"}},
	}
	plain := func(s string) string { return s }
	out := tbl.Render(10, -1, plain, plain)
	for i, line := range strings.Split(out, "\n") {
		if !utf8.ValidString(line) {
			t.Fatalf("line %d invalid UTF-8 after shrink+truncate: %q", i, line)
		}
	}
}
