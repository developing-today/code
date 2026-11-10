package daemon

import (
	"strings"
	"testing"
)

// #1923 slice 1: word-splitting/stemming, the tag tier + --tag filter, and
// --explain breakdowns, exercised through the registry search the daemon
// route serves.
func slice1Registry() *Registry {
	return &Registry{testTools: []ToolInfo{
		{Namespace: "chrome", Server: "chrome", Tool: "take_screenshot", Function: "chrome.takeScreenshot", Description: "Capture the page"},
		{Namespace: "chrome", Server: "chrome", Tool: "takeSnapshot", Function: "chrome.takeSnapshot", Description: "Accessibility tree"},
		{Namespace: "files", Server: "files", Tool: "read_file", Function: "files.readFile", Description: "Read a file", Tags: []string{"io", "disk"}},
		{Namespace: "files", Server: "files", Tool: "list_dir", Function: "files.listDir", Description: "List a directory", Tags: []string{"io"}},
	}}
}

func TestSearchSlice1StemmingFindsScreenshot(t *testing.T) {
	hits := slice1Registry().Search("screenshots", 10)
	if len(hits) == 0 || hits[0].Tool != "take_screenshot" {
		t.Fatalf("hits = %+v, want take_screenshot first", hits)
	}
	// The match is the stemmed name word, not a substring: "screenshots"
	// does not appear in "take_screenshot".
	hits = slice1Registry().SearchSemanticOpts("screenshots", 10, false, "", true)
	if len(hits) == 0 || len(hits[0].Explain) != 1 || hits[0].Explain[0].Field != "name-word" || hits[0].Explain[0].Points != 40 {
		t.Fatalf("explain = %+v, want one name-word breakdown of 40", hits)
	}
}

func TestSearchSlice1CamelCaseSplits(t *testing.T) {
	hits := slice1Registry().Search("snapshots", 10)
	found := false
	for _, h := range hits {
		if h.Tool == "takeSnapshot" {
			found = true
		}
	}
	if !found {
		t.Fatalf("hits = %+v, want takeSnapshot via camelCase split + stem", hits)
	}
}

func TestSearchSlice1TagTierAndFilter(t *testing.T) {
	// "disk" matches only read_file's tag: exactly the tag tier's 50.
	hits := slice1Registry().SearchSemanticOpts("disk", 10, false, "", true)
	if len(hits) != 1 || hits[0].Tool != "read_file" || hits[0].Score != 50 {
		t.Fatalf("hits = %+v, want read_file alone at tag-tier 50", hits)
	}
	if len(hits[0].Explain) != 1 || hits[0].Explain[0].Field != "tag" {
		t.Fatalf("explain = %+v, want the tag field named", hits[0].Explain)
	}
	// The --tag filter is exact: both io tools, and nothing else, whatever
	// the query terms would otherwise rank.
	hits = slice1Registry().SearchSemanticOpts("file", 10, false, "io", false)
	if len(hits) != 2 {
		t.Fatalf("tag-filtered hits = %+v, want the two io tools", hits)
	}
	for _, h := range hits {
		if !strings.HasPrefix(h.Function, "files.") {
			t.Fatalf("tag filter leaked %q", h.Function)
		}
	}
}

func TestSearchSlice1DefaultOutputHasNoExplain(t *testing.T) {
	hits := slice1Registry().Search("screenshot", 10)
	if len(hits) == 0 {
		t.Fatal("no hits")
	}
	for _, h := range hits {
		if h.Explain != nil {
			t.Fatalf("default search must not attach explanations, got %+v", h.Explain)
		}
	}
}

func TestSplitToolWordsAndStem(t *testing.T) {
	if got := splitToolWords("take_screenshot"); len(got) != 2 || got[0] != "take" || got[1] != "screenshot" {
		t.Fatalf("splitToolWords(take_screenshot) = %v", got)
	}
	if got := splitToolWords("takeScreenshot"); len(got) != 2 || got[1] != "screenshot" {
		t.Fatalf("splitToolWords(takeScreenshot) = %v", got)
	}
	for in, want := range map[string]string{
		"screenshots": "screenshot",
		"reading":     "read",
		"files":       "file",
		"class":       "class",
	} {
		if got := stemWord(in); got != want {
			t.Fatalf("stemWord(%q) = %q, want %q", in, got, want)
		}
	}
}
