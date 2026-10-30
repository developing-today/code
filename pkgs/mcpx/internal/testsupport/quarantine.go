package testsupport

import (
	"fmt"
	"strings"
	"testing"
	"time"
)

// Quarantine skips the calling test under a tracked, expiring exemption
// (#1973, approach C): fix-now is the default, and quarantine is only
// allowed with a link to the issue that owns the flake and an expiry
// date. Until the expiry date the test skips; after it, the test FAILS,
// so a quarantine can never become a silent permanent skip.
//
// Usage, in the flaky test itself:
//
//	testsupport.Quarantine(t, "https://github.com/developing-today/code/issues/1234", "2026-12-31")
//
// The arguments must be string literals: TestQuarantineGuard scans the
// module for call sites and fails on any call whose issue link is
// missing or whose expiry has passed, so an expired quarantine fails
// the whole suite, not just the one test, at the next run.
func Quarantine(t testing.TB, issue, expiry string) {
	t.Helper()
	if err := ValidateQuarantine(issue, expiry, time.Now().UTC()); err != nil {
		t.Fatalf("testsupport.Quarantine: %v", err)
	}
	t.Skipf("quarantined until %s (%s)", expiry, issue)
}

// ValidateQuarantine is the check Quarantine and the guard test share:
// issue must be a link to a GitHub issue and expiry must be a
// YYYY-MM-DD date that has not passed as of now.
func ValidateQuarantine(issue, expiry string, now time.Time) error {
	if !strings.Contains(issue, "github.com/") || !strings.Contains(issue, "/issues/") {
		return fmt.Errorf("issue %q is not a GitHub issue link", issue)
	}
	exp, err := time.Parse("2006-01-02", expiry)
	if err != nil {
		return fmt.Errorf("expiry %q is not a YYYY-MM-DD date: %w", expiry, err)
	}
	today := time.Date(now.Year(), now.Month(), now.Day(), 0, 0, 0, 0, time.UTC)
	if today.After(exp) {
		return fmt.Errorf("quarantine expired on %s: fix the test or remove the call (issue %s)", expiry, issue)
	}
	return nil
}
