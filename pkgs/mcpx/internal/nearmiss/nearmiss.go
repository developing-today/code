// Package nearmiss owns the one definition of whether one name is
// "close" to another.
//
// mcpx grew two: diagnostics used plain Levenshtein (a transposition
// cost two edits, no prefix rule) and launcher placeholders used a
// distance-of-one test with a separate transposition check and a prefix
// rule. The same typo was therefore close in one place and not in the
// other, and search was about to add a third definition. Here the
// definition lives once (#1904): normalise, Damerau-Levenshtein, a
// length-relative threshold, deterministic tie-breaking.
package nearmiss

import (
	"strings"
	"unicode"
)

// Normalize folds a name to the form distances are measured on: case
// folded, with snake_case, kebab-case and camelCase boundaries removed,
// so the three spellings of one name are the same name. Digits and
// letters survive; every other rune is a boundary.
func Normalize(s string) string {
	var b strings.Builder
	prevLower := false
	for _, r := range s {
		switch {
		case unicode.IsLetter(r) || unicode.IsDigit(r):
			if unicode.IsUpper(r) && prevLower {
				// A camel boundary: the split is recorded by dropping
				// nothing -- the letters join -- because the boundary
				// carries no more meaning than a separator does.
			}
			b.WriteRune(unicode.ToLower(r))
			prevLower = unicode.IsLower(r) || unicode.IsDigit(r)
		default:
			prevLower = false
		}
	}
	return b.String()
}

// Distance is the Damerau-Levenshtein distance between two names after
// normalisation: an adjacent transposition costs one edit, which is
// what makes "entyr" one step from "entry" rather than two.
func Distance(a, b string) int {
	return damerau(Normalize(a), Normalize(b))
}

// damerau is optimal string alignment Damerau-Levenshtein over bytes of
// already-normalised (lowercase ASCII-folded) strings.
func damerau(a, b string) int {
	la, lb := len(a), len(b)
	d := make([][]int, la+1)
	for i := range d {
		d[i] = make([]int, lb+1)
		d[i][0] = i
	}
	for j := range d[0] {
		d[0][j] = j
	}
	for i := 1; i <= la; i++ {
		for j := 1; j <= lb; j++ {
			cost := 1
			if a[i-1] == b[j-1] {
				cost = 0
			}
			d[i][j] = min(d[i-1][j]+1, d[i][j-1]+1, d[i-1][j-1]+cost)
			if i > 1 && j > 1 && a[i-1] == b[j-2] && a[i-2] == b[j-1] {
				d[i][j] = min(d[i][j], d[i-2][j-2]+1)
			}
		}
	}
	return d[la][lb]
}

// Threshold is how far a name of this (normalised) length may drift and
// still count as a near miss: a third of the name may differ, and at
// least one edit always counts. Beyond it a suggestion is noise.
func Threshold(normalisedLen int) int {
	if t := 1 + normalisedLen/3; t > 1 {
		return t
	}
	return 1
}

// Near reports the strict near-miss: the names are equal after
// normalisation, one is a prefix of the other (for names longer than
// two characters), or their distance is at most one. This is the
// launcher's deliberately narrow test, unchanged in reach.
func Near(a, b string) bool {
	na, nb := Normalize(a), Normalize(b)
	if na == nb {
		return true
	}
	if len(na) > 2 && (strings.HasPrefix(nb, na) || strings.HasPrefix(na, nb)) {
		return true
	}
	return damerau(na, nb) <= 1
}

// Nearest returns the closest candidate to name, when one is within
// Threshold of it. Ties break deterministically: the smaller distance
// wins, then the lexicographically smaller candidate, so the answer
// never depends on the order candidates were enumerated in.
func Nearest(name string, candidates []string) (string, bool) {
	nn := Normalize(name)
	best, bestD := "", 1<<30
	for _, c := range candidates {
		d := damerau(nn, Normalize(c))
		if d < bestD || (d == bestD && c < best) {
			best, bestD = c, d
		}
	}
	if best == "" || bestD > Threshold(len(nn)) {
		return "", false
	}
	return best, true
}
