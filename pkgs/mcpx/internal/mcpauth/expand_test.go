package mcpauth_test

import (
	"encoding/base64"
	"strings"
	"testing"

	"github.com/developing-today/code/pkgs/mcpx/internal/mcpauth"
)

// The expansion contract (#1662, pinned by triage): only the braced
// ${VAR} shape is a reference. Every other $ in a credential is literal
// text that must reach the server byte-for-byte, and $${VAR} is the
// escape that writes a literal ${VAR}. These cases adapt the prepared
// probe for the defect: the BUG cases failed under the old os.Expand
// behaviour, the CTRL cases passed and must keep passing.

func basicWith(t *testing.T, user, pass string, get func(string) string) string {
	t.Helper()
	r, err := (&mcpauth.Auth{Type: "basic", Username: user, Password: pass}).ResolveWith(get)
	if err != nil {
		t.Fatal(err)
	}
	if len(r.Missing) > 0 {
		t.Fatalf("Missing = %v, want none for a literal credential", r.Missing)
	}
	return r.Headers["Authorization"]
}

func wantBasic(user, pass string) string {
	return "Basic " + base64.StdEncoding.EncodeToString([]byte(user+":"+pass))
}

func TestLiteralDollarCredentialsReachTheServerVerbatim(t *testing.T) {
	home := func(name string) string {
		if name == "HOME" {
			return "/home/daemon"
		}
		return ""
	}
	// A literal password ending in a digit-dollar segment is not the
	// special variable $2.
	if got := basicWith(t, "u", "hunter$2", home); got != wantBasic("u", "hunter$2") {
		t.Errorf("Authorization = %q, want the literal %q", got, wantBasic("u", "hunter$2"))
	}
	// '$$' in a literal password is not the special variable $.
	if got := basicWith(t, "u", "pa$$word", home); got != wantBasic("u", "pa$$word") {
		t.Errorf("Authorization = %q, want the literal %q", got, wantBasic("u", "pa$$word"))
	}
	// A $-segment naming a variable that IS set is not spliced into the
	// credential: only the braced shape is a reference.
	if got := basicWith(t, "u", "s3cret$HOME", home); got != wantBasic("u", "s3cret$HOME") {
		t.Errorf("Authorization = %q, want the literal %q (decoded payload must not contain the daemon's HOME)", got, wantBasic("u", "s3cret$HOME"))
	}
	// An env value that merely mentions a price keeps it.
	r2, err := (&mcpauth.Auth{Type: "env", Env: map[string]string{"NOTE": "cost is $5"}}).ResolveWith(
		func(string) string { return "" })
	if err != nil {
		t.Fatal(err)
	}
	if r2.Env["NOTE"] != "cost is $5" || len(r2.Missing) > 0 {
		t.Errorf("Env[NOTE] = %q, Missing = %v; want the literal value and no Missing", r2.Env["NOTE"], r2.Missing)
	}
}

func TestExpansionContractShapes(t *testing.T) {
	get := func(name string) string {
		if name == "MY_TOKEN" {
			return "tok"
		}
		return ""
	}
	bearer := func(tok string) *mcpauth.Resolved {
		r, err := (&mcpauth.Auth{Type: "bearer", Token: tok}).ResolveWith(get)
		if err != nil {
			t.Fatal(err)
		}
		return r
	}
	// CTRL: the braced reference form expands.
	if r := bearer("${MY_TOKEN}"); r.Headers["Authorization"] != "Bearer "+"tok" || len(r.Missing) != 0 {
		t.Errorf("reference: got %v, Missing %v", r.Headers, r.Missing)
	}
	// The escape renders the literal text ${VAR} and references nothing.
	if r := bearer("$${MY_TOKEN}"); r.Headers["Authorization"] != "Bearer "+"${MY_TOKEN}" || len(r.Missing) != 0 {
		t.Errorf("escape: got %v, Missing %v; want the literal reference text", r.Headers, r.Missing)
	}
	// Literal shapes survive byte-for-byte: bare name, trailing $, and
	// braced-looking text that is not a plain name.
	for _, lit := range []string{"$MY_TOKEN", "abc$", "${MY_TOKEN:-fallback}", "${}", "${UNTERMINATED"} {
		if r := bearer(lit); r.Headers["Authorization"] != "Bearer "+lit || len(r.Missing) != 0 {
			t.Errorf("%q: got %v, Missing %v; want it literal with no Missing", lit, r.Headers, r.Missing)
		}
	}
	// A reference and a literal $ in one value: only the reference moves.
	if r := bearer("${MY_TOKEN}-$5"); r.Headers["Authorization"] != "Bearer "+"tok"+"-$5" {
		t.Errorf("mixed: got %v", r.Headers)
	}
	// CTRL: a genuinely referenced, unset variable is still reported.
	r3 := bearer("${NOT_THERE}")
	if len(r3.Missing) != 1 || !strings.Contains(r3.Missing[0], "NOT_THERE") {
		t.Errorf("missing: Missing = %v, want [NOT_THERE]", r3.Missing)
	}
	// A header value with a literal $ digit is untouched too.
	h, err := (&mcpauth.Auth{Type: "header", Header: "X-Key", Value: "fee$2"}).ResolveWith(get)
	if err != nil {
		t.Fatal(err)
	}
	if h.Headers["X-Key"] != "fee$2" || len(h.Missing) != 0 {
		t.Errorf("header: got %v, Missing %v", h.Headers, h.Missing)
	}
}
