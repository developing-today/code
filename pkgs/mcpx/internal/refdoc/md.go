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
// Cells are escaped: a "|" becomes "\|" and a newline becomes "<br>",
// so a cell can never break the table's shape. Rows shorter than the
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
	s = strings.ReplaceAll(s, "|", `\|`)
	s = strings.ReplaceAll(s, "\r\n", "<br>")
	s = strings.ReplaceAll(s, "\n", "<br>")
	s = strings.ReplaceAll(s, "\r", "<br>")
	return s
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
