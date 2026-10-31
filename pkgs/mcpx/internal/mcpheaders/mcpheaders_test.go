package mcpheaders

import (
	"encoding/json"
	"strings"
	"testing"
)

// TestCheckIntegerComparesNumericallyBothWays pins #1665: an integer
// body value compares numerically against the header whichever side
// carries the float form. Before the fix, plain() rejected any
// float-form body literal as "not an integer" before the comparison
// ran, so only a float-form header was forgiven.
func TestCheckIntegerComparesNumericallyBothWays(t *testing.T) {
	params := []Param{{Name: "n", Path: []string{"n"}}}
	header := func(v string) func(string) (string, bool) {
		return func(string) (string, bool) { return v, true }
	}
	accepted := []struct{ args, hdr string }{
		{`{"n": 42}`, "42"},
		{`{"n": 42}`, "42.0"},
		{`{"n": 42.0}`, "42"},
		{`{"n": 42.0}`, "42.0"},
		{`{"n": 42.00}`, "42"},
		{`{"n": 4.2e1}`, "42"},
		{`{"n": -0.0}`, "0"},
	}
	for _, c := range accepted {
		if err := Check(params, json.RawMessage(c.args), header(c.hdr)); err != nil {
			t.Errorf("Check(%s, header %q) = %v, want nil", c.args, c.hdr, err)
		}
	}
	rejected := []struct {
		args, hdr, want string
	}{
		{`{"n": 42}`, "43", "does not match"},
		{`{"n": 42.0}`, "43", "does not match"},
		{`{"n": 42.5}`, "42", "is not an integer"},
		{`{"n": 9007199254740993}`, "9007199254740993", "outside the JavaScript safe-integer range"},
		{`{"n": 1e20}`, "100000000000000000000", "outside the JavaScript safe-integer range"},
	}
	for _, c := range rejected {
		err := Check(params, json.RawMessage(c.args), header(c.hdr))
		if err == nil {
			t.Errorf("Check(%s, header %q) = nil, want error containing %q", c.args, c.hdr, c.want)
		} else if !strings.Contains(err.Error(), c.want) {
			t.Errorf("Check(%s, header %q) = %v, want error containing %q", c.args, c.hdr, err, c.want)
		}
	}
}

// TestCheckIntegerPrecisionPermutations pins the Codex finding on
// PR 2955: integrality must be judged on the exact decimal value, not
// on a float64 round-trip -- 9007199254740990.5 rounds to an integer
// in float64, and 1e-400 underflows to 0.
func TestCheckIntegerPrecisionPermutations(t *testing.T) {
	params := []Param{{Name: "n", Path: []string{"n"}}}
	header := func(v string) func(string) (string, bool) {
		return func(string) (string, bool) { return v, true }
	}
	accepted := []struct{ args, hdr string }{
		{`{"n": 9007199254740990.0}`, "9007199254740990"},
		{`{"n": 9007199254740991.0}`, "9007199254740991"},
		{`{"n": 0.5e1}`, "5"},
		{`{"n": 42}`, "4.2e1"},
	}
	for _, c := range accepted {
		if err := Check(params, json.RawMessage(c.args), header(c.hdr)); err != nil {
			t.Errorf("Check(%s, header %q) = %v, want nil", c.args, c.hdr, err)
		}
	}
	rejected := []struct {
		args, hdr, want string
	}{
		{`{"n": 9007199254740990.5}`, "9007199254740990", "is not an integer"},
		{`{"n": -9007199254740990.5}`, "-9007199254740990", "is not an integer"},
		{`{"n": 1e-400}`, "0", "is not an integer"},
		{`{"n": 5e-1}`, "0", "is not an integer"},
		{`{"n": 9007199254740990}`, "9007199254740990.5", "does not match"},
		{`{"n": 9007199254740992.0}`, "9007199254740992", "outside the JavaScript safe-integer range"},
		{`{"n": 1e999999}`, "1", "outside the JavaScript safe-integer range"},
	}
	for _, c := range rejected {
		err := Check(params, json.RawMessage(c.args), header(c.hdr))
		if err == nil {
			t.Errorf("Check(%s, header %q) = nil, want error containing %q", c.args, c.hdr, c.want)
		} else if !strings.Contains(err.Error(), c.want) {
			t.Errorf("Check(%s, header %q) = %v, want error containing %q", c.args, c.hdr, err, c.want)
		}
	}
	// Values shares plain(): a value mcpx sends is a value mcpx
	// accepts, and a non-integer is refused on the send side too.
	if _, err := Values(params, map[string]any{"n": json.Number("9007199254740990.5")}); err == nil ||
		!strings.Contains(err.Error(), "is not an integer") {
		t.Errorf("Values(9007199254740990.5) = %v, want 'is not an integer'", err)
	}
	got, err := Values(params, map[string]any{"n": json.Number("42.0")})
	if err != nil {
		t.Fatalf("Values(42.0) = %v, want nil", err)
	}
	if got[ParamPrefix+"n"] != "42" {
		t.Errorf("Values(42.0) header = %q, want %q", got[ParamPrefix+"n"], "42")
	}
}
