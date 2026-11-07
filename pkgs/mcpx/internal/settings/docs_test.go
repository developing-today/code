package settings_test

import (
	"os"
	"strings"
	"testing"

	"github.com/developing-today/code/pkgs/mcpx/internal/settings"
)

// update is declared in scriptenv_test.go, which regenerates docs/environment.md
// the same way.

const configurationPath = "../../docs/configuration.md"

// The markers separate the prose, which is written, from the inventory, which
// is not. Spelled the way scriptenv_test.go spells them, so the two generated
// documents look the same to a reader who opens both.
const (
	tableBegin = "<!-- BEGIN GENERATED: settings.Registry(); `go test ./internal/settings -run TestTheDocumentationListsEverySetting -update` -->"
	tableEnd   = "<!-- END GENERATED -->"
)

// TestTheDocumentationListsEverySetting keeps docs/configuration.md honest.
//
// A settings inventory written by hand is wrong within two releases, and
// wrong documentation about configuration is worse than none: it sends people
// looking for a knob that does not exist and lets them miss the one that
// does.
//
// The document said it was generated and it was not. Nothing produced it, and
// what did exist -- a Contains() check for the path, the variable and the
// primary flag somewhere in the file -- passed while the table had drifted:
// `exec.output` was missing its `--output` alias and `search.limit` was
// missing `--n`, so two real flags were invisible to anyone reading the
// inventory rather than the source. A Contains() check also cannot see a
// wrong kind, a wrong default, a wrong scope, a row in the wrong section, or
// a row for a setting that no longer exists.
//
// So it is generated now, the way docs/parity.md is: this test regenerates
// the section below the marker and fails when it differs.
func TestTheDocumentationListsEverySetting(t *testing.T) {
	b, err := os.ReadFile(configurationPath)
	if err != nil {
		t.Fatalf("the configuration inventory should exist: %v", err)
	}
	doc := string(b)
	i := strings.Index(doc, tableBegin)
	j := strings.Index(doc, tableEnd)
	if i < 0 || j < i {
		t.Fatalf("docs/configuration.md has lost the markers that separate its "+
			"prose from its generated inventory (%s ... %s)", tableBegin, tableEnd)
	}
	s, err := settings.New(settings.Registry())
	if err != nil {
		t.Fatal(err)
	}
	want := doc[:i+len(tableBegin)] + "\n\n" + settings.ConfigurationTable(s.All()) + doc[j:]
	if *update {
		if err := os.WriteFile(configurationPath, []byte(want), 0o644); err != nil {
			t.Fatal(err)
		}
		return
	}
	if doc != want {
		t.Errorf("docs/configuration.md no longer matches settings.Registry(); regenerate it with\n" +
			"  go test ./internal/settings -run TestTheDocumentationListsEverySetting -update")
	}
}

// TestTheDocumentationDescribesTheScopes fails if a scope is added without
// saying what it means, which would make the column in the table unreadable.
// The scopes are explained in the prose above the markers, which is written by
// hand, so nothing else would notice.
func TestTheDocumentationDescribesTheScopes(t *testing.T) {
	b, err := os.ReadFile(configurationPath)
	if err != nil {
		t.Fatal(err)
	}
	for _, sc := range []settings.Scope{
		settings.ScopeDaemon, settings.ScopeClient,
		settings.ScopeCall, settings.ScopePlugin,
	} {
		if !strings.Contains(string(b), "**"+sc.String()+"**") {
			t.Errorf("the %s scope is not explained in docs/configuration.md", sc)
		}
	}
}
