// Command docs-twins builds the agent-markdown twins of the mcpx book
// (#1947, Approach B+C step 1; design defaults recorded by task 15179 on
// the issue).
//
// Every page of the docs site gets a machine-readable twin at
// <page>.html.md (the home page: index.html.md), generated from the
// preprocessed book source: the twin content is the page source rendered
// through `docs-history render --agent`, the same agent form the history
// preprocessor produces. The site also publishes llms.txt (a <=10 KB
// read-whole index: level-1 pages with notes, everything else under
// ## Optional) and llms-full.txt (levels 1-2 concatenated; the
// compendium gets its own file, llms-compendium.txt). Each built HTML
// page carries <link rel="alternate" type="text/markdown"> to its twin
// and <link rel="describedby"> to llms.txt.
//
// Hand-written agent sources ("overrides") live in pkgs/mcpx/agents-docs,
// named by the page's book-relative source path (agents-docs/index.md is
// the home page / install prompt). Per the design defaults, overrides
// are expected only for the home page and the AGENTS.md-derived loop
// pages; any other override must state its reason in an
// `override-reason:` comment near the top of the file — that is the
// drift guard, and both subcommands enforce it.
//
// Subcommands:
//
//	emit  --docs DIR --agents DIR --out DIR [--history-bin PATH]
//	      write twins + llms files into the built site tree at DIR and
//	      inject the discovery <link> tags into its HTML pages.
//	check --docs DIR --agents DIR --out DIR [--history-bin PATH]
//	      the CI gate: fail unless every page has a current twin, the
//	      llms files are current and within budget, and every HTML page
//	      carries its discovery links.
package main

import (
	"bytes"
	"fmt"
	"os"
	"os/exec"
	"path"
	"path/filepath"
	"regexp"
	"sort"
	"strings"
)

// llmsTxtBudget is the read-whole size target for llms.txt (design
// default 2: <=10 KB).
const llmsTxtBudget = 10 * 1024

// compendiumSource is the book-relative source of the compendium page,
// which gets its own full-text file instead of joining llms-full.txt.
const compendiumSource = "compendium.md"
const compendiumFullName = "llms-compendium.txt"

type chapter struct {
	Title string
	// Source is the book-relative markdown path ("" for draft chapters).
	Source string
	// Level is the SUMMARY nesting depth, starting at 1.
	Level int
}

func (c chapter) htmlRel() string {
	base := strings.TrimSuffix(c.Source, ".md")
	if path.Base(base) == "README" || path.Base(base) == "index" {
		return path.Join(path.Dir(base), "index.html")
	}
	return base + ".html"
}

// twinRel is the site-relative path of the chapter's agent twin.
func (c chapter) twinRel() string { return c.htmlRel() + ".md" }

var summaryEntry = regexp.MustCompile(`^(\s*)- \[([^\]]*)\]\(([^)]*)\)`)

// parseSummary reads an mdBook SUMMARY.md into its chapter list, in
// order. Draft chapters (target <> or empty) are dropped: mdBook emits
// no page for them.
func parseSummary(data string) []chapter {
	var chapters []chapter
	for _, line := range strings.Split(data, "\n") {
		m := summaryEntry.FindStringSubmatch(line)
		if m == nil {
			continue
		}
		target := strings.TrimSpace(m[3])
		if target == "" || target == "<>" {
			continue
		}
		chapters = append(chapters, chapter{
			Title:  m[2],
			Source: target,
			Level:  len(m[1])/2 + 1,
		})
	}
	return chapters
}

type config struct {
	docsDir    string
	agentsDir  string
	outDir     string
	historyBin string
}

// overrideReason reports whether an override file carries the required
// drift-guard reason (an `override-reason:` line near the top).
func overrideReason(content string) bool {
	lines := strings.Split(content, "\n")
	for i, line := range lines {
		if i >= 10 {
			break
		}
		if strings.Contains(line, "override-reason:") {
			return true
		}
	}
	return false
}

// pageContent returns the twin content for one chapter: the hand-written
// override when one exists, else the source rendered through the
// history tool's agent form, else the raw source.
func (cfg config) pageContent(ch chapter) (string, error) {
	if cfg.agentsDir != "" {
		overridePath := filepath.Join(cfg.agentsDir, filepath.FromSlash(ch.Source))
		if data, err := os.ReadFile(overridePath); err == nil {
			content := string(data)
			if ch.Source != "index.md" && !overrideReason(content) {
				return "", fmt.Errorf("override %s: hand-written agent sources are for the home page and the AGENTS.md-derived loop pages; any other override must state its reason in an `override-reason:` comment (design default 1, #1947)", overridePath)
			}
			return content, nil
		} else if !os.IsNotExist(err) {
			return "", err
		}
	}
	srcPath := filepath.Join(cfg.docsDir, filepath.FromSlash(ch.Source))
	data, err := os.ReadFile(srcPath)
	if err != nil {
		return "", err
	}
	if cfg.historyBin != "" {
		cmd := exec.Command(cfg.historyBin, "render", "--agent", srcPath)
		var out, stderr bytes.Buffer
		cmd.Stdout = &out
		cmd.Stderr = &stderr
		if err := cmd.Run(); err != nil {
			return "", fmt.Errorf("docs-history render --agent %s: %w: %s", srcPath, err, strings.TrimSpace(stderr.String()))
		}
		return out.String(), nil
	}
	return string(data), nil
}

// noteFor extracts a one-line note for llms.txt: the page's first prose
// line, markup stripped, truncated.
func noteFor(content string) string {
	inFence := false
	for _, line := range strings.Split(content, "\n") {
		line = strings.TrimSpace(line)
		if strings.HasPrefix(line, "```") {
			inFence = !inFence
			continue
		}
		if inFence || line == "" {
			continue
		}
		if strings.HasPrefix(line, "#") || strings.HasPrefix(line, "<") ||
			strings.HasPrefix(line, "<!--") || strings.HasPrefix(line, "|") ||
			strings.HasPrefix(line, "- ") || strings.HasPrefix(line, "> ") {
			continue
		}
		line = strings.ReplaceAll(line, "`", "")
		if len(line) > 110 {
			line = strings.TrimSpace(line[:110]) + "…"
		}
		return line
	}
	return ""
}

// generate computes every file emit would write into the site tree, as
// site-relative path -> content. It also returns the chapter list.
func (cfg config) generate() (map[string]string, []chapter, error) {
	summaryData, err := os.ReadFile(filepath.Join(cfg.docsDir, "SUMMARY.md"))
	if err != nil {
		return nil, nil, err
	}
	chapters := parseSummary(string(summaryData))
	if len(chapters) == 0 {
		return nil, nil, fmt.Errorf("no chapters parsed from %s", filepath.Join(cfg.docsDir, "SUMMARY.md"))
	}

	links := newChapterLinks(chapters)
	contents := make(map[string]string, len(chapters))
	files := make(map[string]string)
	for _, ch := range chapters {
		content, err := cfg.pageContent(ch)
		if err != nil {
			return nil, nil, err
		}
		contents[ch.Source] = links.markdown(content, ch.Source, "llms-full.txt")
		files[ch.twinRel()] = links.markdown(content, ch.Source, ch.twinRel())
	}

	// llms-full.txt: levels 1-2 concatenated, compendium excluded (it
	// gets its own file).
	var full strings.Builder
	full.WriteString("# mcpx — full text (levels 1–2)\n\n")
	full.WriteString("Every level-1 and level-2 page of the mcpx book, concatenated. ")
	full.WriteString("The compendium is in " + compendiumFullName + "; deeper pages are listed under ## Optional in llms.txt.\n")
	for _, ch := range chapters {
		if ch.Level > 2 || ch.Source == compendiumSource {
			continue
		}
		fmt.Fprintf(&full, "\n---\n\n<!-- page: %s -->\n\n%s", ch.twinRel(), contents[ch.Source])
	}
	files["llms-full.txt"] = full.String()
	if content, ok := contents[compendiumSource]; ok {
		files[compendiumFullName] = "# mcpx — the compendium (full text)\n\n<!-- page: compendium.html.md -->\n\n" + content
	}

	llms, err := buildLLMS(chapters, contents)
	if err != nil {
		return nil, nil, err
	}
	files["llms.txt"] = llms
	return files, chapters, nil
}

// buildLLMS renders llms.txt: H1, blockquote summary, a ## Docs file
// list of level-1 pages with notes, and an ## Optional list of the
// rest. Notes are sacrificed from the end of the Docs list first if
// the file would exceed the read-whole budget.
func buildLLMS(chapters []chapter, contents map[string]string) (string, error) {
	render := func(notes map[string]bool) string {
		var b strings.Builder
		b.WriteString("# mcpx\n\n")
		b.WriteString("> mcpx is code mode for MCP: every MCP server behind one daemon, driven by scripts instead of schemas. ")
		b.WriteString("Every page below links to its agent-markdown twin; llms-full.txt concatenates levels 1–2.\n\n")
		b.WriteString("## Docs\n\n")
		for _, ch := range chapters {
			if ch.Level != 1 {
				continue
			}
			fmt.Fprintf(&b, "- [%s](%s)", ch.Title, ch.twinRel())
			if notes[ch.Source] {
				if note := noteFor(contents[ch.Source]); note != "" {
					fmt.Fprintf(&b, ": %s", note)
				}
			}
			b.WriteString("\n")
		}
		b.WriteString("\n## Optional\n\n")
		for _, ch := range chapters {
			if ch.Level == 1 {
				continue
			}
			fmt.Fprintf(&b, "- [%s](%s)\n", ch.Title, ch.twinRel())
		}
		if _, ok := contents[compendiumSource]; ok {
			fmt.Fprintf(&b, "- [The compendium (full text)](%s)\n", compendiumFullName)
		}
		return b.String()
	}

	notes := make(map[string]bool)
	var level1 []string
	for _, ch := range chapters {
		if ch.Level == 1 {
			notes[ch.Source] = true
			level1 = append(level1, ch.Source)
		}
	}
	out := render(notes)
	for len(out) > llmsTxtBudget && len(level1) > 0 {
		// Drop notes starting from the last level-1 page.
		notes[level1[len(level1)-1]] = false
		level1 = level1[:len(level1)-1]
		out = render(notes)
	}
	if len(out) > llmsTxtBudget {
		return "", fmt.Errorf("llms.txt is %d bytes with no notes left to drop, over the %d-byte budget (design default 2, #1947): the book has outgrown the read-whole index and the split needs a human decision", len(out), llmsTxtBudget)
	}
	return out, nil
}

// linkTags returns the discovery <link> tags for a chapter's HTML page.
func linkTags(ch chapter) (alternate, describedby string) {
	depth := strings.Count(ch.htmlRel(), "/")
	describedby = `<link rel="describedby" type="text/plain" href="` + strings.Repeat("../", depth) + `llms.txt">`
	alternate = `<link rel="alternate" type="text/markdown" href="` + path.Base(ch.twinRel()) + `">`
	return alternate, describedby
}

// injectLinks adds the discovery tags to a built HTML page, once.
func injectLinks(htmlPath string, ch chapter, links chapterLinks) error {
	data, err := os.ReadFile(htmlPath)
	if err != nil {
		return err
	}
	alternate, describedby := linkTags(ch)
	content := links.html(string(data), ch)
	if strings.Contains(content, `rel="alternate" type="text/markdown"`) {
		if content == string(data) {
			return nil
		}
		return os.WriteFile(htmlPath, []byte(content), 0o644)
	}
	idx := strings.Index(content, "</head>")
	if idx < 0 {
		return fmt.Errorf("%s: no </head> to inject discovery links into", htmlPath)
	}
	content = content[:idx] + "  " + alternate + "\n  " + describedby + "\n" + content[idx:]
	return os.WriteFile(htmlPath, []byte(content), 0o644)
}

func runEmit(cfg config) error {
	files, chapters, err := cfg.generate()
	if err != nil {
		return err
	}
	names := make([]string, 0, len(files))
	for name := range files {
		names = append(names, name)
	}
	sort.Strings(names)
	for _, name := range names {
		dest := filepath.Join(cfg.outDir, filepath.FromSlash(name))
		if err := os.MkdirAll(filepath.Dir(dest), 0o755); err != nil {
			return err
		}
		if err := os.WriteFile(dest, []byte(files[name]), 0o644); err != nil {
			return err
		}
	}
	links := newChapterLinks(chapters)
	for _, ch := range chapters {
		htmlPath := filepath.Join(cfg.outDir, filepath.FromSlash(ch.htmlRel()))
		if _, err := os.Stat(htmlPath); err != nil {
			return fmt.Errorf("built page missing for chapter %s (%s): emit runs after mdbook build", ch.Source, htmlPath)
		}
		if err := injectLinks(htmlPath, ch, links); err != nil {
			return err
		}
	}
	fmt.Printf("docs-twins: wrote %d files (twins + llms) for %d chapters\n", len(files), len(chapters))
	return nil
}

func runCheck(cfg config) error {
	files, chapters, err := cfg.generate()
	if err != nil {
		return err
	}
	var problems []string
	names := make([]string, 0, len(files))
	for name := range files {
		names = append(names, name)
	}
	sort.Strings(names)
	for _, name := range names {
		dest := filepath.Join(cfg.outDir, filepath.FromSlash(name))
		data, err := os.ReadFile(dest)
		if err != nil {
			problems = append(problems, fmt.Sprintf("missing %s", name))
			continue
		}
		if string(data) != files[name] {
			problems = append(problems, fmt.Sprintf("stale %s (regenerate with docs-twins emit)", name))
		}
	}
	if len(files["llms.txt"]) > llmsTxtBudget {
		problems = append(problems, fmt.Sprintf("llms.txt over the %d-byte budget", llmsTxtBudget))
	}
	links := newChapterLinks(chapters)
	for _, ch := range chapters {
		htmlPath := filepath.Join(cfg.outDir, filepath.FromSlash(ch.htmlRel()))
		data, err := os.ReadFile(htmlPath)
		if err != nil {
			problems = append(problems, fmt.Sprintf("missing built page %s", ch.htmlRel()))
			continue
		}
		if links.html(string(data), ch) != string(data) {
			problems = append(problems, fmt.Sprintf("%s links to an unpublished README.html", ch.htmlRel()))
		}
		alternate, describedby := linkTags(ch)
		if !strings.Contains(string(data), alternate) {
			problems = append(problems, fmt.Sprintf("%s lacks its text/markdown alternate link", ch.htmlRel()))
		}
		if !strings.Contains(string(data), describedby) {
			problems = append(problems, fmt.Sprintf("%s lacks its llms.txt describedby link", ch.htmlRel()))
		}
	}
	if len(problems) > 0 {
		return fmt.Errorf("docs-twins check failed:\n  - %s", strings.Join(problems, "\n  - "))
	}
	fmt.Printf("docs-twins: check passed (%d chapters, all twins current)\n", len(chapters))
	return nil
}

func parseFlags(args []string) (config, error) {
	var cfg config
	for i := 0; i < len(args); i++ {
		switch args[i] {
		case "--docs", "--agents", "--out", "--history-bin":
			flag := args[i]
			if i+1 >= len(args) {
				return cfg, fmt.Errorf("%s requires a value", flag)
			}
			i++
			switch flag {
			case "--docs":
				cfg.docsDir = args[i]
			case "--agents":
				cfg.agentsDir = args[i]
			case "--out":
				cfg.outDir = args[i]
			case "--history-bin":
				cfg.historyBin = args[i]
			}
		default:
			return cfg, fmt.Errorf("unknown argument %q", args[i])
		}
	}
	if cfg.docsDir == "" || cfg.outDir == "" {
		return cfg, fmt.Errorf("--docs and --out are required")
	}
	return cfg, nil
}

func main() {
	if len(os.Args) < 2 {
		fmt.Fprintln(os.Stderr, "usage: docs-twins <emit|check> --docs DIR [--agents DIR] --out DIR [--history-bin PATH]")
		os.Exit(2)
	}
	cfg, err := parseFlags(os.Args[2:])
	if err != nil {
		fmt.Fprintln(os.Stderr, "docs-twins:", err)
		os.Exit(2)
	}
	switch os.Args[1] {
	case "emit":
		err = runEmit(cfg)
	case "check":
		err = runCheck(cfg)
	default:
		fmt.Fprintf(os.Stderr, "docs-twins: unknown subcommand %q\n", os.Args[1])
		os.Exit(2)
	}
	if err != nil {
		fmt.Fprintln(os.Stderr, "docs-twins:", err)
		os.Exit(1)
	}
}
