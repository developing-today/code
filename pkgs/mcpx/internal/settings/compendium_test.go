package settings_test

import (
	"encoding/json"
	"io/fs"
	"os"
	"path/filepath"
	"sort"
	"strings"
	"testing"
)

// manifestPath is the compendium's mapping, next to the structure page that
// describes it. Written relative to this package, the way configurationPath is,
// because the package is where every test that reads a doc path already lives.
const manifestPath = "../../docs/compendium.manifest.json"

// compendiumManifest is compendium.manifest.json: the chapters of the book's
// level 3, and where every markdown file in the module belongs.
type compendiumManifest struct {
	Schema   string `json:"schema"`
	Chapters []struct {
		ID    string `json:"id"`
		Order int    `json:"order"`
		Title string `json:"title"`
	} `json:"chapters"`
	Entries []struct {
		Path    string  `json:"path"`
		Chapter *string `json:"chapter"`
		Role    string  `json:"role"`
		Note    string  `json:"note"`
	} `json:"entries"`
}

// TestEveryDocumentHasAChapter keeps the compendium honest.
//
// The compendium is the encyclopedia: every markdown file in pkgs/mcpx is
// either a page of it or explicitly not, with a reason. A list like that is
// wrong the first time someone adds a document and does not think about the
// book, and nothing notices -- the page simply is not there, and the reader
// concludes it does not exist. Nor does a stale list fail loudly: a chapter
// that points at a deleted file renders as a dead link, and a document that
// moved leaves a chapter pointing at nothing.
//
// So the list is checked against the tree on every test run. The failure has to
// name the file, because the fix is a decision about that file: give it a
// chapter, or say why it has none.
func TestEveryDocumentHasAChapter(t *testing.T) {
	b, err := os.ReadFile(manifestPath)
	if err != nil {
		t.Fatalf("the compendium manifest should exist: %v", err)
	}
	var m compendiumManifest
	if err := json.Unmarshal(b, &m); err != nil {
		t.Fatalf("%s is not valid JSON: %v", manifestPath, err)
	}
	if m.Schema != "mcpx-compendium/v1" {
		t.Fatalf("manifest schema is %q, want mcpx-compendium/v1", m.Schema)
	}

	chapters := map[string]bool{}
	lastOrder := 0
	for _, c := range m.Chapters {
		if c.ID == "" || c.Title == "" {
			t.Fatalf("a chapter has no id or no title: %+v", c)
		}
		if chapters[c.ID] {
			t.Fatalf("chapter %q is listed twice", c.ID)
		}
		if c.Order <= lastOrder {
			t.Fatalf("chapter %q has order %d, which does not follow %d: the "+
				"chapters are one sequence, and the order is what the navigation "+
				"is generated from", c.ID, c.Order, lastOrder)
		}
		lastOrder = c.Order
		chapters[c.ID] = true
	}

	// role says what a file is to the book. chapter-source is a page of it;
	// chapter-index is the README of a directory chapter; symlink is a pointer
	// to a page rather than a second source; included is linked from the book
	// without being a chapter; not-compendium is left out on purpose.
	roles := map[string]bool{
		"chapter-source": true,
		"chapter-index":  true,
		"symlink":        true,
		"included":       true,
		"not-compendium": true,
	}

	mapped := map[string]bool{}
	for _, e := range m.Entries {
		if mapped[e.Path] {
			t.Fatalf("%s is in the manifest twice", e.Path)
		}
		mapped[e.Path] = true
		if !roles[e.Role] {
			t.Errorf("%s has role %q, which is not one of chapter-source, "+
				"chapter-index, symlink, included, not-compendium", e.Path, e.Role)
		}
		if strings.TrimSpace(e.Note) == "" {
			t.Errorf("%s has no note: every entry says why it is where it is, "+
				"because that reason is the only thing that cannot be generated", e.Path)
		}
		switch e.Role {
		case "chapter-source", "chapter-index", "symlink":
			if e.Chapter == nil {
				t.Errorf("%s is a %s but has no chapter", e.Path, e.Role)
			} else if !chapters[*e.Chapter] {
				t.Errorf("%s is in chapter %q, which is not one of the chapters "+
					"the structure page lists", e.Path, *e.Chapter)
			}
		default:
			if e.Chapter != nil {
				t.Errorf("%s is %s but still claims chapter %q", e.Path, e.Role, *e.Chapter)
			}
		}
		if _, err := os.Lstat(filepath.Join("../..", e.Path)); err != nil {
			t.Errorf("the manifest places %s, but it is not in the tree: %v", e.Path, err)
		}
	}

	// Now the other direction: everything on disk has to be placed. This is the
	// half that catches a new document, which is the half that actually happens.
	root := "../.."
	var docs []string
	err = filepath.WalkDir(root, func(p string, d fs.DirEntry, err error) error {
		if err != nil {
			return err
		}
		if d.IsDir() {
			switch d.Name() {
			case ".git", "node_modules", "testdata":
				return filepath.SkipDir
			}
			return nil
		}
		if ext := strings.ToLower(filepath.Ext(p)); ext != ".md" && ext != ".markdown" {
			return nil
		}
		rel, err := filepath.Rel(root, p)
		if err != nil {
			return err
		}
		docs = append(docs, filepath.ToSlash(rel))
		return nil
	})
	if err != nil {
		t.Fatalf("walking the module: %v", err)
	}
	sort.Strings(docs)

	var unplaced []string
	for _, p := range docs {
		if !mapped[p] {
			unplaced = append(unplaced, p)
		}
	}
	if len(unplaced) > 0 {
		t.Errorf("%d markdown file(s) are not in the compendium manifest, so the "+
			"book does not know about them:\n  %s\nGive each one a chapter in %s, "+
			"or a role and a reason for not having one.",
			len(unplaced), strings.Join(unplaced, "\n  "), manifestPath)
	}
}
