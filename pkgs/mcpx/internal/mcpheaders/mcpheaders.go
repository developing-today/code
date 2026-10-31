// Package mcpheaders is 2026-07-28's request-metadata header encoding:
// the Base64 sentinel, x-mcp-header annotations, and the Mcp-Param-*
// values they mirror. One implementation, used by mcpx as a client (to
// send the headers) and as a server (to check them), so the two sides
// cannot drift apart.
//
// https://modelcontextprotocol.io/specification/2026-07-28/basic/transports/streamable-http#custom-headers-from-tool-parameters
package mcpheaders

import (
	"encoding/base64"
	"encoding/json"
	"fmt"
	"math/big"
	"sort"
	"strconv"
	"strings"
)

const (
	SentinelPrefix = "=?base64?"
	SentinelSuffix = "?="
	ParamPrefix    = "Mcp-Param-"
)

// Encode renders a value for a header, using the Base64 sentinel
// where the plain form is not safe: anything outside visible ASCII, space and
// tab; leading or trailing whitespace, which HTTP strips; and any plain value
// that already looks like the sentinel, which would otherwise be decoded into
// something the client never sent.
func Encode(v string) string {
	if Safe(v) && !(strings.HasPrefix(v, SentinelPrefix) && strings.HasSuffix(v, SentinelSuffix)) {
		return v
	}
	return SentinelPrefix + base64.StdEncoding.EncodeToString([]byte(v)) + SentinelSuffix
}

func Safe(v string) bool {
	if v == "" {
		return true
	}
	if first, last := v[0], v[len(v)-1]; first == ' ' || first == '\t' || last == ' ' || last == '\t' {
		return false
	}
	for i := 0; i < len(v); i++ {
		b := v[i]
		if b == '\t' || (b >= 0x20 && b <= 0x7e) {
			continue
		}
		return false
	}
	return true
}

// Param is one x-mcp-header annotation: the header name part and the
// chain of properties keys that leads to the annotated value.
type Param struct {
	Name string
	Path []string
}

// ToolParams validates a tool's x-mcp-header annotations and returns them.
// An error means the tool definition is invalid and must be excluded.
//
// The constraints are the schema-extension list: a non-empty RFC 9110 token,
// unique case-insensitively, on a primitive (integer, string, boolean -- not
// number) reached only through "properties". An annotation anywhere else --
// under items, a composition keyword, a $ref -- makes the whole tool invalid,
// so the walk looks everywhere and only accepts the reachable ones.
func ToolParams(schema json.RawMessage) ([]Param, error) {
	if len(schema) == 0 {
		return nil, nil
	}
	var root any
	if err := json.Unmarshal(schema, &root); err != nil {
		return nil, nil // not our concern; the schema is forwarded as it is
	}
	var out []Param
	seen := map[string]bool{}
	var walk func(node any, path []string, reachable bool) error
	walk = func(node any, path []string, reachable bool) error {
		switch n := node.(type) {
		case []any:
			for _, v := range n {
				if err := walk(v, path, false); err != nil {
					return err
				}
			}
		case map[string]any:
			if raw, ok := n["x-mcp-header"]; ok {
				if !reachable || len(path) == 0 {
					return fmt.Errorf("x-mcp-header %v is not on a property reachable through properties alone", raw)
				}
				name, ok := raw.(string)
				if !ok {
					return fmt.Errorf("x-mcp-header on %s is not a string", strings.Join(path, "."))
				}
				if err := validHeaderName(name); err != nil {
					return fmt.Errorf("x-mcp-header %q on %s: %w", name, strings.Join(path, "."), err)
				}
				if t, _ := n["type"].(string); t != "string" && t != "integer" && t != "boolean" {
					return fmt.Errorf("x-mcp-header %q on %s: type %v is not integer, string or boolean",
						name, strings.Join(path, "."), n["type"])
				}
				if seen[strings.ToLower(name)] {
					return fmt.Errorf("x-mcp-header %q is not unique (case-insensitively)", name)
				}
				seen[strings.ToLower(name)] = true
				out = append(out, Param{Name: name, Path: append([]string(nil), path...)})
			}
			keys := make([]string, 0, len(n))
			for k := range n {
				keys = append(keys, k)
			}
			sort.Strings(keys)
			for _, k := range keys {
				v := n[k]
				if k == "properties" {
					if props, ok := v.(map[string]any); ok {
						pk := make([]string, 0, len(props))
						for p := range props {
							pk = append(pk, p)
						}
						sort.Strings(pk)
						for _, p := range pk {
							if err := walk(props[p], append(append([]string(nil), path...), p), reachable); err != nil {
								return err
							}
						}
						continue
					}
				}
				if k == "x-mcp-header" {
					continue
				}
				if err := walk(v, path, false); err != nil {
					return err
				}
			}
		}
		return nil
	}
	if err := walk(root, nil, true); err != nil {
		return nil, err
	}
	return out, nil
}

// validHeaderName is RFC 9110 token syntax, 1*tchar.
func validHeaderName(s string) error {
	if s == "" {
		return fmt.Errorf("empty")
	}
	for i := 0; i < len(s); i++ {
		c := s[i]
		switch {
		case c >= 'a' && c <= 'z', c >= 'A' && c <= 'Z', c >= '0' && c <= '9':
		case strings.IndexByte("!#$%&'*+-.^_`|~", c) >= 0:
		default:
			return fmt.Errorf("byte %q is not an HTTP token character", c)
		}
	}
	return nil
}

// Values extracts the Mcp-Param-* headers for one call. A value that is
// absent or null omits its header; a value of the wrong type is an error,
// because sending a header the server will reject is worse than not calling.
func Values(params []Param, args any) (map[string]string, error) {
	if len(params) == 0 {
		return nil, nil
	}
	root, err := decodeArgs(args)
	if err != nil {
		return nil, err
	}
	out := map[string]string{}
	for _, p := range params {
		s, present, err := plain(root, p)
		if err != nil {
			return nil, err
		}
		if present {
			out[ParamPrefix+p.Name] = Encode(s)
		}
	}
	return out, nil
}

// Check is the server half of Values: every recognised Mcp-Param-* header
// must carry header-safe characters, decode, and equal the body value at its
// annotated path; a body value with no header, or a header with no body
// value, is a mismatch too. header returns a header's value and whether it
// was sent at all, since an empty string is a value. A non-nil error is a
// HeaderMismatch.
func Check(params []Param, args json.RawMessage, header func(string) (string, bool)) error {
	if len(params) == 0 {
		return nil
	}
	var root any
	if len(args) > 0 {
		dec := json.NewDecoder(strings.NewReader(string(args)))
		dec.UseNumber()
		if err := dec.Decode(&root); err != nil {
			return err
		}
	}
	for _, p := range params {
		name := ParamPrefix + p.Name
		raw, sent := header(name)
		body, present, err := plain(root, p)
		if err != nil {
			return err
		}
		if sent && !Safe(raw) {
			return fmt.Errorf("%s contains characters a header may not carry", name)
		}
		switch {
		case !sent && !present:
			continue
		case !sent:
			return fmt.Errorf("%s header is required: the body has a value at %s", name, strings.Join(p.Path, "."))
		case !present:
			return fmt.Errorf("%s header was sent but the body has no value at %s", name, strings.Join(p.Path, "."))
		}
		got, err := Decode(raw)
		if err != nil {
			return fmt.Errorf("%s header is not valid Base64: %v", name, err)
		}
		if got == body {
			continue
		}
		// Integers compare numerically: 42.0 in a header is 42.
		// The comparison is exact: a float64 round-trip would call
		// 9007199254740990.5 equal to 9007199254740990.
		if _, isNum := lookupNumber(root, p.Path); isNum {
			hf, hok := exactNumber(got)
			bf, bok := exactNumber(body)
			if hok && bok && hf.Cmp(bf) == 0 {
				continue
			}
		}
		return fmt.Errorf("%s header value '%s' does not match body value '%s'", name, got, body)
	}
	return nil
}

// Decode undoes the Base64 sentinel. A value without both markers is
// literal. The encoding is strict -- padding required, alphabet only --
// because the specification's test-case table rejects both.
func Decode(v string) (string, error) {
	if !strings.HasPrefix(v, SentinelPrefix) || !strings.HasSuffix(v, SentinelSuffix) ||
		len(v) < len(SentinelPrefix)+len(SentinelSuffix) {
		return v, nil
	}
	b, err := base64.StdEncoding.Strict().DecodeString(v[len(SentinelPrefix) : len(v)-len(SentinelSuffix)])
	if err != nil {
		return "", err
	}
	return string(b), nil
}

func decodeArgs(args any) (any, error) {
	b, err := json.Marshal(args)
	if err != nil {
		return nil, err
	}
	var root any
	dec := json.NewDecoder(strings.NewReader(string(b)))
	dec.UseNumber()
	if err := dec.Decode(&root); err != nil {
		return nil, err
	}
	return root, nil
}

func lookupNumber(root any, path []string) (json.Number, bool) {
	v, _ := lookup(root, path)
	n, ok := v.(json.Number)
	return n, ok
}

// plain is the unencoded header form of the value at p's path, and whether
// there is one: absent and null are both "no header".
func plain(root any, p Param) (string, bool, error) {
	v, ok := lookup(root, p.Path)
	if !ok || v == nil {
		return "", false, nil
	}
	switch x := v.(type) {
	case string:
		return x, true, nil
	case bool:
		return strconv.FormatBool(x), true, nil
	case json.Number:
		// A whole-valued number written in float form (42.0, 42.00,
		// 4.2e1, -0.0) is an integer for comparison purposes: JSON
		// Schema's integer type accepts it and the spec says integers
		// compare numerically. Rejecting it here, before Check's
		// numeric fallback can run, made the forgiveness one-sided —
		// it only ever applied to the header's form (#1665). The
		// verdict is computed on the exact decimal value: a float64
		// round-trip rounds 9007199254740990.5 to an integer and
		// underflows 1e-400 to 0.
		i, verdict := exactInteger(x.String())
		switch verdict {
		case integerNotWhole:
			return "", false, fmt.Errorf("%s: %s is not an integer", strings.Join(p.Path, "."), x)
		case integerOutOfRange:
			return "", false, fmt.Errorf("%s: %s is outside the JavaScript safe-integer range", strings.Join(p.Path, "."), x)
		}
		return strconv.FormatInt(i, 10), true, nil
	}
	return "", false, fmt.Errorf("%s: a %T cannot be mirrored into a header", strings.Join(p.Path, "."), v)
}

type integerVerdict int

const (
	integerOK integerVerdict = iota
	integerNotWhole
	integerOutOfRange
)

// exactInteger judges a JSON number literal on its exact decimal
// value: whether it is a whole number, and if so whether it lies
// inside the JavaScript safe-integer range.
func exactInteger(s string) (int64, integerVerdict) {
	if i, err := strconv.ParseInt(s, 10, 64); err == nil {
		if i > maxSafeInt || i < -maxSafeInt {
			return 0, integerOutOfRange
		}
		return i, integerOK
	}
	d, ok := parseDecimal(s)
	if d.digits == "" {
		// Zero in any form (-0.0, 0e5), or no literal at all; the
		// ParseInt path above already took plain "0", so this is the
		// float/exponent spellings of zero.
		if ok {
			return 0, integerOK
		}
		return 0, integerNotWhole
	}
	if !ok {
		// Grammar was valid but the literal is beyond the processing
		// bounds (thousands of significant digits or an enormous
		// effective scale). After normalisation scale < 0 means the
		// last significant digit sits below the units place, so the
		// value is a nonzero fraction; scale >= 0 makes it an integer
		// with thousands of digits, far outside the safe range.
		if d.scale < 0 {
			return 0, integerNotWhole
		}
		return 0, integerOutOfRange
	}
	if d.scale < 0 {
		return 0, integerNotWhole
	}
	// Whole: the integer has len(digits)+scale digits. More than 16
	// cannot fit the safe range (which tops out at 9007199254740991).
	if len(d.digits)+d.scale > 16 {
		return 0, integerOutOfRange
	}
	v, err := strconv.ParseInt(d.digits+strings.Repeat("0", d.scale), 10, 64)
	if err != nil {
		return 0, integerOutOfRange
	}
	if d.neg {
		v = -v
	}
	if v > maxSafeInt || v < -maxSafeInt {
		return 0, integerOutOfRange
	}
	return v, integerOK
}

// exactNumber parses a strict JSON number literal into its exact
// rational value, for Check's numeric comparison. A header string is
// sender-controlled, so the whole string must be the literal --
// quoted strings, trailing brackets and other junk are not numbers
// (Codex review on PR 2955).
func exactNumber(s string) (*big.Rat, bool) {
	d, ok := parseDecimal(s)
	if !ok {
		return nil, false
	}
	if d.digits == "" {
		return new(big.Rat), true
	}
	num, _ := new(big.Int).SetString(d.digits, 10)
	r := new(big.Rat).SetInt(num)
	if d.scale != 0 {
		n := d.scale
		if n < 0 {
			n = -n
		}
		pow := new(big.Rat).SetInt(new(big.Int).Exp(big.NewInt(10), big.NewInt(int64(n)), nil))
		if d.scale > 0 {
			r.Mul(r, pow)
		} else {
			r.Quo(r, pow)
		}
	}
	if d.neg {
		r.Neg(r)
	}
	return r, true
}

// maxDecimalDigits bounds the significant digits (and the effective
// scale) one literal may carry into exact arithmetic. Without it, a
// sender-supplied literal with a million digits -- or an exponent
// that only cancels after materialising one -- would make the exact
// comparison allocate in proportion to the attacker's text, or
// worse, to their exponent (Codex review on PR 2955).
const maxDecimalDigits = 4096

// decimal is a JSON number literal in normalised form:
//
//	value = (neg ? -1 : 1) * digits * 10^scale
//
// digits carries no leading or trailing zeros, so scale < 0 exactly
// when the value is a nonzero fraction. digits == "" is exactly zero,
// at any exponent. Normalising BEFORE any bound is applied is what
// lets 1000...0e-513 (514-digit mantissa, exponent -513) be judged
// as the small integer it is, instead of being refused for its raw
// exponent.
type decimal struct {
	neg    bool
	digits string
	scale  int
}

// parseDecimal validates s as one strict JSON number literal -- the
// whole string, JSON grammar: -?(0|[1-9][0-9]*)(\.[0-9]+)?([eE][+-]?[0-9]+)?
// -- and normalises it. ok is false when the grammar fails, or when
// the normalised literal exceeds maxDecimalDigits; in the bounds
// case the returned decimal still describes the literal (with the
// exponent clamped) so callers can classify it.
func parseDecimal(s string) (decimal, bool) {
	i := 0
	neg := false
	if i < len(s) && s[i] == '-' {
		neg = true
		i++
	}
	if i >= len(s) {
		return decimal{}, false
	}
	intStart := i
	if s[i] == '0' {
		i++
	} else if s[i] >= '1' && s[i] <= '9' {
		for i < len(s) && s[i] >= '0' && s[i] <= '9' {
			i++
		}
	} else {
		return decimal{}, false
	}
	intPart := s[intStart:i]
	fracPart := ""
	if i < len(s) && s[i] == '.' {
		i++
		fs := i
		for i < len(s) && s[i] >= '0' && s[i] <= '9' {
			i++
		}
		if i == fs {
			return decimal{}, false
		}
		fracPart = s[fs:i]
	}
	exp := 0
	if i < len(s) && (s[i] == 'e' || s[i] == 'E') {
		i++
		sign := 1
		if i < len(s) && (s[i] == '+' || s[i] == '-') {
			if s[i] == '-' {
				sign = -1
			}
			i++
		}
		es := i
		for i < len(s) && s[i] >= '0' && s[i] <= '9' {
			i++
		}
		if i == es {
			return decimal{}, false
		}
		// The exponent text itself is attacker-controlled length;
		// anything past six significant digits is clamped to a
		// sentinel far beyond the processing bounds.
		et := strings.TrimLeft(s[es:i], "0")
		if len(et) > 6 {
			exp = sign * (maxDecimalDigits * 1000)
		} else if et != "" {
			e, _ := strconv.Atoi(et)
			exp = sign * e
		}
	}
	if i != len(s) {
		return decimal{}, false
	}
	digits := intPart + fracPart
	scale := exp - len(fracPart)
	digits = strings.TrimLeft(digits, "0")
	trimmed := strings.TrimRight(digits, "0")
	scale += len(digits) - len(trimmed)
	digits = trimmed
	if digits == "" {
		return decimal{}, true
	}
	d := decimal{neg: neg, digits: digits, scale: scale}
	if len(digits) > maxDecimalDigits || scale > maxDecimalDigits || scale < -maxDecimalDigits {
		return d, false
	}
	return d, true
}

// maxSafeInt is 2^53-1, JavaScript's Number.MAX_SAFE_INTEGER.
const maxSafeInt = int64(1)<<53 - 1

func lookup(root any, path []string) (any, bool) {
	cur := root
	for _, k := range path {
		m, ok := cur.(map[string]any)
		if !ok {
			return nil, false
		}
		if cur, ok = m[k]; !ok {
			return nil, false
		}
	}
	return cur, true
}
