package e2e_test

import (
	"os"
	"regexp"
	"strings"
	"testing"
)

// The label is a fact about what a test does, not a naming convention
// (#1990, approach C): a test in a real_*_test.go file that never calls
// testsupport.RealServer is a fake test hiding in the realsoftware tag,
// and a RealServer call outside the tag would drag real servers into
// the per-PR path. This guard runs in the default build, where the
// tagged files themselves do not compile, and keeps both honest.
func TestRealSoftwareGuard(t *testing.T) {
	entries, err := os.ReadDir(".")
	if err != nil {
		t.Fatal(err)
	}
	realFile := regexp.MustCompile(`^real_.*_test\.go$`)
	tagged := 0
	for _, entry := range entries {
		name := entry.Name()
		if !strings.HasSuffix(name, "_test.go") || name == "realsoftware_guard_test.go" {
			continue
		}
		body, err := os.ReadFile(name)
		if err != nil {
			t.Fatal(err)
		}
		text := string(body)
		if realFile.MatchString(name) {
			tagged++
			if !strings.Contains(text, "//go:build realsoftware") {
				t.Errorf("%s: real_*_test.go without the realsoftware build tag runs in the per-PR path", name)
			}
			if !strings.Contains(text, "testsupport.RealServer(") {
				t.Errorf("%s: real_*_test.go never calls testsupport.RealServer — a fake test hiding in the tag", name)
			}
		} else if strings.Contains(text, "testsupport.RealServer(") {
			t.Errorf("%s: calls testsupport.RealServer outside a real_*_test.go file; real servers belong behind the tag", name)
		}
	}
	if tagged == 0 {
		t.Fatal("no real_*_test.go files found; the real-software subset (#1990) is gone")
	}
}
