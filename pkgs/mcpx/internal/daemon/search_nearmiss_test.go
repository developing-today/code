package daemon

import "testing"

// #1923 slice 2: typo tolerance — a term that matches no exact tier falls
// back to the shared nearmiss matcher against tool/namespace names only,
// scored below every exact tier, with the tier named in --explain.
func nearmissRegistry() *Registry {
	return &Registry{testTools: []ToolInfo{
		{Namespace: "chrome", Server: "chrome", Tool: "take_screenshot", Function: "chrome.takeScreenshot", Description: "Capture the page"},
		{Namespace: "chrome", Server: "chrome", Tool: "takeSnapshot", Function: "chrome.takeSnapshot", Description: "Accessibility tree"},
		{Namespace: "files", Server: "files", Tool: "read_file", Function: "files.readFile", Description: "Read a file"},
		{Namespace: "files", Server: "files", Tool: "list_dir", Function: "files.listDir", Description: "List a directory"},
	}}
}

func TestSearchNearmissMisspellingFindsScreenshot(t *testing.T) {
	// "scrrenshot" (extra r) and "screeshot" (missing n) share no substring
	// or stemmed word with take_screenshot, so only the typo tier can find it.
	for _, q := range []string{"scrrenshot", "screeshot"} {
		hits := nearmissRegistry().Search(q, 10)
		if len(hits) == 0 || hits[0].Tool != "take_screenshot" {
			t.Fatalf("Search(%q) = %+v, want take_screenshot first", q, hits)
		}
		if hits[0].Score != 5 {
			t.Fatalf("Search(%q) score = %d, want the typo tier's 5", q, hits[0].Score)
		}
	}
}

func TestSearchNearmissTranspositionUsesNearmissSemantics(t *testing.T) {
	// "screenshto" is "screenshot" with the last two letters transposed:
	// Damerau-Levenshtein distance 1 in the shared nearmiss matcher, where
	// plain Levenshtein would charge 2 for the swap plus argue about the
	// rest. The proposals.md example must find take_screenshot.
	hits := nearmissRegistry().Search("screenshto", 10)
	if len(hits) == 0 || hits[0].Tool != "take_screenshot" {
		t.Fatalf("hits = %+v, want take_screenshot for the transposed query", hits)
	}
}

func TestSearchNearmissExplainNamesTypoTier(t *testing.T) {
	hits := nearmissRegistry().SearchSemanticOpts("scrrenshot", 10, false, "", true)
	if len(hits) == 0 || len(hits[0].Explain) != 1 || hits[0].Explain[0].Field != "name-typo" || hits[0].Explain[0].Points != 5 {
		t.Fatalf("explain = %+v, want one name-typo breakdown of 5", hits)
	}
}

func TestSearchNearmissNamespaceTypo(t *testing.T) {
	// A typo of the namespace name alone ("fiels" for "files") matches its
	// tools through the typo tier.
	hits := nearmissRegistry().Search("fiels", 10)
	found := map[string]bool{}
	for _, h := range hits {
		found[h.Tool] = true
	}
	if !found["read_file"] || !found["list_dir"] {
		t.Fatalf("hits = %+v, want the files tools for a namespace typo", hits)
	}
}

func TestSearchNearmissExactQueryUnchanged(t *testing.T) {
	// The typo tier sits below every exact tier and fires only when a term
	// matched nothing exact, so an exact query keeps its pre-slice-2 score
	// and breakdown.
	hits := nearmissRegistry().SearchSemanticOpts("screenshot", 10, false, "", true)
	if len(hits) == 0 || hits[0].Tool != "take_screenshot" || hits[0].Score != 40 {
		t.Fatalf("hits = %+v, want take_screenshot at the exact name-substring 40", hits)
	}
	if len(hits[0].Explain) != 1 || hits[0].Explain[0].Field != "name" {
		t.Fatalf("explain = %+v, want the exact name tier, not name-typo", hits[0].Explain)
	}
	// A term with no exact match and no near miss still finds nothing.
	if hits := nearmissRegistry().Search("zzzzqqqq", 10); len(hits) != 0 {
		t.Fatalf("hits = %+v, want none for a term far from every name", hits)
	}
}

func TestSearchNearmissMultiTermTypo(t *testing.T) {
	// requireAll runs first: both terms must match, a typo counting as a
	// match, so "take scrrenshot" still lands on take_screenshot alone.
	hits := nearmissRegistry().Search("take scrrenshot", 10)
	if len(hits) != 1 || hits[0].Tool != "take_screenshot" {
		t.Fatalf("hits = %+v, want take_screenshot alone", hits)
	}
	if hits[0].Score != 65 {
		t.Fatalf("score = %d, want 60 (name-prefix take) + 5 (typo screenshot)", hits[0].Score)
	}
}
