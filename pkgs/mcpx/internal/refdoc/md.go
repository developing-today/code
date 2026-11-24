// Package refdoc renders the Markdown fragments the generated
// reference pages are built from (#1846 step 1).
//
// It is a leaf package: it imports nothing from mcpx, so any package
// that renders reference data can use it without an import cycle.
// Everything it emits is deterministic — same input, same bytes, on
// every run — because the golden drift test (slice 4) compares its
// output byte for byte, and because a reference page must never
// contain resolved values, only the schema it was generated from.
//
// The output contains no generator syntax. A Region is an HTML
// comment pair: mdBook strips any line containing "ANCHOR:" from an
// include, and GitHub hides HTML comments, so one file serves both
// the book and a raw read.
package refdoc

import "strings"

// Table renders a Markdown table with the given headers and rows.
// Cells are escaped: a "|" becomes "\|", angle brackets in prose
// become "&lt;"/"&gt;" (see EscapeAngles) and a newline becomes
// "<br>", so a cell can never break the table's shape or lose a
// placeholder token to the HTML renderer. Rows shorter than the
// header are padded with empty cells.
func Table(headers []string, rows [][]string) string {
	var b strings.Builder
	writeRow := func(cells []string) {
		b.WriteString("| ")
		for i, c := range cells {
			if i > 0 {
				b.WriteString(" | ")
			}
			b.WriteString(escapeCell(c))
		}
		b.WriteString(" |\n")
	}
	writeRow(headers)
	sep := make([]string, len(headers))
	for i := range sep {
		sep[i] = "---"
	}
	writeRow(sep)
	for _, row := range rows {
		cells := append([]string(nil), row...)
		for len(cells) < len(headers) {
			cells = append(cells, "")
		}
		writeRow(cells)
	}
	return b.String()
}

func escapeCell(s string) string {
	s = EscapeAngles(s)
	s = strings.ReplaceAll(s, "|", `\|`)
	s = strings.ReplaceAll(s, "\r\n", "<br>")
	s = strings.ReplaceAll(s, "\n", "<br>")
	s = strings.ReplaceAll(s, "\r", "<br>")
	return s
}

// EscapeAngles escapes "<" and ">" as the HTML entities &lt; and
// &gt; in prose, so a placeholder token like <id> survives rendering
// instead of being eaten as an HTML tag (an mdBook build warns about
// the unclosed tag, and the placeholder vanishes from the published
// page). Text inside a backtick code span is already literal, so it
// passes through untouched: a source that writes `flake:<ref>` keeps
// exactly that text. Existing entities are not special-cased --
// "&lt;" contains no angle bracket and passes through as written.
// A backtick run with no matching closing run is literal text
// (CommonMark), so it does not switch the rest of the string to code.
func EscapeAngles(s string) string {
	if !strings.ContainsAny(s, "<>") {
		return s
	}
	var b strings.Builder
	b.Grow(len(s))
	span := 0 // backtick run length of the code span we are inside; 0 = prose
	for i := 0; i < len(s); {
		c := s[i]
		if c == '`' {
			j := i
			for j < len(s) && s[j] == '`' {
				j++
			}
			run := j - i
			switch {
			case span == 0 && hasClosingRun(s[j:], run):
				span = run
			case span != 0 && run == span:
				span = 0
			}
			b.WriteString(s[i:j])
			i = j
			continue
		}
		if span == 0 {
			switch c {
			case '<':
				b.WriteString("&lt;")
				i++
				continue
			case '>':
				b.WriteString("&gt;")
				i++
				continue
			}
		}
		b.WriteByte(c)
		i++
	}
	return b.String()
}

// hasClosingRun reports whether s contains a backtick run of exactly
// n backticks, which is what closes a code span opened by n backticks.
func hasClosingRun(s string, n int) bool {
	for i := 0; i < len(s); {
		if s[i] != '`' {
			i++
			continue
		}
		j := i
		for j < len(s) && s[j] == '`' {
			j++
		}
		if j-i == n {
			return true
		}
		i = j
	}
	return false
}

// Code renders s as a fenced code block. The fence is one backtick
// longer than the longest backtick run inside s (and at least three),
// so code that itself contains fences cannot break out of the block.
func Code(s string) string {
	longest, run := 0, 0
	for _, r := range s {
		if r == '`' {
			run++
			if run > longest {
				longest = run
			}
		} else {
			run = 0
		}
	}
	n := longest + 1
	if n < 3 {
		n = 3
	}
	fence := strings.Repeat("`", n)
	body := s
	if body != "" && !strings.HasSuffix(body, "\n") {
		body += "\n"
	}
	return fence + "\n" + body + fence + "\n"
}

// Region wraps body in the ANCHOR comment pair mdBook includes
// understand: <!-- ANCHOR: name --> … <!-- ANCHOR_END: name -->.
// Trailing newlines on body are normalised so a Region ends with
// exactly one newline after the closing marker.
func Region(name, body string) string {
	return "<!-- ANCHOR: " + name + " -->\n" +
		strings.TrimRight(body, "\n") + "\n" +
		"<!-- ANCHOR_END: " + name + " -->\n"
}
