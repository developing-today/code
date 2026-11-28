package logstore

import (
	"strings"
	"testing"
)

// TestBareReadPragmaRegistryGuard is the class guard for the bare-PRAGMA
// fix (#6638): it iterates the bareReadPragmas registry itself, so a name
// added to or dropped from the allowlist is classified consistently in
// every form — bare, schema-qualified, and upper-case — and no
// registered read can be turned into an assignment.
func TestBareReadPragmaRegistryGuard(t *testing.T) {
	for name := range bareReadPragmas {
		for _, q := range []string{
			"PRAGMA " + name,
			"PRAGMA main." + name,
			"PRAGMA " + strings.ToUpper(name) + ";",
		} {
			if err := CheckReadOnly(q); err != nil {
				t.Errorf("registered bare read rejected: %q: %v", q, err)
			}
		}
		// The assignment form of a registered read stays refused.
		if err := CheckReadOnly("PRAGMA " + name + "=1"); err == nil {
			t.Errorf("assignment form of a bare read was allowed: %q", name)
		}
		// The call form stays refused too, except for the function-form
		// reads whose argument names an object to inspect.
		if !functionFormReadPragmas[name] {
			if err := CheckReadOnly("PRAGMA " + name + "(1)"); err == nil {
				t.Errorf("call form of a bare-only read was allowed: %q", name)
			}
		}
	}
}

// TestBareActionPragmasStayRefused is the independent oracle for the
// action side of the class: these names come from SQLite's PRAGMA
// inventory, not from the registry under test, and every one must stay
// out of the allowlist and refused in every form.
func TestBareActionPragmasStayRefused(t *testing.T) {
	actions := []string{
		"optimize", "wal_checkpoint", "incremental_vacuum", "shrink_memory",
	}
	for _, name := range actions {
		if bareReadPragmas[name] {
			t.Errorf("action PRAGMA %q is in the bare read allowlist", name)
		}
		for _, q := range []string{
			"PRAGMA " + name,
			"PRAGMA main." + name,
			"PRAGMA " + strings.ToUpper(name),
			"PRAGMA " + name + "(1)",
		} {
			if err := CheckReadOnly(q); err == nil {
				t.Errorf("action PRAGMA was allowed: %q", q)
			}
		}
	}
}
