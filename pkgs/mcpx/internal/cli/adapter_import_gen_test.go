package cli

import (
	"encoding/json"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"github.com/developing-today/code/pkgs/mcpx/internal/adapter"
)

var genNow = time.Date(2026, 11, 12, 12, 0, 0, 0, time.UTC)

// Oracle for every parser test in this file: the fixture's own text.
// Every subcommand and flag the fixture lists appears in the
// generated declaration exactly once, with the fixture's
// description; nothing the fixture does not list appears.

const helpFixture = `faketool - does fake things for tests

Usage: faketool [options] <command> [args]

Commands:
  frobnicate    Frobnicate the widget
  list-things   List all the things

Options:
  -v, --verbose          Say more
  -o, --output <file>    Write to a file
  --level <n>            Set the level
`

func TestGenerateFromHelpParsesCommandsAndFlags(t *testing.T) {
	spec, err := buildGeneratedSpec("faketool", "faketool", "help", helpFixture,
		parseHelpText(helpFixture), "/usr/bin/faketool", "1.2.3", genNow)
	if err != nil {
		t.Fatal(err)
	}
	if spec.Provenance == nil || spec.Provenance.Source != "help" ||
		spec.Provenance.BinaryVersion != "1.2.3" || spec.Provenance.SourceSHA256 == "" ||
		spec.Provenance.GeneratedAt != "2026-11-12T12:00:00Z" {
		t.Fatalf("provenance should record source, version, hash and date: %+v", spec.Provenance)
	}
	if len(spec.Tools) != 2 || spec.Tools[0].Name != "frobnicate" || spec.Tools[1].Name != "list-things" {
		t.Fatalf("tools should be the fixture's commands in order: %+v", spec.Tools)
	}
	frob := spec.Tools[0]
	if frob.Description != "Frobnicate the widget" || len(frob.Args) != 1 || frob.Args[0] != "frobnicate" {
		t.Fatalf("the tool should carry its description and fixed arg: %+v", frob)
	}
	byName := map[string]adapter.Param{}
	for _, p := range frob.Params {
		byName[p.Name] = p
	}
	if byName["verbose"].Type != "boolean" || byName["verbose"].Flag != "--verbose" {
		t.Errorf("--verbose is a boolean flag: %+v", byName["verbose"])
	}
	if byName["output"].Flag != "--output" || byName["output"].Type == "boolean" ||
		byName["output"].Description != "Write to a file" {
		t.Errorf("--output takes a file value: %+v", byName["output"])
	}
	if _, ok := byName["args"]; !ok {
		t.Errorf("every generated tool carries the args catch-all: %+v", frob.Params)
	}
	// The emission round-trips through the load path unchanged.
	doc, err := json.Marshal(AdapterFile{Adapters: []adapter.Spec{spec}})
	if err != nil {
		t.Fatal(err)
	}
	path := filepath.Join(t.TempDir(), "adapters.json")
	if err := os.WriteFile(path, doc, 0o644); err != nil {
		t.Fatal(err)
	}
	loaded, err := loadAdapterFiles([]string{path})
	if err != nil {
		t.Fatalf("a generated declaration should load: %v", err)
	}
	if loaded[0].Provenance == nil || loaded[0].Provenance.BinaryVersion != "1.2.3" {
		t.Fatalf("provenance should survive the load round-trip: %+v", loaded[0].Provenance)
	}
}

func TestParseHelpSkipsWrappedUsageLines(t *testing.T) {
	// git's --help wraps its usage over two lines; the continuation
	// is syntax, and must not become the generated description.
	text := "usage: git [-v | --version] [-h | --help] [-C <path>]\n" +
		"           [--exec-path[=<path>]] [--html-path] [--man-path]\n" +
		"\nThese are common Git commands used in various situations:\n\n" +
		"   add        Add file contents to the index\n" +
		"   status     Show the working tree status\n"
	parsed := parseHelpText(text)
	if parsed.description != "" {
		t.Errorf("a wrapped usage line is not a description: %q", parsed.description)
	}
	if len(parsed.tools) != 2 || parsed.tools[0].name != "add" || parsed.tools[1].name != "status" {
		t.Errorf("the commands after the usage block still parse: %+v", parsed.tools)
	}
}

func TestGenerateIsDeterministic(t *testing.T) {
	// Oracle: the same source parsed twice yields byte-identical
	// declarations (same fixed clock), because generation is a
	// parser, not a model.
	a, err := buildGeneratedSpec("faketool", "faketool", "help", helpFixture,
		parseHelpText(helpFixture), "", "", genNow)
	if err != nil {
		t.Fatal(err)
	}
	b, err := buildGeneratedSpec("faketool", "faketool", "help", helpFixture,
		parseHelpText(helpFixture), "", "", genNow)
	if err != nil {
		t.Fatal(err)
	}
	aj, _ := json.Marshal(a)
	bj, _ := json.Marshal(b)
	if string(aj) != string(bj) {
		t.Fatalf("generation must be deterministic:\n%s\n%s", aj, bj)
	}
}

func TestGenerateRefusesUnparseableSource(t *testing.T) {
	// A source with no commands and no flags must be refused, not
	// emitted as an empty declaration that looks complete.
	_, err := buildGeneratedSpec("faketool", "faketool", "help",
		"faketool does things.\nUsage: faketool\n",
		parseHelpText("faketool does things.\nUsage: faketool\n"), "", "", genNow)
	if err == nil {
		t.Fatal("an unparseable source should be refused")
	}
	for _, want := range []string{"no subcommands or flags", "--bare"} {
		if !strings.Contains(err.Error(), want) {
			t.Errorf("the refusal should name %q: %v", want, err)
		}
	}
}

const fishFixture = `# faketool completions
complete -c faketool -f -a "frobnicate list-things" -n "__fish_use_subcommand"
complete -c faketool -s v -l verbose -d 'Say more'
complete -c faketool -s o -l output -r -d 'Write to a file'
complete -c faketool -n "__fish_seen_subcommand_from frobnicate" -s q -l quiet -d 'Keep it down'
`

func TestGenerateFromFishCompletion(t *testing.T) {
	spec, err := buildGeneratedSpec("faketool", "faketool", "completion", fishFixture,
		parseCompletionText("fish", fishFixture), "", "", genNow)
	if err != nil {
		t.Fatal(err)
	}
	if len(spec.Tools) != 2 {
		t.Fatalf("the -a word list names two subcommands: %+v", spec.Tools)
	}
	byName := map[string]adapter.Param{}
	for _, p := range spec.Tools[0].Params {
		byName[p.Name] = p
	}
	if byName["quiet"].Flag != "--quiet" {
		t.Errorf("a seen-subcommand line's flag belongs to that tool: %+v", spec.Tools[0].Params)
	}
	if byName["verbose"].Type != "boolean" {
		t.Errorf("-v/--verbose without -r is boolean: %+v", byName["verbose"])
	}
	if byName["output"].Type == "boolean" {
		t.Errorf("-r marks --output value-taking: %+v", byName["output"])
	}
	if byName["output"].Description != "Write to a file" {
		t.Errorf("the -d description survives quoting: %+v", byName["output"])
	}
}

const zshFixture = `#compdef faketool
_arguments \
  '--verbose[Say more]' \
  '--output=[Write to a file]:file:_files' \
  '1:command:_commands'
local -a commands
commands=(
  "frobnicate:Frobnicate the widget"
  "list-things:List all the things"
)
`

func TestGenerateFromZshCompletion(t *testing.T) {
	spec, err := buildGeneratedSpec("faketool", "faketool", "completion", zshFixture,
		parseCompletionText("zsh", zshFixture), "", "", genNow)
	if err != nil {
		t.Fatal(err)
	}
	if len(spec.Tools) != 2 || spec.Tools[0].Description != "Frobnicate the widget" {
		t.Fatalf("commands-array pairs name the tools: %+v", spec.Tools)
	}
	byName := map[string]adapter.Param{}
	for _, p := range spec.Tools[0].Params {
		byName[p.Name] = p
	}
	if byName["verbose"].Type != "boolean" || byName["verbose"].Description != "Say more" {
		t.Errorf("'--verbose[Say more]' is a described boolean: %+v", byName["verbose"])
	}
	if byName["output"].Type == "boolean" {
		t.Errorf("'--output=[...]:...' takes a value: %+v", byName["output"])
	}
}

const bashFixture = `# faketool bash completion
_faketool() {
  local cur opts
  opts="--verbose --output"
  if [[ $COMP_CWORD -eq 1 ]]; then
    COMPREPLY=( $(compgen -W "frobnicate list-things" -- "$cur") )
  fi
}
complete -F _faketool faketool
`

func TestGenerateFromBashCompletion(t *testing.T) {
	spec, err := buildGeneratedSpec("faketool", "faketool", "completion", bashFixture,
		parseCompletionText("bash", bashFixture), "", "", genNow)
	if err != nil {
		t.Fatal(err)
	}
	if len(spec.Tools) != 2 {
		t.Fatalf("the -W word list names the subcommands: %+v", spec.Tools)
	}
	found := map[string]bool{}
	for _, p := range spec.Tools[0].Params {
		found[p.Name] = true
	}
	if !found["verbose"] || !found["output"] {
		t.Errorf("opts word-list flags become params: %+v", spec.Tools[0].Params)
	}
}

const manFixture = `FAKETOOL(1)                 User Commands                FAKETOOL(1)

NAME
       faketool - does fake things for tests

SYNOPSIS
       faketool [options] <command>

OPTIONS
       -v, --verbose
              Say more about what is happening.

       -o <file>, --output <file>
              Write to a file instead of stdout.

COMMANDS
       frobnicate
              Frobnicate the widget.
`

func TestGenerateFromManPage(t *testing.T) {
	parsed := parseManText(manFixture)
	spec, err := buildGeneratedSpec("faketool", "faketool", "man", manFixture,
		parsed, "", "", genNow)
	if err != nil {
		t.Fatal(err)
	}
	if spec.Description != "does fake things for tests" {
		t.Errorf("the NAME line is the description: %q", spec.Description)
	}
	if len(spec.Tools) != 1 || spec.Tools[0].Name != "frobnicate" {
		t.Fatalf("the COMMANDS section names the tools: %+v", spec.Tools)
	}
	byName := map[string]adapter.Param{}
	for _, p := range spec.Tools[0].Params {
		byName[p.Name] = p
	}
	if !strings.Contains(byName["verbose"].Description, "Say more") {
		t.Errorf("a man flag's description continues on the next line: %+v", byName["verbose"])
	}
	if byName["output"].Flag != "--output" || byName["output"].Type == "boolean" {
		t.Errorf("'-o <file>, --output <file>' takes a value: %+v", byName["output"])
	}
}

func TestParseFlagLinePlaceholders(t *testing.T) {
	// Regression oracle (found generating for git's man page): a
	// value placeholder is part of the flag spec even when nothing
	// follows it on the line -- it must not leak into the
	// description or leave the flag boolean.
	f, ok := parseFlagLine("     -C <path>")
	if !ok || f.short != "-C" || f.arg != "<path>" || f.description != "" {
		t.Errorf("-C <path>: %+v ok=%v", f, ok)
	}
	f, ok = parseFlagLine("       -o <file>, --output <file>")
	if !ok || f.long != "--output" || f.arg != "<file>" || f.description != "" {
		t.Errorf("-o/--output: %+v ok=%v", f, ok)
	}
	f, ok = parseFlagLine("  --level <n>            Set the level")
	if !ok || f.long != "--level" || f.arg != "<n>" || f.description != "Set the level" {
		t.Errorf("--level: %+v ok=%v", f, ok)
	}
	if _, ok := parseFlagLine("  a line of prose - not a flag"); ok {
		t.Error("prose is not a flag line")
	}
}

func TestSplitImportArgsGenerationFlags(t *testing.T) {
	opts, err := splitImportArgsFull([]string{"--source", "help", "git", "--out=f.json"})
	if err != nil {
		t.Fatal(err)
	}
	if opts.source != "help" || opts.out != "f.json" || len(opts.rest) != 1 || opts.rest[0] != "git" {
		t.Errorf("got %+v", opts)
	}
	opts, err = splitImportArgsFull([]string{"--completion", "git.fish", "--shell", "fish"})
	if err != nil || opts.completion != "git.fish" || opts.shell != "fish" {
		t.Errorf("got %+v, %v", opts, err)
	}
	if _, err := splitImportArgsFull([]string{"--source", "astrology", "git"}); err == nil {
		t.Error("an unknown --source should be refused")
	}
	// The bare-era surface still parses through both entry points.
	bare, desc, out, rest, err := splitImportArgs([]string{"--bare", "git", "--description", "vcs"})
	if err != nil || !bare || desc != "vcs" || out != "" || len(rest) != 1 {
		t.Errorf("bare parsing regressed: %v %q %q %q", bare, desc, out, rest)
	}
}

func TestInferShellAndCompletionName(t *testing.T) {
	if got := inferShell("/x/_git", ""); got != "zsh" {
		t.Errorf("_git is a zsh completion: %q", got)
	}
	if got := inferShell("/x/git.fish", ""); got != "fish" {
		t.Errorf("git.fish is fish: %q", got)
	}
	if got := inferShell("/x/git", "complete -c git -f"); got != "fish" {
		t.Errorf("content can decide too: %q", got)
	}
	if got := completionName("/usr/share/zsh/site-functions/_git"); got != "git" {
		t.Errorf("got %q", got)
	}
	if got := completionName("/x/git.fish"); got != "git" {
		t.Errorf("got %q", got)
	}
}

// fakeTool writes an executable shell script standing in for a real
// binary, so generation and drift are tested against the behaviour
// the script actually has --version and --help included.
func fakeTool(t *testing.T, dir, version string) string {
	t.Helper()
	script := "#!/bin/sh\n" +
		"if [ \"$1\" = \"--version\" ]; then echo \"faketool version " + version + "\"; exit 0; fi\n" +
		"if [ \"$1\" = \"--help\" ]; then cat <<'HELP'\n" + helpFixture + "HELP\n exit 0; fi\n" +
		"echo 'unknown' >&2; exit 1\n"
	path := filepath.Join(dir, "faketool")
	if err := os.WriteFile(path, []byte(script), 0o755); err != nil {
		t.Fatal(err)
	}
	return path
}

func TestGenerateForScriptBinaryAndVersionDrift(t *testing.T) {
	dir := t.TempDir()
	bin := fakeTool(t, dir, "1.2.3")
	opts, err := splitImportArgsFull([]string{"--source", "help", bin})
	if err != nil {
		t.Fatal(err)
	}
	spec, err := generateImport(opts, genNow)
	if err != nil {
		t.Fatal(err)
	}
	if spec.Provenance.BinaryVersion != "1.2.3" {
		t.Fatalf("provenance records the --version the binary printed: %+v", spec.Provenance)
	}
	// Oracle: the binary's own --version output. Same version, no
	// drift; after the script is rewritten at a new version, drift.
	ctx := t.Context()
	declared, found, drift, err := spec.VersionDrift(ctx)
	if err != nil || drift || declared != "1.2.3" || found != "1.2.3" {
		t.Fatalf("same version should not drift: %q %q %v %v", declared, found, drift, err)
	}
	fakeTool(t, dir, "1.3.0")
	_, found, drift, err = spec.VersionDrift(ctx)
	if err != nil || !drift || found != "1.3.0" {
		t.Fatalf("a rewritten binary at 1.3.0 should drift against 1.2.3: %q %v %v", found, drift, err)
	}
}

func TestExtractVersionShapes(t *testing.T) {
	for in, want := range map[string]string{
		"git version 2.43.0\n":       "2.43.0",
		"jq-1.7.1\n":                 "1.7.1",
		"faketool version 1.2\nmore": "1.2",
		"no version here\n":          "",
		"version 3 is not dotted\n":  "",
		"1.2.3-rc.1 built today\n":   "1.2.3-rc.1",
	} {
		if got := adapter.ExtractVersion(in); got != want {
			t.Errorf("ExtractVersion(%q) = %q, want %q", in, got, want)
		}
	}
}

func TestGenerateForGitFromHelp(t *testing.T) {
	// The acceptance check from #1962 itself: a generated
	// declaration for a well-known binary (git) that passes
	// validation and whose recorded version matches the live one.
	if _, err := exec.LookPath("git"); err != nil {
		t.Skip("git is not installed here")
	}
	opts, err := splitImportArgsFull([]string{"--source", "help", "git"})
	if err != nil {
		t.Fatal(err)
	}
	spec, err := generateImport(opts, genNow)
	if err != nil {
		t.Fatal(err)
	}
	if spec.Name != "git" || len(spec.Tools) == 0 {
		t.Fatalf("git's --help should yield tools: %+v", spec)
	}
	names := map[string]bool{}
	for _, tool := range spec.Tools {
		names[tool.Name] = true
	}
	if !names["status"] && !names["add"] && !names["commit"] {
		t.Errorf("git's common commands should be among the tools: %v", names)
	}
	out, err := exec.Command("git", "--version").Output()
	if err != nil {
		t.Fatal(err)
	}
	if want := adapter.ExtractVersion(string(out)); spec.Provenance.BinaryVersion != want {
		t.Errorf("recorded version %q should match live git %q", spec.Provenance.BinaryVersion, want)
	}
}

// The tests below pin the Codex review findings on PR #9041: each
// names the external behaviour it is an oracle for.

func TestParseHelpCommandsWithPositionals(t *testing.T) {
	// Oracle: networkctl's real --help shape -- a command entry
	// carrying positional syntax still names that command, with
	// the description from its column.
	const fixture = `networkctl [OPTIONS...] COMMAND ...

Commands:
  list [PATTERN...]      List links
  status [PATTERN...]    Show link status
  up DEVICES...          Bring devices up
  reload                 Reload configuration
`
	parsed := parseHelpText(fixture)
	descs := map[string]string{}
	for _, tool := range parsed.tools {
		descs[tool.name] = tool.description
	}
	for name, want := range map[string]string{
		"list": "List links", "status": "Show link status",
		"up": "Bring devices up", "reload": "Reload configuration",
	} {
		if got, ok := descs[name]; !ok || got != want {
			t.Errorf("command %q: got %q (present %v), want %q", name, got, ok, want)
		}
	}
}

func TestZshFlagSpecArgAfterDescription(t *testing.T) {
	// Oracle: the zsh _arguments grammar -- ':format:' after the
	// description bracket is the option's argument field, so the
	// option takes a value; no field after the bracket is boolean.
	f, ok := parseZshFlagSpec("--json[Shows output formatted as JSON]:format:_networkctl_get_json")
	if !ok || f.arg == "" || f.description != "Shows output formatted as JSON" {
		t.Errorf("an argument field after the description makes the flag value-taking: %+v ok=%v", f, ok)
	}
	f, ok = parseZshFlagSpec("--verbose[Say more]")
	if !ok || f.arg != "" {
		t.Errorf("no argument field means boolean: %+v ok=%v", f, ok)
	}
}

func TestGenerateCompletionRejectsPositional(t *testing.T) {
	// Oracle: every argument must be accounted for -- a positional
	// beside --completion names a different program than the file
	// describes and must be refused, never silently ignored.
	dir := t.TempDir()
	file := filepath.Join(dir, "faketool.fish")
	if err := os.WriteFile(file, []byte(fishFixture), 0o644); err != nil {
		t.Fatal(err)
	}
	opts, err := splitImportArgsFull([]string{"intended-binary", "--completion", file})
	if err != nil {
		t.Fatal(err)
	}
	if _, err := generateImport(opts, genNow); err == nil {
		t.Fatal("a positional argument beside --completion must be refused")
	}
}

func TestGenerateFromCompletionKeepsExecutableSpelling(t *testing.T) {
	// Oracle: the file's name names the executable as installed --
	// python3.12 is probed and run as python3.12, while the
	// adapter namespace normalizes to python3-12 so a call can
	// address it.
	if got := completionName("/x/python3.12.fish"); got != "python3.12" {
		t.Fatalf("completionName keeps the executable spelling: %q", got)
	}
	dir := t.TempDir()
	file := filepath.Join(dir, "python3.12.fish")
	if err := os.WriteFile(file, []byte(fishFixture), 0o644); err != nil {
		t.Fatal(err)
	}
	opts, err := splitImportArgsFull([]string{"--completion", file})
	if err != nil {
		t.Fatal(err)
	}
	spec, err := generateImport(opts, genNow)
	if err != nil {
		t.Fatal(err)
	}
	if spec.Command != "python3.12" || spec.Name != "python3-12" {
		t.Errorf("command keeps the executable spelling, namespace normalizes: name=%q command=%q", spec.Name, spec.Command)
	}
}

func TestGeneratedGlobalParamsAreMarkedGlobal(t *testing.T) {
	// Oracle: the generated declaration itself -- options parsed
	// from a program-level source are marked Global on every tool,
	// so buildArgs stands them before the subcommand.
	spec, err := buildGeneratedSpec("faketool", "faketool", "help", helpFixture,
		parseHelpText(helpFixture), "", "", genNow)
	if err != nil {
		t.Fatal(err)
	}
	for _, tool := range spec.Tools {
		for _, p := range tool.Params {
			if p.Name == "verbose" && !p.Global {
				t.Errorf("tool %s: the program-level --verbose is not marked global: %+v", tool.Name, p)
			}
		}
	}
}

func TestVersionDriftReadsStderrVersion(t *testing.T) {
	// Oracle: the binary's own --version, on whichever stream it
	// writes -- the import probe reads combined output, so the
	// drift check must too, or an unchanged binary whose version
	// goes to stderr reads as versionless.
	dir := t.TempDir()
	path := filepath.Join(dir, "faketool")
	script := "#!/bin/sh\nif [ \"$1\" = \"--version\" ]; then echo 'faketool version 1.2.3' >&2; exit 0; fi\nexit 1\n"
	if err := os.WriteFile(path, []byte(script), 0o755); err != nil {
		t.Fatal(err)
	}
	spec := adapter.Spec{
		Name: "faketool", Command: path,
		Provenance: &adapter.Provenance{Source: "help", BinaryVersion: "1.2.3"},
	}
	_, found, drift, err := spec.VersionDrift(t.Context())
	if err != nil || drift || found != "1.2.3" {
		t.Fatalf("a version printed to stderr compares like any other: found=%q drift=%v err=%v", found, drift, err)
	}
}
