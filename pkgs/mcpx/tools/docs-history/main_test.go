package main

import (
	"os"
	"path/filepath"
	"strings"
	"testing"
)

const sampleDoc = "# Doc\n" +
	"\n" +
	"## Pools\n" +
	"\n" +
	"```\n" +
	"created:      2026-09-25T18:30:00-05:00\n" +
	"last-updated: 2026-09-27T05:05:00-05:00\n" +
	"id:           pools\n" +
	"since:        unreleased\n" +
	"status:       core\n" +
	"description:  pools of processes\n" +
	"changed:\n" +
	"  - 0.2.0 (2026-11-02): idle exit is now a setting\n" +
	"```\n" +
	"\n" +
	"Pool prose.\n" +
	"\n" +
	"### Lifetime\n" +
	"\n" +
	"<!-- id: pools-lifetime since: 0.2.0 -->\n" +
	"\n" +
	"Lifetime prose.\n" +
	"\n" +
	"```\n" +
	"## not a heading\n" +
	"created: not a key block, it is a sample\n" +
	"```\n" +
	"\n" +
	"## Other\n" +
	"\n" +
	"No metadata here.\n"

func TestParserSkipsFencesAndReadsKeyBlocks(t *testing.T) {
	doc := ParseDocument("story.md", sampleDoc)
	if len(doc.Sections) != 4 {
		t.Fatalf("want 4 sections (fence heading skipped), got %d", len(doc.Sections))
	}
	pools := doc.ByID["pools"]
	if pools == nil {
		t.Fatal("pools section not found by explicit id")
	}
	if pools.Since != "unreleased" || pools.Status != "core" {
		t.Fatalf("pools metadata wrong: since=%q status=%q", pools.Since, pools.Status)
	}
	if len(pools.Changed) != 1 || pools.Changed[0].Version != "0.2.0" {
		t.Fatalf("pools changed wrong: %+v", pools.Changed)
	}
	life := doc.ByID["pools-lifetime"]
	if life == nil || life.Since != "0.2.0" {
		t.Fatalf("compact marker not parsed: %+v", life)
	}
	if life.Parent != pools {
		t.Fatal("lifetime should be a child of pools")
	}
}

func TestKeyBlockAfterCompactMarker(t *testing.T) {
	src := "## Parent\n\n### Sub\n\n<!-- id: sub since: pre-release -->\n\n```\ncreated: 2026-09-27T05:05:00-05:00\nid: sub\nsince: pre-release\nstatus: standard\ndescription: d\n```\n\nProse.\n"
	doc := ParseDocument("x.md", src)
	sub := doc.ByID["sub"]
	if sub == nil || sub.KeyBlock == nil {
		t.Fatal("key block after a compact marker was not recognised")
	}
	if sub.Status != "standard" {
		t.Fatalf("status = %q", sub.Status)
	}
}

func TestMigratedBlockLeadingWithID(t *testing.T) {
	src := "## Entry\n\n```\nid:           entry\nsince:        pre-release\ncreated:      2026-09-25T00:00:00-05:00\nstatus:       core\ndescription:  d\n```\n\nProse.\n"
	doc := ParseDocument("x.md", src)
	sec := doc.ByID["entry"]
	if sec == nil || sec.KeyBlock == nil {
		t.Fatal("migrated key block leading with id: was not recognised")
	}
	if sec.Since != "pre-release" {
		t.Fatalf("since = %q", sec.Since)
	}
}

func TestSlugify(t *testing.T) {
	cases := map[string]string{
		"Session-isolated process pools":      "session-isolated-process-pools",
		"[feat(a11y): Teletypewriter](../x.md)": "feata11y-teletypewriter",
		"~~Modes and scopes~~ — shipped":      "modes-and-scopes-shipped",
	}
	for in, want := range cases {
		if got := Slugify(in); got != want {
			t.Errorf("Slugify(%q) = %q, want %q", in, got, want)
		}
	}
}

func TestRenderBubbleUp(t *testing.T) {
	doc := ParseDocument("story.md", sampleDoc)
	out := Render(doc, false)
	if !strings.Contains(out, "Since: unreleased") {
		t.Error("render missing since marker")
	}
	if !strings.Contains(out, "<details><summary>History -- 1 changes, latest 0.2.0 (2026-11-02): idle exit is now a setting</summary>\n\n- 0.2.0 (2026-11-02): idle exit is now a setting") {
		t.Error("render missing own-entry history with blank line after summary")
	}
	if !strings.Contains(out, "</details>") {
		t.Error("render missing closing details")
	}
}

func TestRenderBubbleUpFromDescendant(t *testing.T) {
	src := "# Doc\n\n## Parent\n\n```\ncreated: 2026-09-25T00:00:00-05:00\nid: parent\nsince: pre-release\nstatus: core\ndescription: p\n```\n\nProse.\n\n### Child\n\n<!-- id: child since: pre-release -->\n<!-- changed: 0.3.0 (2026-12-01): child changed -->\n\nChild prose.\n"
	out := Render(ParseDocument("x.md", src), false)
	if !strings.Contains(out, "History -- 1 changes, latest 0.3.0") {
		t.Error("parent disclosure should count the descendant change")
	}
	if !strings.Contains(out, "**0.3.0**") || !strings.Contains(out, "[Child](#child) -- child changed") {
		t.Error("parent disclosure should carry one grouped line per descendant change")
	}
}

func TestRenderAgentTwinHasNoDisclosures(t *testing.T) {
	out := Render(ParseDocument("story.md", sampleDoc), true)
	if strings.Contains(out, "<details>") {
		t.Error("agent form must not contain disclosures")
	}
	if !strings.Contains(out, "Since: unreleased · Status: core") {
		t.Error("agent form missing since/status line")
	}
	if !strings.Contains(out, "History: [#pools](#pools)") {
		t.Error("agent form missing history link")
	}
}

func writeFile(t *testing.T, dir, name, content string) string {
	t.Helper()
	p := filepath.Join(dir, name)
	if err := os.MkdirAll(filepath.Dir(p), 0o755); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(p, []byte(content), 0o644); err != nil {
		t.Fatal(err)
	}
	return p
}

func TestStampFoldsFragmentsAndIsIdempotent(t *testing.T) {
	dir := t.TempDir()
	docPath := writeFile(t, dir, "docs/story.md", sampleDoc)
	fragPath := writeFile(t, dir, "docs/changes/pools-idle.md",
		"sections: pools\nkind: Changed\nsummary: idle exit is now a setting\n")
	if err := Stamp([]string{docPath}, "0.2.0", "2026-11-02", ""); err != nil {
		t.Fatal(err)
	}
	data, _ := os.ReadFile(docPath)
	got := string(data)
	if !strings.Contains(got, "since:        0.2.0") {
		t.Error("stamp did not replace since: unreleased")
	}
	if strings.Count(got, "idle exit is now a setting") != 1 {
		t.Errorf("fragment summary should appear exactly once in changed list:\n%s", got)
	}
	if _, err := os.Stat(fragPath); !os.IsNotExist(err) {
		t.Error("fragment file should be deleted after stamping")
	}
	// Second run: no fragments left, no since: unreleased — a no-op.
	if err := Stamp([]string{docPath}, "0.2.0", "2026-11-02", ""); err != nil {
		t.Fatal(err)
	}
	data2, _ := os.ReadFile(docPath)
	if string(data2) != got {
		t.Error("second stamp run was not idempotent")
	}
}

func TestStampFailsOnUnknownSection(t *testing.T) {
	dir := t.TempDir()
	docPath := writeFile(t, dir, "docs/story.md", sampleDoc)
	writeFile(t, dir, "docs/changes/bad.md", "sections: no-such-id\nkind: Added\nsummary: x\n")
	if err := Stamp([]string{docPath}, "0.2.0", "2026-11-02", ""); err == nil {
		t.Fatal("stamp should fail on a fragment naming an unknown section ID")
	}
	data, _ := os.ReadFile(docPath)
	if strings.Contains(string(data), "since:        0.2.0") {
		t.Error("a failed stamp must not write any file")
	}
}

func TestCheckDuplicateAndBadSince(t *testing.T) {
	dir := t.TempDir()
	src := "## A\n\n<!-- id: dup since: banana -->\n\nx\n\n## B\n\n<!-- id: dup since: 0.1.0 -->\n\ny\n"
	p := writeFile(t, dir, "docs/a.md", src)
	problems := Check([]string{p}, "", false, "")
	joined := strings.Join(problems, "\n")
	if !strings.Contains(joined, "duplicate section ID") {
		t.Errorf("missing duplicate problem: %v", problems)
	}
	if !strings.Contains(joined, "bad since value") {
		t.Errorf("missing bad-since problem: %v", problems)
	}
}

const sampleDiff = "diff --git a/docs/story.md b/docs/story.md\n" +
	"--- a/docs/story.md\n" +
	"+++ b/docs/story.md\n" +
	"@@ -20,3 +20,3 @@\n" +
	" Lifetime prose.\n" +
	"-old line\n" +
	"+new line\n" +
	" \n"

func TestCheckDiffNeedsFragment(t *testing.T) {
	dir := t.TempDir()
	p := writeFile(t, dir, "docs/story.md", sampleDoc)
	// The diff touches line 21 (1-based) — inside Lifetime's prose.
	problems := Check([]string{p}, sampleDiff, false, "")
	if !strings.Contains(strings.Join(problems, "\n"), "pools-lifetime") {
		t.Fatalf("expected a problem naming pools-lifetime, got %v", problems)
	}
	// With a fragment naming the section, the same diff passes.
	writeFile(t, dir, "docs/changes/lifetime.md", "sections: pools-lifetime\nkind: Fixed\nsummary: wording\n")
	if problems := Check([]string{p}, sampleDiff, false, ""); len(problems) != 0 {
		t.Fatalf("fragment should satisfy the check, got %v", problems)
	}
	// Trivial marker also satisfies it (fragment removed again).
	os.Remove(filepath.Join(dir, "docs/changes/lifetime.md"))
	if problems := Check([]string{p}, sampleDiff, true, ""); len(problems) != 0 {
		t.Fatalf("trivial check should pass, got %v", problems)
	}
}

func TestOwnerSectionChildHeaderIsNotParentProse(t *testing.T) {
	src := "## Parent\n\n```\ncreated: 2026-09-25T00:00:00-05:00\nid: parent\nsince: pre-release\nstatus: core\ndescription: p\n```\n\nParent prose.\n\n### Child\n\n<!-- id: child since: pre-release -->\n\nChild prose.\n"
	doc := ParseDocument("x.md", src)
	// Line 14 (0-based) is the child's compact marker: header zone of
	// the child, not prose of the parent.
	if got := ownerSection(doc, 14); got != nil {
		t.Fatalf("child marker line owned by %v, want nil", got.ID)
	}
	// Line 16 is child prose.
	if got := ownerSection(doc, 16); got == nil || got.ID != "child" {
		t.Fatalf("child prose line owned by %v, want child", got)
	}
	// Line 10 is parent prose.
	if got := ownerSection(doc, 10); got == nil || got.ID != "parent" {
		t.Fatalf("parent prose line owned by %v, want parent", got)
	}
}

func TestStampTwoFragmentsOneChangedList(t *testing.T) {
	dir := t.TempDir()
	src := "## Entry\n\n```\ncreated: 2026-09-25T00:00:00-05:00\nid: entry\nsince: unreleased\nstatus: core\ndescription: d\n```\n\nProse.\n"
	docPath := writeFile(t, dir, "docs/a.md", src)
	writeFile(t, dir, "docs/changes/one.md", "sections: entry\nkind: Added\nsummary: first\n")
	writeFile(t, dir, "docs/changes/two.md", "sections: entry\nkind: Fixed\nsummary: second\n")
	if err := Stamp([]string{docPath}, "0.2.0", "2026-11-02", ""); err != nil {
		t.Fatal(err)
	}
	data, _ := os.ReadFile(docPath)
	got := string(data)
	if strings.Count(got, "changed:") != 1 {
		t.Fatalf("want exactly one changed: key, got:\n%s", got)
	}
	if !strings.Contains(got, "  - 0.2.0 (2026-11-02): first\n  - 0.2.0 (2026-11-02): second") {
		t.Fatalf("both entries should share the one list:\n%s", got)
	}
}

func TestCheckDiffDeletionOnlyNeedsFragment(t *testing.T) {
	dir := t.TempDir()
	p := writeFile(t, dir, "docs/story.md", sampleDoc)
	diff := "diff --git a/docs/story.md b/docs/story.md\n--- a/docs/story.md\n+++ b/docs/story.md\n@@ -22,3 +22,2 @@\n Lifetime prose.\n-old line\n \n"
	problems := Check([]string{p}, diff, false, "")
	if !strings.Contains(strings.Join(problems, "\n"), "pools-lifetime") {
		t.Fatalf("deletion-only change should name pools-lifetime, got %v", problems)
	}
}

func TestRenderAnchorForRenamedSection(t *testing.T) {
	src := "## New name\n\n```\ncreated: 2026-09-25T00:00:00-05:00\nid: old-name\nsince: pre-release\nstatus: core\ndescription: d\nchanged:\n  - 0.2.0 (2026-11-02): x\n```\n\nProse.\n\n## Other\n\n```\ncreated: 2026-09-25T00:00:00-05:00\nid: other\nsince: pre-release\nstatus: core\ndescription: d\nchanged:\n  - 0.2.0 (2026-11-02): y\n```\n\nMore.\n"
	out := Render(ParseDocument("x.md", src), false)
	if !strings.Contains(out, "<a id=\"old-name\"></a>\n## New name") {
		t.Error("renamed section should get an explicit anchor for its frozen ID")
	}
}

func TestCheckFragmentUnknownID(t *testing.T) {
	dir := t.TempDir()
	p := writeFile(t, dir, "docs/story.md", sampleDoc)
	writeFile(t, dir, "docs/changes/bad.md", "sections: ghost\nkind: Added\nsummary: x\n")
	problems := Check([]string{p}, "", false, "")
	if !strings.Contains(strings.Join(problems, "\n"), "unknown section ID") {
		t.Fatalf("expected unknown-ID problem, got %v", problems)
	}
}

func TestMdBookRoundTrip(t *testing.T) {
	book := `[{"name":"ctx"}, {"sections":[{"Chapter":{"name":"C","content":"# T\n\n## S\n\n<!-- id: s since: 0.1.0 -->\n\nprose\n","sub_items":[],"number":[1],"path":"c.md"}}]}]`
	out, err := renderMdBook([]byte(book), false)
	if err != nil {
		t.Fatal(err)
	}
	if !strings.Contains(string(out), "Since: 0.1.0") || !strings.Contains(string(out), "\"number\":[1]") {
		t.Errorf("mdBook render lost content or fields: %s", out)
	}
}
