package testsupport

import (
	"errors"
	"os"
	"path/filepath"
	"regexp"
	"strings"
	"testing"
	"time"
)

func TestValidateQuarantine(t *testing.T) {
	now := time.Date(2026, 10, 30, 12, 0, 0, 0, time.UTC)
	cases := []struct {
		name    string
		issue   string
		expiry  string
		wantErr bool
	}{
		{"valid", "https://github.com/developing-today/code/issues/1973", "2026-12-31", false},
		{"expires today is still valid", "https://github.com/developing-today/code/issues/1973", "2026-10-30", false},
		{"expired", "https://github.com/developing-today/code/issues/1973", "2026-10-29", true},
		{"missing issue link", "", "2026-12-31", true},
		{"not an issue link", "https://github.com/developing-today/code/pull/12", "2026-12-31", true},
		{"bad date", "https://github.com/developing-today/code/issues/1973", "next friday", true},
	}
	for _, c := range cases {
		t.Run(c.name, func(t *testing.T) {
			err := ValidateQuarantine(c.issue, c.expiry, now)
			if (err != nil) != c.wantErr {
				t.Fatalf("ValidateQuarantine(%q, %q) err = %v, wantErr %v", c.issue, c.expiry, err, c.wantErr)
			}
		})
	}
}

type fakeTB struct {
	testing.TB
	skipped, failed bool
}

func (f *fakeTB) Helper() {}
func (f *fakeTB) Skipf(format string, args ...any) {
	f.skipped = true
}
// errFakeFatal is the panic a fake Fatalf raises to model what the real
// testing.T.Fatalf does: it does not return. Without this the flow under
// test continues past the fatal call and the double records a skip after
// a failure, which no real test can do.
var errFakeFatal = errors.New("fakeTB: Fatalf called")

func (f *fakeTB) Fatalf(format string, args ...any) {
	f.failed = true
	panic(errFakeFatal)
}

// callQuarantine runs Quarantine against a fakeTB and absorbs the
// fake-fatal panic, so the caller can assert on what was recorded.
func callQuarantine(f *fakeTB, issue, expiry string) {
	defer func() {
		if r := recover(); r != nil && r != errFakeFatal {
			panic(r)
		}
	}()
	Quarantine(f, issue, expiry)
}

func TestQuarantineSkipsAndFails(t *testing.T) {
	ok := &fakeTB{}
	callQuarantine(ok, "https://github.com/developing-today/code/issues/1973", "2999-01-01")
	if !ok.skipped || ok.failed {
		t.Fatalf("valid quarantine: skipped=%v failed=%v, want skip only", ok.skipped, ok.failed)
	}
	bad := &fakeTB{}
	callQuarantine(bad, "https://github.com/developing-today/code/issues/1973", "2000-01-01")
	if !bad.failed || bad.skipped {
		t.Fatalf("expired quarantine: failed=%v skipped=%v, want fail only", bad.failed, bad.skipped)
	}
}

// TestQuarantineGuard is the guard from #1973: every Quarantine call
// site in the module must use string literals, name a GitHub issue,
// and carry an expiry that has not passed. A call that evades the
// literal pattern fails here too, so the audit cannot be dodged by
// passing a variable.
func TestQuarantineGuard(t *testing.T) {
	call := regexp.MustCompile(`Quarantine\(\s*[A-Za-z_][A-Za-z0-9_.]*\s*,\s*"([^"]*)"\s*,\s*"([^"]*)"\s*,?\s*\)`)
	moduleRoot := filepath.Join("..", "..")
	now := time.Now().UTC()
	err := filepath.WalkDir(moduleRoot, func(path string, d os.DirEntry, err error) error {
		if err != nil {
			return err
		}
		if d.IsDir() {
			if d.Name() == "testdata" {
				return filepath.SkipDir
			}
			return nil
		}
		if !strings.HasSuffix(path, ".go") || strings.HasSuffix(path, "quarantine.go") || strings.HasSuffix(path, "quarantine_test.go") {
			return nil
		}
		data, err := os.ReadFile(path)
		if err != nil {
			return err
		}
		src := string(data)
		matches := call.FindAllStringSubmatch(src, -1)
		if n := strings.Count(src, "Quarantine("); n != len(matches) {
			t.Errorf("%s: %d Quarantine( mentions but %d literal call sites: every call must pass literal issue and expiry strings", path, n, len(matches))
		}
		for _, m := range matches {
			if err := ValidateQuarantine(m[1], m[2], now); err != nil {
				t.Errorf("%s: %v", path, err)
			}
		}
		return nil
	})
	if err != nil {
		t.Fatal(err)
	}
}
