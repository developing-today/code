package refdoc_test

import (
	"flag"
	"os"
	"strings"
	"testing"

	"github.com/developing-today/code/pkgs/mcpx/internal/cli"
	"github.com/developing-today/code/pkgs/mcpx/internal/settings"
)

// TestCommittedRegionsAreCurrent is the one shared golden check for
// every committed generated region (#4909, remainder of #1846 slice
// 5). Before it, each surface had its own ad-hoc test — the settings
// inventory had one, parity.md had one, protocol.md had none, which is
// how its §2.2 table went stale while claiming to describe the code.
//
// It covers:
//   - docs/configuration.md — the settings inventory region
//     (settings.ConfigurationTable), between its BEGIN/END GENERATED
//     markers;
//   - docs/parity.md — the whole file (cli.ParityDoc);
//   - docs/protocol.md — the protocol-features region in §1 and the
//     protocol-capabilities region in §2.2, between their named
//     BEGIN/END GENERATED markers.
//
// Regenerate everything with:
//
//	go test ./internal/refdoc -run TestCommittedRegionsAreCurrent -update
//
// The per-surface tests in internal/settings and internal/cli still
// exist and check the same bytes; this test is the single place a
// reader (or a Nix checkPhase) can confirm every region at once.
var update = flag.Bool("update", false, "rewrite generated regions instead of comparing them")

const docsDir = "../../docs/"

func readDoc(t *testing.T, name string) string {
	t.Helper()
	b, err := os.ReadFile(docsDir + name)
	if err != nil {
		t.Fatalf("read %s: %v", name, err)
	}
	return string(b)
}

func writeDoc(t *testing.T, name, body string) {
	t.Helper()
	if err := os.WriteFile(docsDir+name, []byte(body), 0o644); err != nil {
		t.Fatal(err)
	}
}

// replaceRegion swaps the body between the named markers, keeping the
// marker lines themselves and the prose around them.
func replaceRegion(t *testing.T, doc, name, body string) string {
	t.Helper()
	begin := "<!-- BEGIN GENERATED: " + name + " -->"
	end := "<!-- END GENERATED: " + name + " -->"
	i := strings.Index(doc, begin)
	j := strings.Index(doc, end)
	if i < 0 || j < i {
		t.Fatalf("markers for generated region %q are missing or out of order", name)
	}
	return doc[:i+len(begin)] + "\n\n" + strings.TrimRight(body, "\n") + "\n\n" + doc[j:]
}

func TestCommittedRegionsAreCurrent(t *testing.T) {
	// docs/configuration.md: the inventory between its (unnamed)
	// BEGIN/END GENERATED markers is settings.ConfigurationTable.
	{
		doc := readDoc(t, "configuration.md")
		const begin = "<!-- BEGIN GENERATED: settings.Registry(); `go test ./internal/settings -run TestTheDocumentationListsEverySetting -update` -->"
		const end = "<!-- END GENERATED -->"
		i := strings.Index(doc, begin)
		j := strings.Index(doc, end)
		if i < 0 || j < i {
			t.Fatalf("docs/configuration.md has lost its generated-region markers")
		}
		s, err := settings.New(settings.Registry())
		if err != nil {
			t.Fatal(err)
		}
		want := doc[:i+len(begin)] + "\n\n" + settings.ConfigurationTable(s.All()) + doc[j:]
		if *update {
			writeDoc(t, "configuration.md", want)
		} else if doc != want {
			t.Errorf("docs/configuration.md region is stale; regenerate with\n" +
				"  go test ./internal/refdoc -run TestCommittedRegionsAreCurrent -update")
		}
	}

	// docs/parity.md: the whole file is cli.ParityDoc over the plugin's
	// daemon.ts.
	{
		ts, err := os.ReadFile("../../plugin/core/daemon.ts")
		if err != nil {
			t.Fatal(err)
		}
		want := cli.ParityDoc(string(ts))
		doc := readDoc(t, "parity.md")
		if *update {
			writeDoc(t, "parity.md", want)
		} else if doc != want {
			t.Errorf("docs/parity.md is stale; regenerate with\n" +
				"  go test ./internal/refdoc -run TestCommittedRegionsAreCurrent -update")
		}
	}

	// docs/protocol.md: features in §1, declared capabilities in §2.2.
	{
		doc := readDoc(t, "protocol.md")
		want := replaceRegion(t, doc, "protocol-features", cli.ProtocolFeaturesTable())
		want = replaceRegion(t, want, "protocol-capabilities", cli.ProtocolCapabilitiesTable())
		if *update {
			writeDoc(t, "protocol.md", want)
		} else if doc != want {
			t.Errorf("docs/protocol.md generated regions are stale; regenerate with\n" +
				"  go test ./internal/refdoc -run TestCommittedRegionsAreCurrent -update")
		}
	}
}
