package main

import (
	"os"
	"path/filepath"
	"strings"
	"testing"
)

func TestPublishedChapterDestinations(t *testing.T) {
	docs, agents, out := fixture(t)
	write := func(file, body string) {
		t.Helper()
		if err := os.WriteFile(file, []byte(body), 0644); err != nil {
			t.Fatal(err)
		}
	}
	summary := filepath.Join(docs, "SUMMARY.md")
	write(summary, read(t, summary)+"  - [Directory](sub/README.md)\n")
	write(filepath.Join(docs, "sub", "README.md"), "# Directory\n\nOther chapter: [child](child.md?mode=a#intro).\n")
	write(filepath.Join(out, "sub", "index.html"), "<html><head></head></html>")
	page := filepath.Join(out, "sub", "child.html")
	write(page, "<html><head></head><body><a href=\"README.html#x\">Directory</a></body></html>")
	sample := "`[inline](README.md)`\n\n~~~text\n[example](README.md)\n~~~\n"
	write(filepath.Join(docs, "sub", "child.md"), "# Child\n\n[Directory](./README.md?x=1#x) [local](#part) [site](https://example.org/README.md)\n\n[r]: <../guide.md#top>\n\n- outer\n  - inner\n    [nested](README.md)\n\n"+sample)
	cfg := config{docsDir: docs, agentsDir: agents, outDir: out}
	if err := runEmit(cfg); err != nil {
		t.Fatal(err)
	}
	check := func(name string, expected ...string) {
		t.Helper()
		data := read(t, filepath.Join(out, name))
		for _, part := range expected {
			if !strings.Contains(data, part) {
				t.Errorf("%s missing %q", name, part)
			}
		}
	}
	check("sub/child.html", `href="index.html#x"`)
	check("sub/child.html.md", "[Directory](index.html.md?x=1#x)", "[local](#part)", "[r]: <../guide.html.md#top>", "[nested](index.html.md)", sample)
	check("llms-full.txt", "[Directory](sub/index.html.md?x=1#x)", "[local](sub/child.html.md#part)", "[child](sub/child.html.md?mode=a#intro)", "[r]: <guide.html.md#top>", "[site](https://example.org/README.md)", sample)
	if err := runCheck(cfg); err != nil {
		t.Fatal(err)
	}
	write(page, strings.Replace(read(t, page), `href="index.html#x"`, `href="README.html#x"`, 1))
	if err := runCheck(cfg); err == nil || !strings.Contains(err.Error(), "unpublished README.html") {
		t.Fatalf("expected broken HTML link diagnostic, got %v", err)
	}
}
