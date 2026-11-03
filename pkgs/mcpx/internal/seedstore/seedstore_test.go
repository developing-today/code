package seedstore_test

import (
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/developing-today/code/pkgs/mcpx/internal/seedstore"
)

// storeRoot is the seed store this guard keeps honest, relative to
// this package: pkgs/mcpx/testdata/seeds/v1.
const storeRoot = "../../testdata/seeds/v1"

// TestEveryStoredSeedIsValid is the #1953 guard test: every record in
// the store parses, names its own file, sits in the month directory
// of its date, and its date matches the timestamp embedded in its
// UUIDv7 id. It runs under go test ./... in the mcpx CI job, so a
// corrupt collector write fails the PR that carries it.
func TestEveryStoredSeedIsValid(t *testing.T) {
	records := 0
	err := filepath.WalkDir(storeRoot, func(path string, entry os.DirEntry, err error) error {
		if err != nil {
			return err
		}
		if entry.IsDir() {
			if entry.Name() == "testdata" || strings.HasPrefix(entry.Name(), ".") {
				return filepath.SkipDir
			}
			return nil
		}
		if !strings.HasSuffix(entry.Name(), ".json") {
			return nil
		}
		contents, err := os.ReadFile(path)
		if err != nil {
			return err
		}
		// One line of compact JSON: no embedded newline, and no
		// pretty-printing (the store's diffs stay one line per seed).
		if body := strings.TrimSuffix(string(contents), "\n"); strings.Contains(body, "\n") {
			t.Errorf("%s: record is not a single line", path)
		}
		rec, err := seedstore.Parse(contents)
		if err != nil {
			t.Errorf("%s: %v", path, err)
			return nil
		}
		if err := seedstore.Validate(rec, entry.Name()); err != nil {
			t.Errorf("%s: %v", path, err)
			return nil
		}
		// The file lives in v1/<yyyy>/<mm>/ of its own date.
		rel, err := filepath.Rel(storeRoot, path)
		if err != nil {
			return err
		}
		want := filepath.Join(rec.Date[0:4], rec.Date[5:7], entry.Name())
		if rel != want {
			t.Errorf("%s: record is filed at %q, want %q for its date", path, rel, want)
		}
		records++
		return nil
	})
	if err != nil {
		t.Fatalf("walking the seed store: %v", err)
	}
	if records == 0 {
		t.Fatal("the seed store holds no records; the guard would pass vacuously")
	}
}

// TestValidateRejectsCorruptRecords pins the failure side of the
// guard: a record whose filename or date disagrees with its seed id
// (the corruptions a bad collector write would produce) is rejected.
func TestValidateRejectsCorruptRecords(t *testing.T) {
	good := seedstore.Record{
		Seed:       "01a18b4a-e218-708e-9085-29de6dff0f9c",
		Commit:     "3a42555241a549acfd9c950a9734d75e1c23904c",
		Ref:        "refs/heads/main",
		Event:      "push",
		Run:        "https://github.com/developing-today/code/actions/runs/41008926724",
		Attempt:    1,
		Result:     "success",
		Derivation: "v1",
		Date:       "2026-10-30",
	}
	if err := seedstore.Validate(good, good.Seed+".json"); err != nil {
		t.Fatalf("the known-good record does not validate: %v", err)
	}
	if err := seedstore.Validate(good, "someone-else.json"); err == nil {
		t.Error("a filename that is not the seed id was accepted")
	}
	wrongDate := good
	wrongDate.Date = "2026-10-31"
	if err := seedstore.Validate(wrongDate, wrongDate.Seed+".json"); err == nil {
		t.Error("a date that disagrees with the seed's embedded timestamp was accepted")
	}
	notV7 := good
	notV7.Seed = "01a18b4a-e218-408e-9085-29de6dff0f9c"
	if err := seedstore.Validate(notV7, notV7.Seed+".json"); err == nil {
		t.Error("a version-4 id was accepted as a seed")
	}
}
