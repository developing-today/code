// Command docs-history is the mcpx compendium history tool (#1865,
// design #1863 Approach C). One parser serves three subcommands:
//
//	render  — emit the since marker and closed-by-default <details>
//	          histories (bubble-up: own entries in full, one line per
//	          descendant change grouped by release), or the agent-twin
//	          form with --agent. With no file arguments it acts as an
//	          mdBook-style external preprocessor: book JSON on stdin,
//	          book JSON on stdout.
//	stamp   — the release step: replace `since: unreleased` with the
//	          release version, fold docs/changes/*.md fragments into
//	          the named sections' `changed` lists, delete the
//	          fragments. Idempotent; fails on an unknown section ID.
//	check   — the CI gate: duplicate IDs, bad `since` values, fragments
//	          naming unknown sections, and (with --diff) prose changes
//	          to a section with no fragment naming it and no trivial
//	          marker.
//
// The metadata format is specified in docs/release-runbook.md.
package main

import (
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"regexp"
	"sort"
	"strconv"
	"strings"
	"time"
	"unicode"
)

// ---------- model ----------

type Change struct {
	Version string
	Date    string
	Summary string
}

func (c Change) String() string {
	if c.Date != "" {
		return c.Version + " (" + c.Date + "): " + c.Summary
	}
	return c.Version + ": " + c.Summary
}

type KeyBlock struct {
	Start int // fence opening line index
	End   int // fence closing line index
	Keys  map[string]string
	Order []string
	// line index of the `changed:` key line, -1 if absent; list item
	// lines follow it.
	ChangedKeyLine  int
	ChangedItemLast int // last list-item line index under changed:
}

type Section struct {
	Heading  string // heading text, markup intact
	Level    int
	Line     int // heading line index
	ID       string
	Explicit bool // ID came from metadata, not the slug
	Since    string
	Status   string
	Changed  []Change
	KeyBlock *KeyBlock
	// HeaderEnd is the last line index of the header zone (heading,
	// key block, compact markers). Prose starts after it.
	HeaderEnd int
	RangeEnd  int // first line index NOT in this section (next heading of level <= Level, or EOF)
	Children  []*Section
	Parent    *Section
}

type Document struct {
	Path     string
	Lines    []string
	Sections []*Section // flat, document order
	Roots    []*Section
	ByID     map[string]*Section
}

var (
	headingRe     = regexp.MustCompile(`^(#{1,6})\s+(.*?)\s*$`)
	linkRe        = regexp.MustCompile(`\[([^\]]*)\]\([^)]*\)`)
	keyLineRe     = regexp.MustCompile(`^([a-z][a-z0-9-]*):\s*(.*)$`)
	changedItemRe = regexp.MustCompile(`^\s+-\s+(.*)$`)
	changeValRe   = regexp.MustCompile(`^(\S+)\s+\((\d{4}-\d{2}-\d{2})\):\s*(.*)$`)
	changeValAlt  = regexp.MustCompile(`^(\S+):\s*(.*)$`)
	compactRe     = regexp.MustCompile(`^<!--\s*(.*?)\s*-->$`)
	semverRe      = regexp.MustCompile(`^\d+\.\d+\.\d+$`)
	dateRe        = regexp.MustCompile(`^\d{4}-\d{2}-\d{2}$`)
	sinceUnrelRe  = regexp.MustCompile(`since:\s+unreleased\b`)
)

// Slugify freezes a heading's GitHub-style slug as its section ID.
func Slugify(heading string) string {
	s := linkRe.ReplaceAllString(heading, "$1")
	s = strings.ReplaceAll(s, "~~", "")
	s = strings.ReplaceAll(s, "`", "")
	s = strings.ToLower(s)
	var b strings.Builder
	for _, r := range s {
		if unicode.IsLetter(r) || unicode.IsDigit(r) || r == '-' || r == '_' || r == ' ' {
			b.WriteRune(r)
		}
	}
	return strings.Join(strings.Fields(b.String()), "-")
}

// isFence reports whether a line opens or closes a fenced code block.
func isFence(line string) bool {
	return strings.HasPrefix(strings.TrimRight(line, " \t"), "```")
}

// isKeyBlockFence reports whether the fence opening at line start is
// a metadata key block: one of its first lines carries an id:,
// created: or status: key (migrated blocks lead with id:).
func isKeyBlockFence(lines []string, start, end int) bool {
	for i := start + 1; i < end && i <= start+3; i++ {
		if strings.HasPrefix(lines[i], "created:") || strings.HasPrefix(lines[i], "status:") || strings.HasPrefix(lines[i], "id:") {
			return true
		}
	}
	return false
}

// ParseDocument parses markdown into sections with their metadata.
// Fenced code is never mistaken for headings or metadata, except that
// a fence directly under a heading whose first line is `created:` or
// `status:` is recognised as that heading's key block.
func ParseDocument(path string, content string) *Document {
	lines := strings.Split(content, "\n")
	doc := &Document{Path: path, Lines: lines, ByID: map[string]*Section{}}

	// First pass: headings outside fences, and fence spans.
	type fenceSpan struct{ start, end int }
	var fences []fenceSpan
	inFence := false
	fenceStart := 0
	var headings []int
	for i, ln := range lines {
		if isFence(ln) {
			if inFence {
				fences = append(fences, fenceSpan{fenceStart, i})
				inFence = false
			} else {
				inFence = true
				fenceStart = i
			}
			continue
		}
		if inFence {
			continue
		}
		if headingRe.MatchString(ln) {
			headings = append(headings, i)
		}
	}
	fenceByStart := map[int]int{}
	for _, f := range fences {
		fenceByStart[f.start] = f.end
	}

	slugCount := map[string]int{}
	for hi, hl := range headings {
		m := headingRe.FindStringSubmatch(lines[hl])
		sec := &Section{Heading: m[2], Level: len(m[1]), Line: hl, HeaderEnd: hl}
		// Range: until the next heading of level <= ours.
		sec.RangeEnd = len(lines)
		for _, nl := range headings[hi+1:] {
			if nm := headingRe.FindStringSubmatch(lines[nl]); len(nm[1]) <= sec.Level {
				sec.RangeEnd = nl
				break
			}
		}
		// Key block: first non-blank line after the heading (a
		// compact marker may sit between them) opens a fence whose
		// first content line is created:/status:.
		j := hl + 1
		for j < len(lines) && (strings.TrimSpace(lines[j]) == "" || compactRe.MatchString(strings.TrimSpace(lines[j]))) {
			j++
		}
		if j < len(lines) {
			if end, ok := fenceByStart[j]; ok && end > j+1 && isKeyBlockFence(lines, j, end) {
				sec.KeyBlock = parseKeyBlock(lines, j, end)
				sec.HeaderEnd = end
			}
		}
		// Compact markers between the heading and an accompanying
		// key block come first: the scan below starts after the
		// block, so changed: comments here would otherwise never
		// be parsed and their history silently dropped (Codex,
		// PR 3266).
		if sec.KeyBlock != nil {
			for i := hl + 1; i < sec.KeyBlock.Start; i++ {
				cm := compactRe.FindStringSubmatch(strings.TrimSpace(lines[i]))
				if cm == nil {
					continue
				}
				body := cm[1]
				if strings.HasPrefix(body, "changed:") {
					if ch, ok := parseChange(strings.TrimSpace(strings.TrimPrefix(body, "changed:"))); ok {
						sec.Changed = append(sec.Changed, ch)
					}
					continue
				}
				for _, kv := range parseCompactPairs(body) {
					switch kv[0] {
					case "id":
						sec.ID = kv[1]
						sec.Explicit = true
					case "since":
						sec.Since = kv[1]
					case "status":
						sec.Status = kv[1]
					}
				}
			}
		}
		// Compact markers and changed comments in the header zone:
		// comment lines after the heading / key block, allowing
		// blank lines between.
		k := sec.HeaderEnd + 1
		for k < len(lines) {
			ln := lines[k]
			if strings.TrimSpace(ln) == "" {
				k++
				continue
			}
			cm := compactRe.FindStringSubmatch(strings.TrimSpace(ln))
			if cm == nil {
				break
			}
			body := cm[1]
			if strings.HasPrefix(body, "changed:") {
				if ch, ok := parseChange(strings.TrimSpace(strings.TrimPrefix(body, "changed:"))); ok {
					sec.Changed = append(sec.Changed, ch)
				}
			} else {
				for _, part := range strings.Fields(body) {
					_ = part
				}
				for _, kv := range parseCompactPairs(body) {
					switch kv[0] {
					case "id":
						sec.ID = kv[1]
						sec.Explicit = true
					case "since":
						sec.Since = kv[1]
					case "status":
						sec.Status = kv[1]
					}
				}
			}
			sec.HeaderEnd = k
			k++
		}
		if sec.KeyBlock != nil {
			kb := sec.KeyBlock
			if v, ok := kb.Keys["id"]; ok {
				sec.ID = v
				sec.Explicit = true
			}
			if v, ok := kb.Keys["since"]; ok {
				sec.Since = v
			}
			if v, ok := kb.Keys["status"]; ok {
				sec.Status = v
			}
			sec.Changed = append(sec.Changed, kbChanges(lines, kb)...)
		}
		if sec.ID == "" {
			slug := Slugify(sec.Heading)
			sec.ID = slug
			if n := slugCount[slug]; n > 0 {
				sec.ID = slug + "-" + strconv.Itoa(n)
			}
			slugCount[slug]++
		}
		doc.Sections = append(doc.Sections, sec)
		if prev, dup := doc.ByID[sec.ID]; dup && prev.Explicit && sec.Explicit {
			// Keep the first; check reports the duplicate.
			doc.ByID[sec.ID+"#dup"+strconv.Itoa(hl)] = sec
		} else if !dup {
			doc.ByID[sec.ID] = sec
		}
	}

	// Document-level blocks: a level-1 section whose key block sits
	// under the h1 and its lede (container.md) rather than directly
	// under the heading adopts the first created:/status: fence
	// before its first sub-heading.
	for si, sec := range doc.Sections {
		if sec.Level != 1 || sec.KeyBlock != nil {
			continue
		}
		limit := len(lines)
		if si+1 < len(doc.Sections) {
			limit = doc.Sections[si+1].Line
		}
		for j := sec.Line + 1; j < limit; j++ {
			if end, ok := fenceByStart[j]; ok && end > j+1 && isKeyBlockFence(lines, j, end) {
				{
					sec.KeyBlock = parseKeyBlock(lines, j, end)
					sec.HeaderEnd = end
					if v, ok := sec.KeyBlock.Keys["id"]; ok {
						// Reindex: ByID still maps the heading
						// slug to this section, but Stamp looks
						// fragments up in ByID under the adopted
						// ID (Codex, PR 3266).
						if doc.ByID[sec.ID] == sec {
							delete(doc.ByID, sec.ID)
						}
						sec.ID = v
						sec.Explicit = true
						doc.ByID[sec.ID] = sec
					}
					if v, ok := sec.KeyBlock.Keys["since"]; ok {
						sec.Since = v
					}
					if v, ok := sec.KeyBlock.Keys["status"]; ok {
						sec.Status = v
					}
					sec.Changed = append(sec.Changed, kbChanges(lines, sec.KeyBlock)...)
					break
				}
			}
		}
	}

	// Tree.
	var stack []*Section
	for _, sec := range doc.Sections {
		for len(stack) > 0 && stack[len(stack)-1].Level >= sec.Level {
			stack = stack[:len(stack)-1]
		}
		if len(stack) > 0 {
			sec.Parent = stack[len(stack)-1]
			sec.Parent.Children = append(sec.Parent.Children, sec)
		} else {
			doc.Roots = append(doc.Roots, sec)
		}
		stack = append(stack, sec)
	}
	return doc
}

var compactPairRe = regexp.MustCompile(`([a-z][a-z0-9-]*):\s*(\S+)`)

func parseCompactPairs(body string) [][2]string {
	var out [][2]string
	for _, m := range compactPairRe.FindAllStringSubmatch(body, -1) {
		out = append(out, [2]string{m[1], m[2]})
	}
	return out
}

func parseKeyBlock(lines []string, start, end int) *KeyBlock {
	kb := &KeyBlock{Start: start, End: end, Keys: map[string]string{}, ChangedKeyLine: -1, ChangedItemLast: -1}
	lastKey := ""
	for i := start + 1; i < end; i++ {
		ln := lines[i]
		if m := keyLineRe.FindStringSubmatch(ln); m != nil {
			lastKey = m[1]
			kb.Keys[lastKey] = m[2]
			kb.Order = append(kb.Order, lastKey)
			if lastKey == "changed" {
				kb.ChangedKeyLine = i
				kb.ChangedItemLast = i
			}
			continue
		}
		if lastKey == "changed" && changedItemRe.MatchString(ln) {
			kb.ChangedItemLast = i
			continue
		}
		if lastKey != "" && lastKey != "changed" && (strings.HasPrefix(ln, " ") || strings.HasPrefix(ln, "\t")) {
			kb.Keys[lastKey] += " " + strings.TrimSpace(ln)
		}
	}
	return kb
}

func kbChanges(lines []string, kb *KeyBlock) []Change {
	if kb.ChangedKeyLine < 0 {
		return nil
	}
	var out []Change
	for i := kb.ChangedKeyLine + 1; i <= kb.ChangedItemLast && i < kb.End; i++ {
		if m := changedItemRe.FindStringSubmatch(lines[i]); m != nil {
			if ch, ok := parseChange(strings.TrimSpace(m[1])); ok {
				out = append(out, ch)
			}
		}
	}
	return out
}

func parseChange(s string) (Change, bool) {
	if m := changeValRe.FindStringSubmatch(s); m != nil {
		return Change{Version: m[1], Date: m[2], Summary: m[3]}, true
	}
	if m := changeValAlt.FindStringSubmatch(s); m != nil {
		return Change{Version: m[1], Summary: m[2]}, true
	}
	return Change{}, false
}

// ValidSince reports whether a since value is `unreleased`, the
// migration sentinel `pre-release`, or a release version.
func ValidSince(s string) bool {
	return s == "unreleased" || s == "pre-release" || semverRe.MatchString(s)
}

// subtreeChanges returns a section's own changes and every
// descendant's, in document order.
func subtreeChanges(sec *Section) (own []Change, desc []descChange) {
	own = sec.Changed
	var walk func(s *Section)
	walk = func(s *Section) {
		for _, ch := range s.Children {
			for _, c := range ch.Changed {
				desc = append(desc, descChange{Section: ch, Change: c})
			}
			walk(ch)
		}
	}
	walk(sec)
	return own, desc
}

type descChange struct {
	Section *Section
	Change  Change
}

func latestChange(all []Change) Change {
	latest := all[0]
	for _, c := range all[1:] {
		if c.Date >= latest.Date {
			latest = c
		}
	}
	return latest
}

// ---------- render ----------

// Render produces the published form of a document: a since marker
// under each section that carries one, and one closed-by-default
// <details> history per section whose subtree has history — its own
// entries in full, then one line per descendant change grouped by
// release. In agent form there are no disclosures: a since/status
// line and a history link per section.
func Render(doc *Document, agent bool) string {
	insertAfter := map[int][]string{}
	for _, sec := range doc.Sections {
		var block []string
		own, desc := subtreeChanges(sec)
		total := len(own) + len(desc)
		if agent {
			var parts []string
			if sec.Since != "" {
				parts = append(parts, "Since: "+sec.Since)
			}
			if sec.Status != "" {
				parts = append(parts, "Status: "+sec.Status)
			}
			if len(parts) > 0 {
				block = append(block, strings.Join(parts, " · "))
			}
			if total > 0 {
				block = append(block, "History: [#"+sec.ID+"](#"+sec.ID+")")
			}
		} else {
			if sec.Since != "" {
				block = append(block, "Since: "+sec.Since)
			}
			if total > 0 {
				all := append(append([]Change{}, own...), func() []Change {
					var cs []Change
					for _, d := range desc {
						cs = append(cs, d.Change)
					}
					return cs
				}()...)
				latest := latestChange(all)
				block = append(block, "")
				block = append(block, fmt.Sprintf("<details><summary>History -- %d changes, latest %s</summary>", total, latest.String()))
				block = append(block, "")
				for _, c := range own {
					block = append(block, "- "+c.String())
				}
				// Descendant one-liners grouped by release, in
				// first-seen release order.
				var versions []string
				byVersion := map[string][]descChange{}
				for _, d := range desc {
					v := d.Change.Version
					if _, ok := byVersion[v]; !ok {
						versions = append(versions, v)
					}
					byVersion[v] = append(byVersion[v], d)
				}
				for _, v := range versions {
					block = append(block, "")
					block = append(block, "**"+v+"**")
					for _, d := range byVersion[v] {
						block = append(block, fmt.Sprintf("- [%s](#%s) -- %s", d.Section.Heading, d.Section.ID, d.Change.Summary))
					}
				}
				block = append(block, "")
				block = append(block, "</details>")
			}
		}
		if len(block) > 0 {
			insertAfter[sec.HeaderEnd] = block
		}
	}
	// A section whose frozen ID no longer matches its heading's slug
	// (the heading was reworded) gets an explicit anchor, so history
	// links to #<id> resolve in the rendered document.
	insertBefore := map[int][]string{}
	for _, sec := range doc.Sections {
		if sec.Explicit && sec.ID != Slugify(sec.Heading) {
			insertBefore[sec.Line] = []string{`<a id="` + sec.ID + `"></a>`}
		}
	}
	var out []string
	for i, ln := range doc.Lines {
		if before, ok := insertBefore[i]; ok {
			out = append(out, before...)
		}
		out = append(out, ln)
		if block, ok := insertAfter[i]; ok {
			out = append(out, block...)
		}
	}
	return strings.Join(out, "\n")
}

// renderMdBook implements the external-preprocessor protocol: stdin
// is [context, book] JSON; every chapter's content is rendered and
// the book is written back to stdout.
func renderMdBook(data []byte, agent bool) ([]byte, error) {
	var pair []json.RawMessage
	if err := json.Unmarshal(data, &pair); err != nil {
		return nil, err
	}
	if len(pair) != 2 {
		return nil, fmt.Errorf("expected [context, book] pair")
	}
	var book map[string]interface{}
	if err := json.Unmarshal(pair[1], &book); err != nil {
		return nil, err
	}
	var walk func(items []interface{})
	walk = func(items []interface{}) {
		for _, it := range items {
			m, ok := it.(map[string]interface{})
			if !ok {
				continue
			}
			if ch, ok := m["Chapter"].(map[string]interface{}); ok {
				if content, ok := ch["content"].(string); ok {
					ch["content"] = Render(ParseDocument("", content), agent)
				}
				if subs, ok := ch["sub_items"].([]interface{}); ok {
					walk(subs)
				}
			}
		}
	}
	if sections, ok := book["sections"].([]interface{}); ok {
		walk(sections)
	}
	return json.Marshal(book)
}

// ---------- fragments ----------

type Fragment struct {
	Path     string
	Sections []string
	Kind     string
	Summary  string
}

var changelogKinds = map[string]bool{
	"Added": true, "Changed": true, "Deprecated": true,
	"Removed": true, "Fixed": true, "Security": true,
}

func ParseFragment(path string, content string) (*Fragment, error) {
	f := &Fragment{Path: path}
	for _, ln := range strings.Split(content, "\n") {
		m := keyLineRe.FindStringSubmatch(strings.TrimSpace(ln))
		if m == nil {
			continue
		}
		switch m[1] {
		case "sections":
			for _, s := range strings.Split(m[2], ",") {
				if s = strings.TrimSpace(s); s != "" {
					f.Sections = append(f.Sections, s)
				}
			}
		case "kind":
			f.Kind = strings.TrimSpace(m[2])
		case "summary":
			f.Summary = strings.TrimSpace(m[2])
		}
	}
	if len(f.Sections) == 0 {
		return nil, fmt.Errorf("%s: fragment names no sections", path)
	}
	if !changelogKinds[f.Kind] {
		return nil, fmt.Errorf("%s: kind %q is not a Keep a Changelog kind", path, f.Kind)
	}
	if f.Summary == "" {
		return nil, fmt.Errorf("%s: fragment has no summary", path)
	}
	return f, nil
}

func loadFragments(dir string) ([]*Fragment, error) {
	entries, err := os.ReadDir(dir)
	if err != nil {
		if os.IsNotExist(err) {
			return nil, nil
		}
		return nil, err
	}
	var out []*Fragment
	for _, e := range entries {
		if e.IsDir() || !strings.HasSuffix(e.Name(), ".md") {
			continue
		}
		p := filepath.Join(dir, e.Name())
		data, err := os.ReadFile(p)
		if err != nil {
			return nil, err
		}
		f, err := ParseFragment(p, string(data))
		if err != nil {
			return nil, err
		}
		out = append(out, f)
	}
	sort.Slice(out, func(i, j int) bool { return out[i].Path < out[j].Path })
	return out, nil
}

// ---------- stamp ----------

// Stamp replaces `since: unreleased` with version in every file,
// folds each fragment into its sections' changed lists, and deletes
// the fragments. All fragments are validated before any file is
// written, so an unknown section ID fails the whole stamp.
func Stamp(files []string, version, date, changesDir string) error {
	if !semverRe.MatchString(version) {
		return fmt.Errorf("version %q is not a release version (want X.Y.Z)", version)
	}
	// Validate the date up front, before any file is written or any
	// fragment consumed: an unparseable date would be written into
	// changed entries its own parser cannot read back (Codex, PR 3266).
	if !dateRe.MatchString(date) {
		return fmt.Errorf("date %q is not a stamp date (want YYYY-MM-DD)", date)
	}
	if _, err := time.Parse("2006-01-02", date); err != nil {
		return fmt.Errorf("date %q is not a valid calendar date: %w", date, err)
	}
	docs := map[string]*Document{}
	allIDs := map[string]*Section{}
	for _, f := range files {
		data, err := os.ReadFile(f)
		if err != nil {
			return err
		}
		doc := ParseDocument(f, string(data))
		docs[f] = doc
		for id, sec := range doc.ByID {
			if !strings.Contains(id, "#dup") {
				allIDs[id] = sec
			}
		}
	}
	if changesDir == "" && len(files) > 0 {
		changesDir = filepath.Join(filepath.Dir(files[0]), "changes")
	}
	fragments, err := loadFragments(changesDir)
	if err != nil {
		return err
	}
	for _, frag := range fragments {
		for _, id := range frag.Sections {
			if _, ok := allIDs[id]; !ok {
				return fmt.Errorf("%s: fragment names unknown section ID %q", frag.Path, id)
			}
		}
	}

	// A fragment target must be unambiguous across the stamped set:
	// IDs are unqualified, so an ID living in two documents cannot be
	// folded safely.
	idDocs := map[string]map[string]bool{}
	for path, doc := range docs {
		for _, sec := range doc.Sections {
			if idDocs[sec.ID] == nil {
				idDocs[sec.ID] = map[string]bool{}
			}
			idDocs[sec.ID][path] = true
		}
	}
	for _, frag := range fragments {
		for _, id := range frag.Sections {
			if len(idDocs[id]) > 1 {
				return fmt.Errorf("%s: section ID %q is ambiguous across documents %v", frag.Path, id, idDocs[id])
			}
		}
	}
	// Consolidate every fragment entry per section first: two
	// fragments naming one section produce ONE changed list, not one
	// `changed:` key per fragment.
	entriesBySection := map[*Section][]Change{}
	for _, frag := range fragments {
		for _, id := range frag.Sections {
			sec := allIDs[id]
			entry := Change{Version: version, Date: date, Summary: frag.Summary}
			dup := false
			for _, c := range append(append([]Change{}, sec.Changed...), entriesBySection[sec]...) {
				if c == entry {
					dup = true
				}
			}
			if !dup {
				entriesBySection[sec] = append(entriesBySection[sec], entry)
			}
		}
	}

	// Edits are line insertions/replacements per document, applied
	// bottom-up so indices stay valid.
	type edit struct {
		line    int  // line index the edit anchors at
		replace bool // replace the line, else insert after it
		text    []string
	}
	for path, doc := range docs {
		var edits []edit
		for _, sec := range doc.Sections {
			// since: unreleased -> version (key block line and
			// compact marker).
			if sec.Since == "unreleased" {
				if sec.KeyBlock != nil {
					for i := sec.KeyBlock.Start + 1; i < sec.KeyBlock.End; i++ {
						if m := keyLineRe.FindStringSubmatch(doc.Lines[i]); m != nil && m[1] == "since" {
							edits = append(edits, edit{line: i, replace: true,
								text: []string{strings.Replace(doc.Lines[i], "unreleased", version, 1)}})
						}
					}
				}
				for i := sec.Line + 1; i <= sec.HeaderEnd && i < len(doc.Lines); i++ {
					if sec.KeyBlock != nil && i >= sec.KeyBlock.Start && i <= sec.KeyBlock.End {
						continue // the key-block edit above owns these lines
					}
					if sinceUnrelRe.MatchString(doc.Lines[i]) {
						edits = append(edits, edit{line: i, replace: true,
							text: []string{sinceUnrelRe.ReplaceAllString(doc.Lines[i], "since: "+version)}})
					}
				}
			}
			// Fold the section's consolidated fragment entries in
			// a single edit.
			entries := entriesBySection[sec]
			if len(entries) == 0 {
				continue
			}
			var itemLines []string
			for _, e := range entries {
				itemLines = append(itemLines, "  - "+e.String())
			}
			if sec.KeyBlock != nil {
				kb := sec.KeyBlock
				if kb.ChangedKeyLine >= 0 {
					edits = append(edits, edit{line: kb.ChangedItemLast, text: itemLines})
				} else {
					edits = append(edits, edit{line: kb.End - 1,
						text: append([]string{"changed:"}, itemLines...)})
				}
			} else {
				var comments []string
				for _, e := range entries {
					comments = append(comments, "<!-- changed: "+e.String()+" -->")
				}
				edits = append(edits, edit{line: sec.HeaderEnd, text: comments})
			}
		}
		if len(edits) == 0 {
			continue
		}
		sort.SliceStable(edits, func(i, j int) bool { return edits[i].line > edits[j].line })
		lines := append([]string{}, doc.Lines...)
		for _, e := range edits {
			if e.replace {
				lines[e.line] = e.text[0]
			} else {
				tail := append([]string{}, lines[e.line+1:]...)
				lines = append(lines[:e.line+1], append(e.text, tail...)...)
			}
		}
		if err := os.WriteFile(path, []byte(strings.Join(lines, "\n")), 0o644); err != nil {
			return err
		}
	}
	for _, frag := range fragments {
		if err := os.Remove(frag.Path); err != nil {
			return err
		}
	}
	return nil
}

// ---------- check ----------

// Check validates documents and, when diff is non-empty, requires a
// fragment for every section whose prose changed (unless trivial).
// It returns the list of problems found.
func Check(files []string, diff string, trivial bool, changesDir string) []string {
	var problems []string
	docs := map[string]*Document{}
	allIDs := map[string]bool{}
	for _, f := range files {
		data, err := os.ReadFile(f)
		if err != nil {
			problems = append(problems, err.Error())
			continue
		}
		doc := ParseDocument(f, string(data))
		docs[f] = doc
		seen := map[string]int{}
		for _, sec := range doc.Sections {
			seen[sec.ID]++
			if sec.Explicit {
				allIDs[sec.ID] = true
			}
			if sec.Since != "" && !ValidSince(sec.Since) {
				problems = append(problems, fmt.Sprintf("%s: section %q has bad since value %q (want unreleased, pre-release, or X.Y.Z)", f, sec.ID, sec.Since))
			}
		}
		for id, n := range seen {
			if n > 1 {
				problems = append(problems, fmt.Sprintf("%s: duplicate section ID %q (%d sections)", f, id, n))
			}
		}
	}
	if changesDir == "" && len(files) > 0 {
		changesDir = filepath.Join(filepath.Dir(files[0]), "changes")
	}
	fragments, err := loadFragments(changesDir)
	if err != nil {
		problems = append(problems, err.Error())
		fragments = nil
	}
	// Fragment targets are unqualified IDs: an ID that lives in more
	// than one checked document is ambiguous and cannot be folded or
	// gated safely.
	idDocCount := map[string]map[string]bool{}
	for path, doc := range docs {
		for _, sec := range doc.Sections {
			if idDocCount[sec.ID] == nil {
				idDocCount[sec.ID] = map[string]bool{}
			}
			idDocCount[sec.ID][path] = true
		}
	}
	// When a diff is being checked, only a fragment the PR itself
	// adds or modifies counts toward the gate: a fragment left in
	// docs/changes by an earlier PR records that PR's change and
	// must not silently cover a later PR's prose change to the same
	// section (Codex, PR 3266).
	var diffFiles map[string]map[int]bool
	if diff != "" {
		diffFiles = diffChangedLines(diff)
	}
	fragmentInDiff := func(frag *Fragment) bool {
		if diff == "" {
			return true
		}
		for df := range diffFiles {
			if sameDocPath(df, frag.Path) {
				return true
			}
		}
		return false
	}
	fragmented := map[string]bool{}
	for _, frag := range fragments {
		for _, id := range frag.Sections {
			if fragmentInDiff(frag) {
				fragmented[id] = true
			}
			if !allIDs[id] && !anySlugID(docs, id) {
				problems = append(problems, fmt.Sprintf("%s: fragment names unknown section ID %q", frag.Path, id))
			} else if len(idDocCount[id]) > 1 {
				problems = append(problems, fmt.Sprintf("%s: fragment target section ID %q is ambiguous across documents %v", frag.Path, id, idDocCount[id]))
			}
		}
	}
	if diff != "" && !trivial {
		changed := diffFiles
		for path, doc := range docs {
			for diffPath, lineSet := range changed {
				if !sameDocPath(diffPath, path) {
					continue
				}
				reported := map[string]bool{}
				for ln := range lineSet {
					sec := ownerSection(doc, ln)
					if sec == nil || reported[sec.ID] {
						continue
					}
					reported[sec.ID] = true
					if !fragmented[sec.ID] {
						problems = append(problems, fmt.Sprintf("%s: section %q prose changed with no fragment naming it and no trivial marker", path, sec.ID))
					}
				}
			}
		}
	}
	sort.Strings(problems)
	return problems
}

func anySlugID(docs map[string]*Document, id string) bool {
	for _, doc := range docs {
		for _, sec := range doc.Sections {
			if sec.ID == id {
				return true
			}
		}
	}
	return false
}

// ownerSection returns the section a (0-based) line belongs to: the
// last section whose heading is at or before the line. A line inside
// that section's header zone (heading, key block, compact markers)
// is metadata, not prose, and has no owner. Prose between a heading
// and a document-level key block (the lede in container.md) IS prose
// and keeps its owner.
func ownerSection(doc *Document, line int) *Section {
	var cur *Section
	for _, sec := range doc.Sections {
		if sec.Line > line {
			break
		}
		cur = sec
	}
	if cur == nil || line == cur.Line {
		return nil
	}
	if cur.KeyBlock != nil && line >= cur.KeyBlock.Start && line <= cur.KeyBlock.End {
		return nil
	}
	if line <= cur.HeaderEnd {
		txt := strings.TrimSpace(doc.Lines[line])
		if txt == "" || compactRe.MatchString(txt) {
			return nil
		}
		return cur // lede prose before a document-level key block
	}
	return cur
}

func sameDocPath(diffPath, docPath string) bool {
	diffPath = filepath.ToSlash(diffPath)
	docPath = filepath.ToSlash(docPath)
	return diffPath == docPath || strings.HasSuffix(diffPath, "/"+docPath) || strings.HasSuffix(docPath, "/"+diffPath)
}

var hunkRe = regexp.MustCompile(`^@@ -\d+(?:,\d+)? \+(\d+)(?:,(\d+))? @@`)

// diffChangedLines maps each file in a unified diff to the set of
// added/changed line numbers (0-based) in the new version.
func diffChangedLines(diff string) map[string]map[int]bool {
	out := map[string]map[int]bool{}
	var file string
	newLine := 0
	lastNew := -1
	hunkStart := -1
	inHunk := false
	for _, ln := range strings.Split(diff, "\n") {
		switch {
		case strings.HasPrefix(ln, "diff --git "):
			// A new file's header ends the previous file's hunk;
			// without this, header lines (including "--- a/...",
			// which looks like a deletion) bleed into it.
			inHunk = false
		case !inHunk && strings.HasPrefix(ln, "+++ b/"):
			file = strings.TrimPrefix(ln, "+++ b/")
			inHunk = false
		case !inHunk && strings.HasPrefix(ln, "+++ "):
			file = strings.TrimSpace(strings.TrimPrefix(ln, "+++ "))
			inHunk = false
		case !inHunk && strings.HasPrefix(ln, "--- "):
			inHunk = false
		case hunkRe.MatchString(ln):
			m := hunkRe.FindStringSubmatch(ln)
			start, _ := strconv.Atoi(m[1])
			newLine = start - 1
			hunkStart = newLine
			lastNew = -1
			inHunk = true
			if out[file] == nil {
				out[file] = map[int]bool{}
			}
		case inHunk && strings.HasPrefix(ln, "+"):
			out[file][newLine] = true
			lastNew = newLine
			newLine++
		case inHunk && strings.HasPrefix(ln, "-"):
			// A deleted heading line is the old form of a heading
			// (metadata), not prose of the preceding section: the
			// added heading line is itself ownerless, so a rename
			// must not demand a fragment for the untouched
			// preceding section (Codex, PR 3266).
			if headingRe.MatchString(strings.TrimSpace(strings.TrimPrefix(ln, "-"))) {
				continue
			}
			// A deleted line has no new-file position; attribute
			// it to the nearest preceding new-file line in the
			// hunk (or the hunk's first line), so a deletion-only
			// prose change still needs a fragment.
			pseudo := lastNew
			if pseudo < 0 {
				pseudo = hunkStart
			}
			if pseudo >= 0 {
				out[file][pseudo] = true
			}
		case inHunk && strings.HasPrefix(ln, "\\"):
		case inHunk:
			lastNew = newLine
			newLine++
		}
	}
	return out
}

// ---------- main ----------

func usage() {
	fmt.Fprintln(os.Stderr, "usage:")
	fmt.Fprintln(os.Stderr, "  docs-history render [--agent] <file.md>...   (no files: mdBook preprocessor on stdin)")
	fmt.Fprintln(os.Stderr, "  docs-history stamp --version X.Y.Z --date YYYY-MM-DD [--changes-dir DIR] <file.md>...")
	fmt.Fprintln(os.Stderr, "  docs-history check [--diff FILE] [--trivial] [--changes-dir DIR] <file.md>...")
}

func main() {
	args := os.Args[1:]
	if len(args) > 0 && args[0] == "supports" {
		return // mdBook capability probe: we support every renderer
	}
	if len(args) == 0 {
		// mdBook preprocessor mode.
		data, err := readAllStdin()
		if err != nil {
			fatal(err)
		}
		out, err := renderMdBook(data, false)
		if err != nil {
			fatal(err)
		}
		fmt.Println(string(out))
		return
	}
	switch args[0] {
	case "render":
		agent := false
		var files []string
		for _, a := range args[1:] {
			if a == "--agent" {
				agent = true
			} else {
				files = append(files, a)
			}
		}
		if len(files) == 0 {
			data, err := readAllStdin()
			if err != nil {
				fatal(err)
			}
			out, err := renderMdBook(data, agent)
			if err != nil {
				fatal(err)
			}
			fmt.Println(string(out))
			return
		}
		for _, f := range files {
			data, err := os.ReadFile(f)
			if err != nil {
				fatal(err)
			}
			fmt.Print(Render(ParseDocument(f, string(data)), agent))
		}
	case "stamp":
		var version, date, changesDir string
		var files []string
		rest := args[1:]
		for i := 0; i < len(rest); i++ {
			switch rest[i] {
			case "--version":
				i++
				version = rest[i]
			case "--date":
				i++
				date = rest[i]
			case "--changes-dir":
				i++
				changesDir = rest[i]
			default:
				files = append(files, rest[i])
			}
		}
		if version == "" || date == "" || len(files) == 0 {
			usage()
			os.Exit(2)
		}
		if err := Stamp(files, version, date, changesDir); err != nil {
			fatal(err)
		}
	case "check":
		var diffFile, changesDir string
		trivial := false
		var files []string
		rest := args[1:]
		for i := 0; i < len(rest); i++ {
			switch rest[i] {
			case "--diff":
				i++
				diffFile = rest[i]
			case "--trivial":
				trivial = true
			case "--changes-dir":
				i++
				changesDir = rest[i]
			default:
				files = append(files, rest[i])
			}
		}
		if len(files) == 0 {
			usage()
			os.Exit(2)
		}
		diff := ""
		if diffFile != "" {
			data, err := os.ReadFile(diffFile)
			if err != nil {
				fatal(err)
			}
			diff = string(data)
		}
		problems := Check(files, diff, trivial, changesDir)
		for _, p := range problems {
			fmt.Fprintln(os.Stderr, "check: "+p)
		}
		if len(problems) > 0 {
			os.Exit(1)
		}
		fmt.Println("docs-history check: ok")
	default:
		usage()
		os.Exit(2)
	}
}

func readAllStdin() ([]byte, error) {
	var b strings.Builder
	buf := make([]byte, 65536)
	for {
		n, err := os.Stdin.Read(buf)
		b.Write(buf[:n])
		if err != nil {
			break
		}
	}
	return []byte(b.String()), nil
}

func fatal(err error) {
	fmt.Fprintln(os.Stderr, "docs-history: "+err.Error())
	os.Exit(1)
}
