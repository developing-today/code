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

// TestCheckRejectsNonNumberHeaders pins the Codex findings on
// PR 2955: the numeric fallback must validate the ENTIRE header as a
// JSON number literal. A quoted string, or a number with trailing
// junk, is not the same numeric value.
func TestCheckRejectsNonNumberHeaders(t *testing.T) {
	params := []Param{{Name: "n", Path: []string{"n"}}}
	for _, hdr := range []string{`"42"`, "42]", "42}", "42 ", " 42", "4/2", "0x2A", "42e", "042", "+42", ".42", ""} {
		h := func(string) (string, bool) { return hdr, true }
		if err := Check(params, json.RawMessage(`{"n": 42}`), h); err == nil {
			t.Errorf("Check(header %q) = nil, want a mismatch error", hdr)
		}
	}
}

// TestCheckExponentCancellingMantissa pins the second Codex finding:
// the exponent bound applies AFTER the mantissa is normalised, so a
// literal whose zeros cancel its exponent is judged on its value.
func TestCheckExponentCancellingMantissa(t *testing.T) {
	params := []Param{{Name: "n", Path: []string{"n"}}}
	for _, hdr := range []string{
		"1" + strings.Repeat("0", 513) + "e-513",
		"0." + strings.Repeat("0", 512) + "1e513",
	} {
		h := func(string) (string, bool) { return hdr, true }
		if err := Check(params, json.RawMessage(`{"n": 1}`), h); err != nil {
			t.Errorf("Check(header %.20q...) = %v, want nil", hdr, err)
		}
	}
}

// TestCheckHugeIntegerLiteralBounded pins the third Codex finding: a
// huge integer literal with no exponent is classified out-of-range
// without materialising it in arbitrary precision.
func TestCheckHugeIntegerLiteralBounded(t *testing.T) {
	params := []Param{{Name: "n", Path: []string{"n"}}}
	h := func(string) (string, bool) { return "1", true }
	for _, body := range []string{
		`{"n": ` + strings.Repeat("9", 5000) + `}`,
		`{"n": ` + strings.Repeat("9", 5000) + `.0}`,
	} {
		err := Check(params, json.RawMessage(body), h)
		if err == nil || !strings.Contains(err.Error(), "outside the JavaScript safe-integer range") {
			t.Errorf("Check(huge body) = %v, want safe-range error", err)
		}
	}
}
