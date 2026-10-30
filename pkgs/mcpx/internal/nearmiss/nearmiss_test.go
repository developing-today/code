package nearmiss

import "testing"

func TestNormalizeFoldsSpellingsTogether(t *testing.T) {
	for _, group := range [][]string{
		{"listPets", "list_pets", "list-pets", "ListPets", "list pets"},
		{"entry", "Entry", "ENTRY"},
	} {
		want := Normalize(group[0])
		for _, s := range group[1:] {
			if got := Normalize(s); got != want {
				t.Errorf("Normalize(%q) = %q, want %q (same name as %q)", s, got, want, group[0])
			}
		}
	}
}

func TestDistanceCountsTranspositionAsOneEdit(t *testing.T) {
	cases := []struct {
		a, b string
		want int
	}{
		{"entry", "entry", 0},
		{"entry", "entyr", 1}, // the launcher's example: a swapped pair
		{"entry", "entr", 1},
		{"entry", "antry", 1}, {"entry", "antri", 2},
		{"listPets", "list_pets", 0}, // normalisation, not distance
		{"call", "catalog", 4},
		{"", "abc", 3},
	}
	for _, c := range cases {
		if got := Distance(c.a, c.b); got != c.want {
			t.Errorf("Distance(%q, %q) = %d, want %d", c.a, c.b, got, c.want)
		}
	}
}

func TestNearIsTheStrictTest(t *testing.T) {
	for _, c := range []struct {
		a, b string
		want bool
	}{
		{"entry", "entry", true},
		{"entry", "entyr", true},
		{"entry", "entr", true}, // distance one
		{"entry", "entries", false},
		{"cat", "catalog", true}, // prefix rule, both longer than two
		{"ab", "abcd", false},    // prefix rule needs length > 2
		{"entry", "zebra", false},
	} {
		if got := Near(c.a, c.b); got != c.want {
			t.Errorf("Near(%q, %q) = %v, want %v", c.a, c.b, got, c.want)
		}
	}
}

func TestNearestThresholdAndTieBreaking(t *testing.T) {
	// Diagnose's examples: a third of the name may differ.
	if got, ok := Nearest("listpet", []string{"catalog", "listPets"}); !ok || got != "listPets" {
		t.Errorf("Nearest(listpet) = %q, %v; want listPets, true", got, ok)
	}
	if _, ok := Nearest("zzzzzz", []string{"catalog", "listPets"}); ok {
		t.Error("Nearest(zzzzzz) matched; beyond the threshold is noise")
	}
	// Deterministic ties: equal distances pick the lexicographically
	// smaller candidate regardless of enumeration order.
	a, _ := Nearest("abc", []string{"abd", "abb"})
	b, _ := Nearest("abc", []string{"abb", "abd"})
	if a != "abb" || b != "abb" {
		t.Errorf("tie-breaking not deterministic: %q vs %q, want abb both ways", a, b)
	}
}

func TestThreshold(t *testing.T) {
	for n, want := range map[int]int{0: 1, 2: 1, 3: 2, 5: 2, 6: 3, 9: 4} {
		if got := Threshold(n); got != want {
			t.Errorf("Threshold(%d) = %d, want %d", n, got, want)
		}
	}
}
