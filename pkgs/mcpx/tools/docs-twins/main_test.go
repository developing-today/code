package main

import (
	"os"
	"path/filepath"
	"strings"
	"testing"
)

// fixture builds a small book: a home page with a hand-written
// override, a level-1 page, a level-2 child, a level-3 page, the
// compendium, and a draft chapter (no page).
func fixture(t *testing.T) (docsDir, agentsDir, outDir string) {
	t.Helper()
	root := t.TempDir()
	docsDir = filepath.Join(root, "docs")
	agentsDir = filepath.Join(root, "agents-docs")
	outDir = filepath.Join(root, "site")
	for _, dir := range []string{docsDir, agentsDir, outDir, filepath.Join(docsDir, "sub"), filepath.Join(outDir, "sub")} {
		if err := os.MkdirAll(dir, 0o755); err != nil {
			t.Fatal(err)
		}
	}
	write := func(dir, name, content string) {
		if err := os.WriteFile(filepath.Join(dir, name), []byte(content), 0o644); err != nil {
			t.Fatal(err)
		}
	}
	write(docsDir, "SUMMARY.md", `# Summary

- [Home](index.md)
- [Guide](guide.md)
- [The compendium](compendium.md)
- [Section](<>)
  - [Child](sub/child.md)
    - [Deep](sub/deep.md)
`)
	write(docsDir, "index.md", "# Home\n\nGenerated home content.\n")
	write(docsDir, "guide.md", "# Guide\n\nThe guide explains the thing in one line.\n\nMore prose.\n")
	write(docsDir, "compendium.md", "# Compendium\n\nEverything, at length.\n")
	write(filepath.Join(docsDir, "sub"), "child.md", "# Child\n\nA level two page.\n")
	write(filepath.Join(docsDir, "sub"), "deep.md", "# Deep\n\nA level three page.\n")
	write(agentsDir, "index.md", "# mcpx for agents\n\nInstall prompt lives here.\n")
	// Minimal built HTML pages, as mdbook would emit them.
	for _, page := range []string{"index.html", "guide.html", "compendium.html", "sub/child.html", "sub/deep.html"} {
		write(outDir, page, "<html><head><title>x</title></head><body>page</body></html>")
	}
	return docsDir, agentsDir, outDir
}

func read(t *testing.T, path string) string {
	t.Helper()
	data, err := os.ReadFile(path)
	if err != nil {
		t.Fatal(err)
	}
	return string(data)
}

func TestEmitWritesTwinsAndLLMS(t *testing.T) {
	docsDir, agentsDir, outDir := fixture(t)
	cfg := config{docsDir: docsDir, agentsDir: agentsDir, outDir: outDir}
	if err := runEmit(cfg); err != nil {
		t.Fatalf("emit: %v", err)
	}

	// The home twin is the hand-written override, not the source.
	if got := read(t, filepath.Join(outDir, "index.html.md")); !strings.Contains(got, "Install prompt lives here.") {
		t.Errorf("index.html.md should be the override, got:\n%s", got)
	}
	// A generated twin carries the page source.
	if got := read(t, filepath.Join(outDir, "guide.html.md")); !strings.Contains(got, "The guide explains the thing") {
		t.Errorf("guide.html.md missing source content:\n%s", got)
	}
	if _, err := os.Stat(filepath.Join(outDir, "sub", "deep.html.md")); err != nil {
		t.Errorf("level-3 twin missing: %v", err)
	}

	llms := read(t, filepath.Join(outDir, "llms.txt"))
	if len(llms) > llmsTxtBudget {
		t.Errorf("llms.txt is %d bytes, over budget %d", len(llms), llmsTxtBudget)
	}
	if !strings.HasPrefix(llms, "# mcpx\n") || !strings.Contains(llms, "## Docs") || !strings.Contains(llms, "## Optional") {
		t.Errorf("llms.txt structure wrong:\n%s", llms)
	}
	docsSection := llms[:strings.Index(llms, "## Optional")]
	if !strings.Contains(docsSection, "[Guide](guide.html.md): The guide explains the thing in one line.") {
		t.Errorf("level-1 entry with note missing from ## Docs:\n%s", docsSection)
	}
	if strings.Contains(docsSection, "deep.html.md") {
		t.Errorf("level-3 page leaked into ## Docs:\n%s", docsSection)
	}
	optional := llms[strings.Index(llms, "## Optional"):]
	if !strings.Contains(optional, "[Deep](sub/deep.html.md)") {
		t.Errorf("level-3 page missing from ## Optional:\n%s", optional)
	}

	full := read(t, filepath.Join(outDir, "llms-full.txt"))
	if !strings.Contains(full, "The guide explains the thing") || !strings.Contains(full, "A level two page.") {
		t.Errorf("llms-full.txt should carry levels 1-2:\n%s", full)
	}
	if strings.Contains(full, "Everything, at length.") {
		t.Errorf("llms-full.txt must not carry the compendium:\n%s", full)
	}
	if strings.Contains(full, "A level three page.") {
		t.Errorf("llms-full.txt must not carry level 3:\n%s", full)
	}
	if got := read(t, filepath.Join(outDir, compendiumFullName)); !strings.Contains(got, "Everything, at length.") {
		t.Errorf("compendium full text missing from its own file:\n%s", got)
	}

	html := read(t, filepath.Join(outDir, "sub", "child.html"))
	if !strings.Contains(html, `<link rel="alternate" type="text/markdown" href="child.html.md">`) {
		t.Errorf("child page lacks alternate link:\n%s", html)
	}
	if !strings.Contains(html, `<link rel="describedby" type="text/plain" href="../llms.txt">`) {
		t.Errorf("child page lacks describedby link:\n%s", html)
	}
}

func TestCheckPassesAfterEmitAndFailsOnDrift(t *testing.T) {
	docsDir, agentsDir, outDir := fixture(t)
	cfg := config{docsDir: docsDir, agentsDir: agentsDir, outDir: outDir}
	if err := runEmit(cfg); err != nil {
		t.Fatalf("emit: %v", err)
	}
	if err := runCheck(cfg); err != nil {
		t.Fatalf("check after emit: %v", err)
	}

	// A missing twin fails the gate.
	if err := os.Remove(filepath.Join(outDir, "guide.html.md")); err != nil {
		t.Fatal(err)
	}
	if err := runCheck(cfg); err == nil {
		t.Error("check should fail with a missing twin")
	}

	// A stale twin fails the gate.
	if err := runEmit(cfg); err != nil {
		t.Fatalf("re-emit: %v", err)
	}
	guide := filepath.Join(docsDir, "guide.md")
	if err := os.WriteFile(guide, []byte("# Guide\n\nChanged prose.\n"), 0o644); err != nil {
		t.Fatal(err)
	}
	if err := runCheck(cfg); err == nil {
		t.Error("check should fail with a stale twin")
	}
}

func TestOverrideWithoutReasonIsRefused(t *testing.T) {
	docsDir, agentsDir, outDir := fixture(t)
	if err := os.WriteFile(filepath.Join(agentsDir, "guide.md"), []byte("# Hand-written guide twin\n"), 0o644); err != nil {
		t.Fatal(err)
	}
	cfg := config{docsDir: docsDir, agentsDir: agentsDir, outDir: outDir}
	if err := runEmit(cfg); err == nil {
		t.Error("emit should refuse an override outside the allowed set without an override-reason")
	}
	if err := os.WriteFile(filepath.Join(agentsDir, "guide.md"), []byte("<!-- override-reason: the generated twin loses the worked example -->\n\n# Hand-written guide twin\n"), 0o644); err != nil {
		t.Fatal(err)
	}
	if err := runEmit(cfg); err != nil {
		t.Errorf("emit should accept an override with a stated reason: %v", err)
	}
	if got := read(t, filepath.Join(outDir, "guide.html.md")); !strings.Contains(got, "Hand-written guide twin") {
		t.Errorf("override not used:\n%s", got)
	}
}

func TestParseSummaryLevelsAndDrafts(t *testing.T) {
	chapters := parseSummary("- [A](a.md)\n- [Section](<>)\n  - [B](b.md)\n    - [C](c.md)\n")
	if len(chapters) != 3 {
		t.Fatalf("expected 3 chapters (draft dropped), got %d", len(chapters))
	}
	if chapters[0].Level != 1 || chapters[1].Level != 2 || chapters[2].Level != 3 {
		t.Errorf("levels wrong: %+v", chapters)
	}
}

// Oracle: the operator-script argument contract (#3352) — a value-taking
// flag in trailing position is a usage error naming the flag, never a
// panic (#16401, the #3312 class in docs-twins).
func TestParseFlagsTrailingValueFlagIsUsageError(t *testing.T) {
	for _, flag := range []string{"--docs", "--agents", "--out", "--history-bin"} {
		_, err := parseFlags([]string{flag})
		if err == nil {
			t.Errorf("parseFlags([%s]): expected an error, got nil", flag)
			continue
		}
		if !strings.Contains(err.Error(), flag) || !strings.Contains(err.Error(), "requires a value") {
			t.Errorf("parseFlags([%s]): error %q does not name the flag and its missing value", flag, err)
		}
		// Also trailing after valid flags.
		_, err = parseFlags([]string{"--docs", "d", "--out", "o", flag})
		if err == nil {
			t.Errorf("parseFlags(trailing %s after valid flags): expected an error, got nil", flag)
		}
	}
	// Valid invocations are unchanged.
	cfg, err := parseFlags([]string{"--docs", "d", "--agents", "a", "--out", "o", "--history-bin", "h"})
	if err != nil {
		t.Fatalf("parseFlags(valid): %v", err)
	}
	if cfg.docsDir != "d" || cfg.agentsDir != "a" || cfg.outDir != "o" || cfg.historyBin != "h" {
		t.Errorf("parseFlags(valid) parsed wrong config: %+v", cfg)
	}
}
