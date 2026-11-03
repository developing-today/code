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
	if c.Rows != 61 || c.Sides != 82 {
		t.Errorf("split covers %d rows / %d row-sides, want 61 rows / 82 row-sides", c.Rows, c.Sides)
	}
	if c.Permissive != 69 || c.Validation != 13 {
		t.Errorf("split is %d permissive / %d validation, want 69 / 13", c.Permissive, c.Validation)
	}
	for check, want := range map[string]int{"invalid": 6, "expired": 5, "wrong-resource": 3, "scope": 5} {
		if c.PerCheck[check] != want {
			t.Errorf("%d validation row-sides need %q, want %d", c.PerCheck[check], check, want)
		}
	}
}
