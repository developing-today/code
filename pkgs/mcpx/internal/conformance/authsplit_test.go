package conformance_test

import (
	"path/filepath"
	"testing"

	"github.com/developing-today/code/pkgs/mcpx/internal/conformance"
)

// TestAuthSplit pins the size of the accepts-vs-refuses split (#1972). The
// split is complete by construction (LoadAuthSplit refuses a bound
// authorization side with no verdict), so this guards the numbers the
// issues quote: a new authorization row must be judged, not absorbed.
func TestAuthSplit(t *testing.T) {
	reqs, err := conformance.Load(filepath.Join(root(t), conformance.CataloguePath))
	if err != nil {
		t.Fatal(err)
	}
	sp, err := conformance.LoadAuthSplit(filepath.Join(root(t), conformance.AuthSplitPath), reqs)
	if err != nil {
		t.Fatal(err)
	}
	c := conformance.CountAuthSplit(reqs, sp)
	// Only catalogue-bound sides are judged. #2619 honoured each row's
	// not-implemented-by-choice note (the #1950 decision: client OAuth
	// deferred, /mcp resource server planned at #1975), which marked most
	// authorization sides n/a -- including every validation side. The split
	// therefore covers just the 8 bound sides, all permissive, and #1972's
	// validation target lives with the #1975 work, not here (#2695).
	if c.Rows != 7 || c.Sides != 8 {
		t.Errorf("split covers %d rows / %d row-sides, want 7 rows / 8 row-sides", c.Rows, c.Sides)
	}
	if c.Permissive != 8 || c.Validation != 0 {
		t.Errorf("split is %d permissive / %d validation, want 8 / 0", c.Permissive, c.Validation)
	}
	for check, want := range map[string]int{"invalid": 0, "expired": 0, "wrong-resource": 0, "scope": 0} {
		if c.PerCheck[check] != want {
			t.Errorf("%d validation row-sides need %q, want %d", c.PerCheck[check], check, want)
		}
	}
}
