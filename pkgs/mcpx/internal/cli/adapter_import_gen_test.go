package cli

import (
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/developing-today/code/pkgs/mcpx/internal/adapter"
)

// The parsers' oracle is a hand-written shape: each fixture below is a
// small, real-shaped source, and the expected subcommands, flags and
// value-taking marks are written out in full rather than derived from
// the parser under test.

const fishFixture = `complete -c toolbox -n '__fish_use_subcommand' -a 'add list' -d 'subcommands'
complete -c toolbox -s h -l help -d 'show help'
complete -c toolbox -l config -r -d 'path to config file'
complete -c toolbox -n '__fish_seen_subcommand_from add' -s f -l force -d 'overwrite existing'
complete -c toolbox -n '__fish_seen_subcommand_from add' -l name -r -d 'item name'
complete -c toolbox -n '__fish_seen_subcommand_from list' -s a -l all -d 'show all items'
complete -c othertool -l bogus -d 'not ours'
`

func TestParseFishCompletion(t *testing.T) {
	shape := parseFishCompletion(fishFixture, "toolbox")
	if len(shape.Subs) != 2 || shape.Subs[0].Name != "add" || shape.Subs[1].Name != "list" {
		t.Fatalf("subcommands: %+v", shape.Subs)
	}
	if len(shape.Global) != 2 {
		t.Fatalf("global flags: %+v", shape.Global)
	}
	if shape.Global[0].Long != "help" || shape.Global[0].Short != "h" {
		t.Errorf("first global flag: %+v", shape.Global[0])
	}
	if shape.Global[1].Long != "config" || !shape.Global[1].TakesValue || shape.Global[1].Desc != "path to config file" {
		t.Errorf("config flag: %+v", shape.Global[1])
	}
	add := shape.Subs[0]
	if len(add.Flags) != 2 || add.Flags[0].Long != "force" || add.Flags[0].TakesValue {
		t.Errorf("add flags: %+v", add.Flags)
	}
	if add.Flags[1].Long != "name" || !add.Flags[1].TakesValue {
		t.Errorf("add --name: %+v", add.Flags[1])
	}
	list := shape.Subs[1]
	if len(list.Flags) != 1 || list.Flags[0].Long != "all" || list.Flags[0].Short != "a" {
		t.Errorf("list flags: %+v", list.Flags)
	}
}

const helpFixture = `Toolbox does things with items.

Usage:
  toolbox [command]

Available Commands:
  add         Add an item
  list        List items
  help        Help about any command

Flags:
  -h, --help            help for toolbox
      --config string   config file (default "toolbox.yaml")
  -v, --verbose         verbose output

Use "toolbox [command] --help" for more information about a command.
`

func TestParseHelpText(t *testing.T) {
	shape := parseHelpText(helpFixture)
	if len(shape.Subs) != 2 || shape.Subs[0].Name != "add" || shape.Subs[1].Name != "list" {
		t.Fatalf("subcommands: %+v", shape.Subs)
	}
	if shape.Subs[0].Desc != "Add an item" {
		t.Errorf("add description: %q", shape.Subs[0].Desc)
	}
	if len(shape.Global) != 3 {
		t.Fatalf("global flags: %+v", shape.Global)
	}
	byLong := map[string]importFlag{}
	for _, f := range shape.Global {
		byLong[f.Long] = f
	}
	if !byLong["config"].TakesValue || byLong["config"].Value != "string" {
		t.Errorf("config flag: %+v", byLong["config"])
	}
	if byLong["verbose"].TakesValue || byLong["verbose"].Short != "v" {
		t.Errorf("verbose flag: %+v", byLong["verbose"])
	}
}

const manFixture = `TOOLBOX(1)                     User Commands                    TOOLBOX(1)

NAME
       toolbox - does things with items

SYNOPSIS
       toolbox [OPTIONS] COMMAND

OPTIONS
       -a, --all
              show all items

       --config=FILE
              read configuration from FILE

       --exec-path[=<path>]
              where the executables live; note that
              -wrapped description lines are not flags

       -v
              verbose output

AUTHOR
       Somebody
`

func TestParseManText(t *testing.T) {
	shape := parseManText(manFixture)
	if shape.Description != "does things with items" {
		t.Errorf("description: %q", shape.Description)
	}
	if len(shape.Global) != 4 {
		t.Fatalf("flags: %+v", shape.Global)
	}
	if shape.Global[0].Long != "all" || shape.Global[0].Short != "a" || shape.Global[0].TakesValue {
		t.Errorf("all flag: %+v", shape.Global[0])
	}
	if shape.Global[0].Desc != "show all items" {
		t.Errorf("all description: %q", shape.Global[0].Desc)
	}
	if shape.Global[1].Long != "config" || !shape.Global[1].TakesValue || shape.Global[1].Value != "FILE" {
		t.Errorf("config flag: %+v", shape.Global[1])
	}
	if shape.Global[2].Long != "exec-path" || !shape.Global[2].TakesValue || shape.Global[2].Value != "path" {
		t.Errorf("exec-path flag (bracketed value form): %+v", shape.Global[2])
	}
	if !strings.Contains(shape.Global[2].Desc, "-wrapped description lines are not flags") {
		t.Errorf("a wrapped description line starting with a dash belongs to the description: %q", shape.Global[2].Desc)
	}
	if shape.Global[3].Short != "v" || shape.Global[3].TakesValue {
		t.Errorf("v flag: %+v", shape.Global[3])
	}
}

const zshFixture = `#compdef toolbox

_arguments \
  '--config=[config file]:file:_files' \
  '--verbose[verbose output]' \
  '-a[show all]' \
  '1:command:(add list)'
`

func TestParseZshCompletion(t *testing.T) {
	shape := parseZshCompletion(zshFixture, "toolbox")
	if len(shape.Global) != 3 {
		t.Fatalf("flags: %+v", shape.Global)
	}
	if shape.Global[0].Long != "config" || !shape.Global[0].TakesValue || shape.Global[0].Desc != "config file" {
		t.Errorf("config flag: %+v", shape.Global[0])
	}
	if shape.Global[1].Long != "verbose" || shape.Global[1].TakesValue {
		t.Errorf("verbose flag: %+v", shape.Global[1])
	}
	if shape.Global[2].Short != "a" {
		t.Errorf("a flag: %+v", shape.Global[2])
	}
}

const bashFixture = `_toolbox() {
  local commands="add list"
  local opts="--all --config --verbose"
  COMPREPLY=()
}
complete -F _toolbox toolbox
`

func TestParseBashCompletion(t *testing.T) {
	shape := parseBashCompletion(bashFixture, "toolbox")
	if len(shape.Subs) != 2 || shape.Subs[0].Name != "add" || shape.Subs[1].Name != "list" {
		t.Fatalf("subcommands: %+v", shape.Subs)
	}
	if len(shape.Global) != 3 {
		t.Fatalf("flags: %+v", shape.Global)
	}
	for _, f := range shape.Global {
		if !f.TakesValue {
			t.Errorf("bash flags are declared string-valued (the source cannot say): %+v", f)
		}
	}
}

func TestShapeToSpecSubcommands(t *testing.T) {
	shape := parseFishCompletion(fishFixture, "toolbox")
	prov := &adapter.Provenance{Source: "completion", SourceDetail: "fixture", GeneratedAt: "2026-01-01T00:00:00Z", Generator: "test"}
	spec, notes, err := shapeToSpec("toolbox", "toolbox", shape, prov)
	if err != nil {
		t.Fatal(err)
	}
	if len(spec.Tools) != 2 {
		t.Fatalf("tools: %+v", spec.Tools)
	}
	add := spec.Tools[0]
	if add.Name != "add" || len(add.Args) != 1 || add.Args[0] != "add" {
		t.Errorf("add tool: %+v", add)
	}
	byName := map[string]adapter.Param{}
	for _, p := range add.Params {
		byName[p.Name] = p
	}
	if byName["force"].Type != "boolean" || byName["force"].Flag != "--force" {
		t.Errorf("force param: %+v", byName["force"])
	}
	if byName["name"].Type != "string" || byName["name"].Flag != "--name" {
		t.Errorf("name param: %+v", byName["name"])
	}
	// The one global flag that survives param filtering (--help is
	// skipped) must be reported as unmodelled, not silently dropped.
	joined := strings.Join(notes, " ")
	if !strings.Contains(joined, "--config") {
		t.Errorf("notes should name the unmodelled global flag: %q", joined)
	}
	if spec.Provenance == nil || spec.Provenance.Source != "completion" {
		t.Errorf("provenance: %+v", spec.Provenance)
	}
}

func TestShapeToSpecSingleTool(t *testing.T) {
	shape := parseManText(manFixture)
	spec, _, err := shapeToSpec("toolbox", "toolbox", shape, &adapter.Provenance{Source: "man", GeneratedAt: "2026-01-01T00:00:00Z", Generator: "test"})
	if err != nil {
		t.Fatal(err)
	}
	if len(spec.Tools) != 1 || spec.Tools[0].Name != "run" {
		t.Fatalf("tools: %+v", spec.Tools)
	}
	last := spec.Tools[0].Params[len(spec.Tools[0].Params)-1]
	if last.Name != "args" || last.Type != "array" {
		t.Errorf("trailing param should be the positional catch-all: %+v", last)
	}
}

func TestShapeToSpecCatchAllNameNeverCollides(t *testing.T) {
	// #9558: imported help that itself declares a positional named
	// `args` (or a `--args` flag) must not yield two `args` params —
	// Validate accepts duplicates, Schema overwrites, and buildArgs
	// would render both from the same input. The parsed param keeps
	// its name; the catch-all is renamed, in both branches.
	prov := &adapter.Provenance{Source: "help", GeneratedAt: "2026-01-01T00:00:00Z", Generator: "test"}
	argsFlag := importFlag{Long: "args", Desc: "the args flag", TakesValue: true}
	argsPos := importFlag{Long: "args", Desc: "the args positional", TakesValue: true}

	assertUnique := func(t *testing.T, tool adapter.ToolSpec, wantParsed bool) {
		t.Helper()
		seen := map[string]int{}
		var catchAll *adapter.Param
		for i := range tool.Params {
			seen[tool.Params[i].Name]++
			if tool.Params[i].Type == "array" {
				catchAll = &tool.Params[i]
			}
		}
		for name, n := range seen {
			if n > 1 {
				t.Errorf("tool %s: param name %q appears %d times: %+v", tool.Name, name, n, tool.Params)
			}
		}
		if catchAll == nil {
			t.Fatalf("tool %s: no array catch-all param: %+v", tool.Name, tool.Params)
		}
		if wantParsed {
			if seen["args"] != 1 {
				t.Errorf("tool %s: parsed `args` param should survive exactly once: %+v", tool.Name, tool.Params)
			}
			if catchAll.Name == "args" {
				t.Errorf("tool %s: catch-all must be renamed off the collision: %+v", tool.Name, tool.Params)
			}
		} else if catchAll.Name != "args" {
			t.Errorf("tool %s: uncollided catch-all keeps the name `args`: %+v", tool.Name, tool.Params)
		}
	}

	// Single-tool branch: positional named args, and --args flag.
	for name, shape := range map[string]*importShape{
		"positional args": {Description: "d", Positionals: []importFlag{argsPos}},
		"flag --args":     {Description: "d", Global: []importFlag{argsFlag}},
		"no collision":    {Description: "d", Positionals: []importFlag{{Long: "item", TakesValue: true}}},
	} {
		spec, _, err := shapeToSpec("toolbox", "toolbox", shape, prov)
		if err != nil {
			t.Fatalf("%s: %v", name, err)
		}
		assertUnique(t, spec.Tools[0], name != "no collision")
	}

	// Subcommand branch: positional named args on the subcommand.
	shape := &importShape{Subs: []importSub{{Name: "add", Desc: "add one", Positionals: []importFlag{argsPos}}}}
	spec, _, err := shapeToSpec("toolbox", "toolbox", shape, prov)
	if err != nil {
		t.Fatal(err)
	}
	assertUnique(t, spec.Tools[0], true)
}

func TestParsersRefuseGarbage(t *testing.T) {
	// Sources that are not machine-regular must yield an empty shape,
	// and an empty shape must be an error at the spec step: no partial
	// declarations.
	garbage := "this is not a completion, a man page, or help output\njust prose about flags\n"
	for name, shape := range map[string]*importShape{
		"fish": parseFishCompletion(garbage, "toolbox"),
		"zsh":  parseZshCompletion(garbage, "toolbox"),
		"bash": parseBashCompletion(garbage, "toolbox"),
		"man":  parseManText(garbage),
		"help": parseHelpText(garbage),
	} {
		if !shape.empty() {
			t.Errorf("%s parsed garbage into %+v", name, shape)
		}
	}
	if _, _, err := shapeToSpec("x", "x", &importShape{}, nil); err == nil {
		t.Error("an empty shape should be refused")
	}
}

func writeFakeBinary(t *testing.T, version string) string {
	t.Helper()
	path := filepath.Join(t.TempDir(), "toolbox")
	script := "#!/bin/sh\necho 'toolbox version " + version + "'\n"
	if err := os.WriteFile(path, []byte(script), 0o755); err != nil {
		t.Fatal(err)
	}
	return path
}

func TestBinaryVersionAndDrift(t *testing.T) {
	bin := writeFakeBinary(t, "1.2.3")
	v, err := binaryVersion(bin)
	if err != nil {
		t.Fatal(err)
	}
	if v != "1.2.3" {
		t.Fatalf("version token: %q", v)
	}
	spec := adapter.Spec{Name: "toolbox", Command: bin, Provenance: &adapter.Provenance{BinaryVersion: "1.2.3"}}
	if _, _, drift := versionDrift(spec); drift {
		t.Error("same version should not drift")
	}
	spec.Provenance.BinaryVersion = "2.0.0"
	declared, found, drift := versionDrift(spec)
	if !drift || declared != "2.0.0" || found != "1.2.3" {
		t.Errorf("drift: declared=%q found=%q drift=%v", declared, found, drift)
	}
	plain := adapter.Spec{Name: "toolbox", Command: bin}
	if _, _, drift := versionDrift(plain); drift {
		t.Error("a declaration without provenance cannot drift")
	}
}

func TestBuildCompletionFileImport(t *testing.T) {
	path := filepath.Join(t.TempDir(), "toolbox.fish")
	if err := os.WriteFile(path, []byte(fishFixture), 0o644); err != nil {
		t.Fatal(err)
	}
	spec, _, err := buildCompletionFileImport(importOptions{completion: path})
	if err != nil {
		t.Fatal(err)
	}
	if spec.Name != "toolbox" || spec.Command != "toolbox" {
		t.Errorf("spec identity: %+v", spec)
	}
	if spec.Provenance == nil || spec.Provenance.Source != "completion" || spec.Provenance.SourceDetail != path {
		t.Errorf("provenance: %+v", spec.Provenance)
	}
	if len(spec.Tools) != 2 {
		t.Errorf("tools: %+v", spec.Tools)
	}
	// The generated declaration must load through the existing path
	// unchanged: marshal it into an adapter file and load it back.
	doc, err := marshalAdapterFile(spec)
	if err != nil {
		t.Fatal(err)
	}
	file := filepath.Join(t.TempDir(), "adapters.json")
	if err := os.WriteFile(file, doc, 0o644); err != nil {
		t.Fatal(err)
	}
	loaded, err := loadAdapterFiles([]string{file})
	if err != nil {
		t.Fatal(err)
	}
	if len(loaded) != 1 || loaded[0].Name != "toolbox" || loaded[0].Provenance == nil {
		t.Errorf("round trip: %+v", loaded)
	}
}

func TestParseImportArgsGenerationFlags(t *testing.T) {
	o, err := parseImportArgs([]string{"--source", "help", "git", "--out=x.json", "--name", "g"})
	if err != nil {
		t.Fatal(err)
	}
	if o.source != "help" || o.out != "x.json" || o.name != "g" || len(o.rest) != 1 || o.rest[0] != "git" {
		t.Errorf("options: %+v", o)
	}
	if _, err := parseImportArgs([]string{"--source", "bogus", "git"}); err == nil {
		t.Error("a bogus --source should be refused")
	}
	if _, err := parseImportArgs([]string{"--completion", "x.fish", "--shell", "tcsh"}); err == nil {
		t.Error("a bogus --shell should be refused")
	}
}
