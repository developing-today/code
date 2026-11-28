package config

import "testing"

// #1912, Approach C: scope keys key on directory identity. dirIdentity is
// stubbed here the way gitDevice is stubbed elsewhere, so both volume kinds
// are covered on any OS: a case-insensitive volume is exactly the case
// where two spellings stat to one identity.

func stubIdentity(t *testing.T, ids map[string][2]uint64) {
	t.Helper()
	orig := dirIdentity
	dirIdentity = func(path string) (uint64, uint64, bool) {
		id, ok := ids[path]
		if !ok {
			return 0, 0, false
		}
		return id[0], id[1], true
	}
	identitySpellings.Lock()
	saved := identitySpellings.byID
	identitySpellings.byID = map[[2]uint64]string{}
	identitySpellings.Unlock()
	t.Cleanup(func() {
		dirIdentity = orig
		identitySpellings.Lock()
		identitySpellings.byID = saved
		identitySpellings.Unlock()
	})
}

func TestKeyPathTwoSpellingsOfOneDirectoryKeyTogether(t *testing.T) {
	stubIdentity(t, map[string][2]uint64{
		"/Users/x/Repo": {7, 42},
		"/users/x/repo": {7, 42},
	})
	a := keyPath("/Users/x/Repo")
	b := keyPath("/users/x/repo")
	if a != b {
		t.Fatalf("two spellings of one directory must key together: %q vs %q", a, b)
	}
	if a != "/Users/x/Repo" {
		t.Fatalf("the key should stay a readable path (the first spelling seen): %q", a)
	}
}

func TestKeyPathCaseDifferingDistinctDirectoriesStaySeparate(t *testing.T) {
	stubIdentity(t, map[string][2]uint64{
		"/x/Repo": {7, 42},
		"/x/repo": {7, 43},
	})
	if keyPath("/x/Repo") == keyPath("/x/repo") {
		t.Fatal("genuinely different directories that differ only by case must not share a key")
	}
}

func TestKeyPathFallsBackToThePathWhenIdentityIsUnavailable(t *testing.T) {
	stubIdentity(t, map[string][2]uint64{})
	if got, want := keyPath("/gone/Repo"), "/gone/Repo"; got != want {
		t.Fatalf("an unstattable path keys by itself: %q vs %q", got, want)
	}
}

func TestKeyPathRecycledIdentityDoesNotResurrectADeadSpelling(t *testing.T) {
	// An inode is recycled when its directory is deleted. The side table
	// must notice the remembered spelling no longer stats to the identity
	// and key the live directory by its own spelling.
	ids := map[string][2]uint64{"/old/Repo": {7, 42}, "/new/repo": {7, 42}}
	orig := dirIdentity
	dirIdentity = func(path string) (uint64, uint64, bool) {
		id, ok := ids[path]
		if !ok {
			return 0, 0, false
		}
		return id[0], id[1], true
	}
	identitySpellings.Lock()
	saved := identitySpellings.byID
	identitySpellings.byID = map[[2]uint64]string{}
	identitySpellings.Unlock()
	t.Cleanup(func() {
		dirIdentity = orig
		identitySpellings.Lock()
		identitySpellings.byID = saved
		identitySpellings.Unlock()
	})
	if got := keyPath("/old/Repo"); got != "/old/Repo" {
		t.Fatalf("first spelling keys by itself: %q", got)
	}
	delete(ids, "/old/Repo") // the directory is gone; 7:42 now names /new/repo
	if got := keyPath("/new/repo"); got != "/new/repo" {
		t.Fatalf("a recycled identity must not resurrect the dead spelling: %q", got)
	}
}
