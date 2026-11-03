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
		{"catalog", "cat", true}, // the same pair, swapped
		{"ab", "abcd", false},    // prefix rule needs length > 2
		{"abcd", "ab", false},    // #3075: the swapped pair must agree — the guard keys on both lengths, not the first argument alone
		{"en", "entry", false},   // a two-letter prefix of a known name, both orders
		{"entry", "en", false},
		{"ab", "abcde", false},
		{"abcde", "ab", false},
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

func TestDistanceCountsUnicodeCharacters(t *testing.T) {
	// #3099: one multibyte character is one character. Every pair
	// below differs by exactly one Unicode character, so each costs
	// one edit; measured in UTF-8 bytes each cost two.
	cases := []struct {
		a, b string
		want int
	}{
		{"café", "cafe", 1},     // substitution: é for e
		{"entrée", "entree", 1}, // substitution in a longer name
		{"привет", "привед", 1}, // Cyrillic substitution: т for д
		{"café", "cafés", 1},    // insertion
		{"cafés", "café", 1},    // deletion
		{"café", "caéf", 1},     // adjacent transposition across a multibyte rune
		{"", "éé", 2},           // two runes, four bytes
		{"café", "café", 0},
	}
	for _, c := range cases {
		if got := Distance(c.a, c.b); got != c.want {
			t.Errorf("Distance(%q, %q) = %d, want %d", c.a, c.b, got, c.want)
		}
	}
}

func TestNearRecognisesSingleMultibyteEdits(t *testing.T) {
	for _, c := range []struct {
		a, b string
		want bool
	}{
		{"café", "cafe", true},
		{"entrée", "entree", true},
		{"привет", "привед", true},
		{"café", "caéf", true},
		{"ééé", "éééé", true}, // prefix rule: both longer than two characters
		{"éé", "éééé", false}, // prefix guard counts characters: a two-rune prefix is too short, and the distance is two
	} {
		if got := Near(c.a, c.b); got != c.want {
			t.Errorf("Near(%q, %q) = %v, want %v", c.a, c.b, got, c.want)
		}
		if got := Near(c.b, c.a); got != c.want {
			t.Errorf("Near(%q, %q) = %v, want %v (symmetric)", c.b, c.a, got, c.want)
		}
	}
}

func TestNearestThresholdCountsCharacters(t *testing.T) {
	if got, ok := Nearest("привет", []string{"привед", "catalog"}); !ok || got != "привед" {
		t.Errorf("Nearest(привет) = %q, %v; want привед, true", got, ok)
	}
	// Three runes may drift by Threshold(3) = 2, however many bytes
	// they occupy: a distance of three is noise. Counted in bytes
	// (6) the threshold would inflate to 3 and wrongly match.
	if _, ok := Nearest("ééé", []string{"aaa"}); ok {
		t.Error("Nearest(ééé) matched aaa; the threshold must count characters, not bytes")
	}
}
