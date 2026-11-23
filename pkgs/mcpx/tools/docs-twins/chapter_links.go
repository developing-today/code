package main

import (
	"net/url"
	"path"
	"path/filepath"
	"regexp"
	"strings"
)

// Keep the author's Markdown and rewrite only destinations that identify a
// chapter. Directory README chapters have different source and output names.
type chapterLinks map[string]chapter

func newChapterLinks(chapters []chapter) chapterLinks {
	links := make(chapterLinks, len(chapters)*3)
	for _, ch := range chapters {
		links[ch.Source] = ch
		links[ch.twinRel()] = ch
		links[strings.TrimSuffix(ch.Source, ".md")+".html"] = ch
		links[ch.htmlRel()] = ch
	}
	return links
}

func (links chapterLinks) destination(raw, origin, output string, html bool) string {
	if raw == "" || strings.HasPrefix(raw, "/") {
		return raw
	}
	u, err := url.Parse(raw)
	if err != nil || u.IsAbs() || u.Host != "" {
		return raw
	}
	target := origin
	if u.Path != "" {
		target = path.Join(path.Dir(origin), u.Path)
	}
	ch, ok := links[target]
	if !ok {
		return raw
	}
	dest := ch.twinRel()
	if html {
		dest = ch.htmlRel()
	}
	if u.Path == "" && output == dest {
		return raw
	}
	rel, err := filepath.Rel(path.Dir(output), dest)
	if err != nil {
		return raw
	}
	// Preserve the author's byte spelling of the query and fragment.
	suffix := ""
	if cut := strings.IndexAny(raw, "?#"); cut >= 0 {
		suffix = raw[cut:]
	}
	return (&url.URL{Path: filepath.ToSlash(rel)}).EscapedPath() + suffix
}

// Inline and reference-style chapter destinations supported by this book.
// Opaque code spans and fenced/indented examples are copied verbatim.
var mdDestination = regexp.MustCompile(`(\]\([ \t]*|^ {0,3}\[[^\]]+\]:[ \t]*)(<[^>\n]+>|[^\s)<>]+)`)
var codeRun = regexp.MustCompile("`+")
var fenceStart = regexp.MustCompile("^ {0,3}(`{3,}|~{3,})")
var anchorHref = regexp.MustCompile(`(?i)(<a\s+[^>]*?\bhref=")([^"]*)(")`)

func (links chapterLinks) markdown(content, origin, output string) string {
	var out strings.Builder
	var fence, span string
	for line := range strings.SplitAfterSeq(content, "\n") {
		if marker := fenceStart.FindStringSubmatch(line); marker != nil && span == "" {
			if fence == "" {
				fence = marker[1]
				out.WriteString(line)
				continue
			}
			if marker[1][0] == fence[0] && len(marker[1]) >= len(fence) && strings.TrimSpace(line[len(marker[0]):]) == "" {
				fence = ""
				out.WriteString(line)
				continue
			}
		}
		if fence != "" || strings.HasPrefix(line, "    ") || strings.HasPrefix(line, "\t") {
			out.WriteString(line)
			continue
		}
		if strings.TrimSpace(line) == "" {
			span = ""
		}
		rewrite := func(s string) string {
			return mdDestination.ReplaceAllStringFunc(s, func(match string) string {
				m := mdDestination.FindStringSubmatch(match)
				raw := m[2]
				if strings.HasPrefix(raw, "<") {
					return m[1] + "<" + links.destination(raw[1:len(raw)-1], origin, output, false) + ">"
				}
				return m[1] + links.destination(raw, origin, output, false)
			})
		}
		offset := 0
		for _, run := range codeRun.FindAllStringIndex(line, -1) {
			piece := line[offset:run[0]]
			if span == "" {
				out.WriteString(rewrite(piece))
			} else {
				out.WriteString(piece)
			}
			ticks := line[run[0]:run[1]]
			if span == ticks {
				span = ""
			} else if span == "" {
				span = ticks
			}
			out.WriteString(ticks)
			offset = run[1]
		}
		if span == "" {
			out.WriteString(rewrite(line[offset:]))
		} else {
			out.WriteString(line[offset:])
		}
	}
	return out.String()
}

func (links chapterLinks) html(content string, ch chapter) string {
	return anchorHref.ReplaceAllStringFunc(content, func(match string) string {
		m := anchorHref.FindStringSubmatch(match)
		// mdBook renders source README.md references as README.html but
		// publishes the corresponding chapter at index.html.
		if !strings.Contains(m[2], "README.html") {
			return match
		}
		return m[1] + links.destination(m[2], ch.Source, ch.htmlRel(), true) + m[3]
	})
}
