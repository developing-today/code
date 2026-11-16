package cli

import (
	"context"
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

// Oracle: issue #9557, the Codex scenario from PR #9161. Integer
// typing comes only from complete numeric placeholder tokens;
// ENDPOINT (which merely contains "int") must generate a string
// parameter so argument validation accepts a URL value.
func TestFlagParamsIntegerOnlyFromCompleteNumericTokens(t *testing.T) {
	cases := []struct{ placeholder, want string }{
		{"ENDPOINT", "string"},
		{"POINT", "string"},
		{"FILENAME", "string"},
		{"INT", "integer"},
		{"INTEGER", "integer"},
		{"NUMBER", "integer"},
		{"NUM", "integer"},
		{"COUNT", "integer"},
		{"MAX_COUNT", "integer"},
		{"RETRY-NUM", "integer"},
	}
	for _, c := range cases {
		params := flagParams([]importFlag{{Long: "opt", TakesValue: true, Value: c.placeholder}})
		if len(params) != 1 || params[0].Type != c.want {
			t.Errorf("placeholder %s: got %+v, want type %s", c.placeholder, params, c.want)
		}
	}
}

func TestParseHelpTextOptionalPositionalFromUsage(t *testing.T) {
	text := "usage: demo [-h] src [item]\n\npositional arguments:\n  src   the source\n  item  the item\n\noptions:\n  -h, --help  show help\n"
	shape := parseHelpText(text)
	if len(shape.Positionals) != 2 {
		t.Fatalf("positionals: %+v", shape.Positionals)
	}
	byName := map[string]importFlag{}
	for _, f := range shape.Positionals {
		byName[f.Long] = f
	}
	if byName["src"].Optional {
		t.Errorf("src is unbracketed in the usage and must stay required: %+v", byName["src"])
	}
	if !byName["item"].Optional {
		t.Errorf("item is bracketed [item] in the usage and must be optional: %+v", byName["item"])
	}
}

const optionalPositionalToolScript = `#!/bin/sh
case "$1" in
  "--help")
    echo 'usage: optool [-h] [item]'
    echo
    echo 'positional arguments:'
    echo '  item   the item to use'
    echo
    echo 'options:'
    echo '  -h, --help   show this help message and exit'
    ;;
  "--version") echo 'optool version 1.0.0' ;;
  *) echo "ran:$*" ;;
esac
exit 0
`

// The acceptance oracle for #9559: an imported optional positional is
// not required, and a call omitting it validates and runs with no
// operand rendered.
func TestHelpImportOptionalPositionalMayBeOmitted(t *testing.T) {
	dir := t.TempDir()
	bin := fakeTool(t, dir, "optool", optionalPositionalToolScript)
	spec, _, err := buildGeneratedImport(bin, importOptions{source: "help"})
	if err != nil {
		t.Fatal(err)
	}
	if len(spec.Tools) != 1 {
		t.Fatalf("tools: %+v", spec.Tools)
	}
	found := false
	for _, p := range spec.Tools[0].Params {
		if p.Name == "item" {
			found = true
			if p.Required {
				t.Errorf("the optional positional should not be required: %+v", p)
			}
		}
	}
	if !found {
		t.Fatalf("params: %+v", spec.Tools[0].Params)
	}
	res, err := spec.Call(context.Background(), spec.Tools[0].Name, map[string]any{})
	if err != nil {
		t.Fatalf("a call omitting the optional positional must validate: %v", err)
	}
	if !strings.Contains(res.Stdout, "ran:") || strings.Contains(res.Stdout, "item") {
		t.Errorf("the call should run with no operand rendered, got %q", res.Stdout)
	}
}

const optionalSubPositionalToolScript = `#!/bin/sh
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
    echo '  toolbox add [flags] [item]'
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

func TestHelpImportSubcommandOptionalPositional(t *testing.T) {
	dir := t.TempDir()
	bin := fakeTool(t, dir, "toolbox", optionalSubPositionalToolScript)
	spec, _, err := buildGeneratedImport(bin, importOptions{source: "help"})
	if err != nil {
		t.Fatal(err)
	}
	if len(spec.Tools) != 1 || spec.Tools[0].Name != "add" {
		t.Fatalf("tools: %+v", spec.Tools)
	}
	found := false
	for _, p := range spec.Tools[0].Params {
		if p.Name == "item" {
			found = true
			if p.Required {
				t.Errorf("the subcommand branch must preserve optionality too: %+v", p)
			}
		}
	}
	if !found {
		t.Errorf("params: %+v", spec.Tools[0].Params)
	}
}
