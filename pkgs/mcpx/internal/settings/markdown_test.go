package settings_test

import (
	"strings"
	"testing"

	"github.com/dezren39/mcpx/internal/settings"
)

func mustSchema(t *testing.T) *settings.Schema {
	t.Helper()
	s, err := settings.New(settings.Registry())
	if err != nil {
		t.Fatal(err)
	}
	return s
}

// The generated settings reference (#1846 step 2): one region per
// top-level key, today's columns, *(empty)* for an empty default.
func TestMarkdownRendersRegionsAndColumns(t *testing.T) {
	md := mustSchema(t).Markdown(settings.MarkdownOptions{})
	for _, want := range []string{
		"<!-- ANCHOR: daemon -->",
		"### daemon\n",
		"<!-- ANCHOR_END: daemon -->",
		"| setting | kind | default | scope | hot | flag | variable | governs |",
		"| `daemon.endpoint` | string | *(empty)* | client |",
		"`--daemon-endpoint`",
		"`MCPX_DAEMON_ENDPOINT`",
	} {
		if !strings.Contains(md, want) {
			t.Errorf("Markdown output missing %q", want)
		}
	}
}

func TestMarkdownPlumbingOnlyOnRequest(t *testing.T) {
	s := mustSchema(t)
	var plumbingPath string
	for _, set := range s.All() {
		if set.Plumbing {
			plumbingPath = set.Path
			break
		}
	}
	if plumbingPath == "" {
		t.Fatal("registry has no plumbing setting to test with")
	}
	if strings.Contains(s.Markdown(settings.MarkdownOptions{}), "`"+plumbingPath+"`") {
		t.Errorf("plumbing setting %s rendered without the Plumbing option", plumbingPath)
	}
	withPlumbing := s.Markdown(settings.MarkdownOptions{Plumbing: true})
	if !strings.Contains(withPlumbing, "`"+plumbingPath+"`") || !strings.Contains(withPlumbing, "*(plumbing)*") {
		t.Errorf("plumbing setting %s missing or unmarked with the Plumbing option", plumbingPath)
	}
}

func TestMarkdownLongAddsDefinitionList(t *testing.T) {
	s := mustSchema(t)
	var withLong settings.Setting
	for _, set := range s.All() {
		if set.Long != "" {
			withLong = set
			break
		}
	}
	if withLong.Path == "" {
		t.Fatal("registry has no setting with a Long paragraph to test with")
	}
	long := s.Markdown(settings.MarkdownOptions{Long: true})
	if !strings.Contains(long, "`"+withLong.Path+"`\n:   ") {
		t.Errorf("Long option produced no definition list entry for %s", withLong.Path)
	}
	if !strings.Contains(long, withLong.Long) {
		t.Errorf("Long paragraph for %s not rendered", withLong.Path)
	}
}

// Emitters are deterministic: the schema alone decides the bytes —
// resolved values and the environment must never leak into them.
func TestMarkdownIsDeterministicAndIgnoresEnvironment(t *testing.T) {
	s := mustSchema(t)
	first := s.Markdown(settings.MarkdownOptions{Plumbing: true, Long: true})
	t.Setenv("MCPX_DAEMON_ENDPOINT", "http://example.invalid:9999")
	t.Setenv("MCPX_LOGGING_LEVEL", "debug")
	second := s.Markdown(settings.MarkdownOptions{Plumbing: true, Long: true})
	if first != second {
		t.Error("Markdown bytes changed when MCPX_* environment changed")
	}
	if strings.Contains(second, "example.invalid") {
		t.Error("a resolved environment value leaked into the generated reference")
	}
}

// Schema.JSON carries scope and hot (#1846 step 2), so generators
// reading the JSON form see the same facts as the table.
func TestJSONIncludesScopeAndHot(t *testing.T) {
	js := mustSchema(t).JSON(false)
	if !strings.Contains(js, `"scope":`) || !strings.Contains(js, `"hot":`) {
		t.Error("Schema.JSON is missing the scope/hot fields")
	}
}
