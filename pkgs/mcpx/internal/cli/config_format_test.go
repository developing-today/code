package cli

import (
	"context"
	"strings"
	"testing"

	"github.com/dezren39/mcpx/internal/settings"
)

// `mcpx config --schema --format` (#1846 step 2): markdown emits
// exactly Schema.Markdown's bytes, json matches Schema.JSON, text is
// the Describe default, and an unknown format is an error rather
// than a silent fallthrough.
func TestConfigSchemaFormats(t *testing.T) {
	sch, err := settings.New(settings.Registry())
	if err != nil {
		t.Fatal(err)
	}

	out, err := captureStdout(t, func() error {
		return (&App{}).CmdConfig(context.Background(), []string{"--schema", "--format", "markdown"})
	})
	if err != nil {
		t.Fatal(err)
	}
	if want := sch.Markdown(settings.MarkdownOptions{}); out != want {
		t.Errorf("--format markdown output is not Schema.Markdown's bytes\n got: %.200q\nwant: %.200q", out, want)
	}

	out, err = captureStdout(t, func() error {
		return (&App{}).CmdConfig(context.Background(), []string{"--schema", "--format", "json"})
	})
	if err != nil {
		t.Fatal(err)
	}
	if want := sch.JSON(false) + "\n"; out != want {
		t.Errorf("--format json output differs from Schema.JSON\n got: %.200q\nwant: %.200q", out, want)
	}

	out, err = captureStdout(t, func() error {
		return (&App{}).CmdConfig(context.Background(), []string{"--schema"})
	})
	if err != nil {
		t.Fatal(err)
	}
	if !strings.HasPrefix(out, sch.Describe(false)) {
		t.Error("default --schema output no longer starts with Describe's text")
	}

	_, err = captureStdout(t, func() error {
		return (&App{}).CmdConfig(context.Background(), []string{"--schema", "--format", "yaml"})
	})
	if err == nil || !strings.Contains(err.Error(), "unknown --format") {
		t.Errorf("--format yaml: err = %v, want an unknown --format error", err)
	}
}
