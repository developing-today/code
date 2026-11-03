//go:build darwin

package config

import (
	"os"
	"path/filepath"
	"strings"
	"testing"
)

// Tests of macOS filesystem behaviour: /tmp is a symlink to /private/tmp, and
// the default volume is case-insensitive. Compiled only on macOS, so they are
// absent on Linux rather than counted there as skipped tests.

func TestGitParityTmpAndPrivateTmp(t *testing.T) {
	f := newGitFix(t)
	base, err := os.MkdirTemp("/tmp", "mcpx-gitparity-")
	if err != nil {
		t.Skip(err)
	}
	t.Cleanup(func() { os.RemoveAll(base) })
	t.Setenv("GIT_CEILING_DIRECTORIES", mustReal(t, "/tmp"))
	f.git(base, "init", "-q", "r")
	viaTmp := filepath.Join(base, "r")
	viaPrivate := filepath.Join("/private", viaTmp)
	f.same(viaTmp, certain)
	f.same(viaPrivate, certain)
	a, _ := ScopeRepo.Key(CallContext{Cwd: viaTmp})
	b, _ := ScopeRepo.Key(CallContext{Cwd: viaPrivate})
	if a != b || !strings.HasPrefix(a, "repo:/private/tmp/") {
		t.Errorf("both spellings must key alike, on the physical path: %q vs %q", a, b)
	}
}

func TestGitCaseOfAWrongCaseCwdIsKept(t *testing.T) {
	// Inverted by the #1912 fix; this test used to pin the bug. Native
	// discovery still keeps the caller's spelling -- getting the on-disk
	// spelling means F_GETPATH on macOS, which Go reaches only through
	// unsafe calls -- but that no longer splits the instance: scope keys
	// key on directory identity (dev, inode), so the two spellings of one
	// checkout resolve to one key on the cwd, repo and worktree scopes
	// alike, where before they got two instances.
	f := newGitFix(t)
	r := f.repo("CaseRepo")
	lower := filepath.Join(filepath.Dir(r), "caserepo")
	if _, err := os.Stat(lower); err != nil {
		t.Skip("case-sensitive filesystem")
	}
	top, _ := ref(lower, "--show-toplevel")
	w, err := discoverGit(lower)
	if err != nil {
		t.Fatal(err)
	}
	if top != r {
		t.Fatalf("expected git to answer with the on-disk case %s, got %s", r, top)
	}
	if w.worktree != lower || !strings.EqualFold(w.worktree, top) {
		t.Errorf("native should keep the given spelling and differ only in case: %s vs %s", w.worktree, top)
	}
	for _, sc := range []Scope{ScopeCwd, ScopeRepo, ScopeWorktree} {
		a, _ := sc.Key(CallContext{Cwd: r})
		b, _ := sc.Key(CallContext{Cwd: lower})
		if a != b {
			t.Errorf("scope %s: two spellings of one directory must key together: %q vs %q", sc, a, b)
		}
	}
}
