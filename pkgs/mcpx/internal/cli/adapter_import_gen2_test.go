package cli

import (
	"os"
	"path/filepath"
	"strings"
	"testing"
)

// Regression tests for the PR #9161 review findings. The oracles are
// the findings' own scenarios: a mutating binary import must never be
// probed with operand-shaped arguments, an advertised completion
// subcommand is the probe gate, subcommand tools must be able to take
// the positionals their operation needs, equals-bound values must stay
// attached, fish conditions bind flags to every listed spelling, and a
// relative binary path is stored absolute.

func TestFishConditionAppliesToEverySpelling(t *testing.T) {
	text := "complete -c toolbox -n '__fish_seen_subcommand_from add append' -l force -d 'overwrite'\n"
	shape := parseFishCompletion(text, "toolbox")
	if len(shape.Subs) != 2 {
		t.Fatalf("subs: %+v", shape.Subs)
	}
	for _, sub := range shape.Subs {
		if len(sub.Flags) != 1 || sub.Flags[0].Long != "force" {
			t.Errorf("sub %s flags: %+v", sub.Name, sub.Flags)
		}
	}
}

func TestBracketedValueBindsEqualsOnly(t *testing.T) {
	flags := parseFlagSpec("--exec-path[=<path>]")
	if len(flags) != 1 {
		t.Fatalf("flags: %+v", flags)
	}
	f := flags[0]
	if f.Long != "exec-path" || !f.TakesValue || !f.EqualsOnly || f.Value != "path" {
		t.Errorf("flag: %+v", f)
	}
	params := flagParams(flags)
	if len(params) != 1 || !params[0].Equals {
		t.Errorf("param should carry the equals binding: %+v", params)
	}
	plain := parseFlagSpec("--config=<file>")
	if len(plain) != 1 || plain[0].EqualsOnly {
		t.Errorf("the unbracketed equals form accepts a separated value too: %+v", plain)
	}
}

// fakeTool writes an executable script answering from a case on $1 and
// logging every invocation to the file named by $CALLS_LOG.
func fakeTool(t *testing.T, dir, name, body string) string {
	t.Helper()
	path := filepath.Join(dir, name)
	if err := os.WriteFile(path, []byte(body), 0o755); err != nil {
		t.Fatal(err)
	}
	return path
}

const mutatingToolScript = `#!/bin/sh
echo "$@" >> "$CALLS_LOG"
case "$1" in
  --help)
    echo 'Usage: muttool [OPTIONS] FILE...'
    echo
    echo 'Options:'
    echo '  -f, --force   do it anyway'
    ;;
  --version) echo 'muttool version 3.1.4' ;;
esac
exit 0
`

func TestAutoImportNeverProbesAnUnadvertisedBinary(t *testing.T) {
	dir := t.TempDir()
	log := filepath.Join(dir, "calls.log")
	t.Setenv("CALLS_LOG", log)
	bin := fakeTool(t, dir, "muttool", mutatingToolScript)
	spec, _, err := buildGeneratedImport(bin, importOptions{})
	if err != nil {
		t.Fatal(err)
	}
	if spec.Provenance.Source != "help" {
		t.Errorf("source should fall through to help, got %q", spec.Provenance.Source)
	}
	calls, err := os.ReadFile(log)
	if err != nil {
		t.Fatal(err)
	}
	for _, line := range strings.Split(string(calls), "\n") {
		if strings.HasPrefix(line, "completion") {
			t.Errorf("the binary was probed with operand-shaped arguments: %q", line)
		}
	}
}

const cobraToolScript = `#!/bin/sh
case "$1" in
  --help)
    echo 'Usage:'
    echo '  cobratool [command]'
    echo
    echo 'Available Commands:'
    echo '  completion  Generate completion script'
    echo '  serve       Serve things'
    echo
    echo 'Flags:'
    echo '  -h, --help   help for cobratool'
    ;;
  completion)
    if [ "$2" = "fish" ]; then
      echo "complete -c cobratool -n '__fish_use_subcommand' -a 'serve' -d 'subcommands'"
      echo "complete -c cobratool -n '__fish_seen_subcommand_from serve' -l port -r -d 'port'"
    fi
    ;;
  --version) echo 'cobratool version 0.9.1' ;;
esac
exit 0
`

func TestAutoImportProbesAnAdvertisedCompletion(t *testing.T) {
	dir := t.TempDir()
	bin := fakeTool(t, dir, "cobratool", cobraToolScript)
	spec, _, err := buildGeneratedImport(bin, importOptions{})
	if err != nil {
		t.Fatal(err)
	}
	if spec.Provenance.Source != "completion" {
		t.Fatalf("source: %q (detail %q)", spec.Provenance.Source, spec.Provenance.SourceDetail)
	}
	if len(spec.Tools) != 1 || spec.Tools[0].Name != "serve" {
		t.Fatalf("tools: %+v", spec.Tools)
	}
}

const positionalToolScript = `#!/bin/sh
case "$1 $2" in
  "--help ")
    echo 'Usage:'
    echo '  toolbox [command]'
    echo
    echo 'Available Commands:'
    echo '  add   Add an item'
    ;;
  "add --help")
    echo 'Usage:'
    echo '  toolbox add [flags]'
    echo
    echo 'Positional Arguments:'
    echo '  item   the item to add'
    echo
    echo 'Flags:'
    echo '  -f, --force   overwrite existing'
    ;;
  "--version ") echo 'toolbox version 1.0.0' ;;
esac
exit 0
`

func TestHelpImportPreservesSubcommandPositionals(t *testing.T) {
	dir := t.TempDir()
	bin := fakeTool(t, dir, "toolbox", positionalToolScript)
	spec, _, err := buildGeneratedImport(bin, importOptions{source: "help"})
	if err != nil {
		t.Fatal(err)
	}
	if len(spec.Tools) != 1 || spec.Tools[0].Name != "add" {
		t.Fatalf("tools: %+v", spec.Tools)
	}
	byName := map[string]bool{}
	var last string
	for _, p := range spec.Tools[0].Params {
		byName[p.Name] = true
		last = p.Name
		if p.Name == "item" && !p.Required {
			t.Errorf("the parsed positional should be required: %+v", p)
		}
	}
	if !byName["force"] || !byName["item"] {
		t.Errorf("params: %+v", spec.Tools[0].Params)
	}
	if last != "args" {
		t.Errorf("the positional catch-all should ride last, got %q", last)
	}
}

func TestRelativeBinaryPathIsStoredAbsolute(t *testing.T) {
	dir := t.TempDir()
	if err := os.MkdirAll(filepath.Join(dir, "bin"), 0o755); err != nil {
		t.Fatal(err)
	}
	fakeTool(t, dir, "bin/tool", positionalToolScript)
	t.Chdir(dir)
	spec, _, err := buildGeneratedImport("./bin/tool", importOptions{source: "help"})
	if err != nil {
		t.Fatal(err)
	}
	want := filepath.Join(dir, "bin", "tool")
	if spec.Command != want {
		t.Errorf("command: got %q, want absolute %q", spec.Command, want)
	}
}
