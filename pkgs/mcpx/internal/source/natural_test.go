package source

import "testing"

func TestNaturalLessComparesArbitrarilyLongDigitRuns(t *testing.T) {
	tests := []struct {
		name string
		a    string
		b    string
	}{
		{
			name: "both overflowing runs with suffixes",
			a:    "f99999999999999999999999z",
			b:    "f100000000000000000000000a",
		},
		{
			name: "both overflowing runs without suffixes",
			a:    "f99999999999999999999999",
			b:    "f100000000000000000000000",
		},
		{
			name: "one run overflows",
			a:    "f8999999999999999999",
			b:    "f9999999999999999999",
		},
		{
			name: "leading zeroes do not change numeric order",
			a:    "f0000000000000000000002",
			b:    "f10",
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			if !naturalLess(tt.a, tt.b) {
				t.Errorf("naturalLess(%q, %q) = false, want true", tt.a, tt.b)
			}
			if naturalLess(tt.b, tt.a) {
				t.Errorf("naturalLess(%q, %q) = true, want false", tt.b, tt.a)
			}
		})
	}
}

func TestNaturalLessContinuesAfterNumericallyEqualDigitRuns(t *testing.T) {
	if !naturalLess("f0002a", "f2b") {
		t.Fatal("equal numeric runs should continue comparing their suffixes")
	}
	if naturalLess("f2b", "f0002a") {
		t.Fatal("suffix ordering should be symmetric")
	}
	if naturalLess("f0002", "f2") || naturalLess("f2", "f0002") {
		t.Fatal("equal numeric runs without differing suffixes should compare equal")
	}
	if !naturalLess("f00099999999999999999999999a", "f99999999999999999999999b") {
		t.Fatal("equal arbitrarily long numeric runs should continue comparing their suffixes")
	}
}

func TestNaturalLessIsStrictWeakOrder(t *testing.T) {
	values := []string{
		"", "A", "a", "a0", "a00", "a01", "a1", "a1z", "a1zz", "a2", "a02", "a2b", "a10",
		"a8999999999999999999", "a9999999999999999999", "a99999999999999999999999z",
		"a00099999999999999999999999z",
		"a100000000000000000000000a", "a100000000000000000000000",
	}
	equivalent := func(a, b string) bool {
		return !naturalLess(a, b) && !naturalLess(b, a)
	}

	for _, a := range values {
		if naturalLess(a, a) {
			t.Errorf("naturalLess(%q, %q) = true, want false", a, a)
		}
		for _, b := range values {
			if naturalLess(a, b) && naturalLess(b, a) {
				t.Errorf("naturalLess orders %q before %q and %q before %q", a, b, b, a)
			}
			for _, c := range values {
				if naturalLess(a, b) && naturalLess(b, c) && !naturalLess(a, c) {
					t.Errorf("naturalLess is not transitive for %q, %q, %q", a, b, c)
				}
				if equivalent(a, b) && equivalent(b, c) && !equivalent(a, c) {
					t.Errorf("naturalLess equivalence is not transitive for %q, %q, %q", a, b, c)
				}
			}
		}
	}
}
