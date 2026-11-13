package cli

import (
	"context"
	"encoding/json"
	"flag"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"regexp"
	"sort"
	"strings"
	"time"

	"github.com/developing-today/code/pkgs/mcpx/internal/adapter"
)

// Declaration generation for `mcpx adapter import` (#1962 steps 2-5).
//
// The source ladder is the issue's: completions first (they are generated
// data structures, not prose), then man pages, then --help text. Every
// source parses deterministically -- no model anywhere in the loop -- and
// a source that yields nothing is an error naming what was tried, never a
// partial declaration emitted silently. The winner is recorded in the
// declaration's provenance, together with the binary's resolved path and
// version, so `adapter check` can report version drift instead of the
// declaration silently going stale (#83's open question, answered).

type importOptions struct {
	bare        bool
	description string
	out         string
	source      string // auto, completion, man or help; empty means auto
	completion  string // a completion script to parse instead of a binary
	shell       string // bash, zsh or fish; empty sniffs the file
	name        string
	rest        []string
}

const importUsageFull = "usage: mcpx adapter import <binary> [--source auto|completion|man|help] [--out FILE]\n" +
	"       mcpx adapter import --completion <file> [--shell bash|zsh|fish] [--name NAME] [--out FILE]\n" +
	"       mcpx adapter import --bare <binary> --description \"what it is for\" [--out FILE]"

// parseImportArgs is splitImportArgs grown up: import's flags are its
// own, lifted wherever they appear, because the stdlib parser stops at
// the first positional and `import echo --source help` must work.
func parseImportArgs(args []string) (importOptions, error) {
	var o importOptions
	valued := map[string]*string{
		"--description": &o.description,
		"--out":         &o.out,
		"--source":      &o.source,
		"--completion":  &o.completion,
		"--shell":       &o.shell,
		"--name":        &o.name,
	}
	for i := 0; i < len(args); i++ {
		a := args[i]
		switch {
		case a == "--":
			o.rest = append(o.rest, args[i+1:]...)
			i = len(args)
		case a == "--bare":
			o.bare = true
		case a == "--help" || a == "-h":
			return o, errImportHelp
		case strings.HasPrefix(a, "--"):
			name, val, hasVal := strings.Cut(a, "=")
			dst, ok := valued[name]
			if !ok {
				return o, fmt.Errorf("unknown flag %q: %s", name, importUsageFull)
			}
			if !hasVal {
				if i+1 >= len(args) {
					return o, fmt.Errorf("%s needs a value: %s", name, importUsageFull)
				}
				i++
				val = args[i]
			}
			*dst = val
		case strings.HasPrefix(a, "-") && a != "-":
			return o, fmt.Errorf("unknown flag %q: %s", a, importUsageFull)
		default:
			o.rest = append(o.rest, a)
		}
	}
	switch o.source {
	case "", "auto", "completion", "man", "help":
	default:
		return o, fmt.Errorf("unknown --source %q: auto, completion, man or help", o.source)
	}
	switch o.shell {
	case "", "bash", "zsh", "fish":
	default:
		return o, fmt.Errorf("unknown --shell %q: bash, zsh or fish", o.shell)
	}
	return o, nil
}

var errImportHelp = fmt.Errorf("help requested")

// splitImportArgs is the original three-flag partition, kept as a
// projection of parseImportArgs so the two never disagree.
func splitImportArgs(args []string) (bare bool, description, out string, rest []string, err error) {
	o, perr := parseImportArgs(args)
	if perr != nil {
		if perr == errImportHelp {
			return false, "", "", nil, flag.ErrHelp
		}
		return false, "", "", nil, perr
	}
	return o.bare, o.description, o.out, o.rest, nil
}

// adapterImport emits one declaration: the --bare passthrough, a
// declaration generated from a binary down the source ladder, or a
// declaration generated from a completion file given directly.
func adapterImport(args []string) error {
	o, err := parseImportArgs(args)
	if err == errImportHelp {
		fmt.Fprintln(os.Stderr, importUsageFull)
		return flag.ErrHelp
	}
	if err != nil {
		return err
	}
	var spec adapter.Spec
	var notes []string
	switch {
	case o.bare:
		if o.completion != "" {
			return fmt.Errorf("--bare takes a binary, not --completion: %s", importUsageFull)
		}
		if len(o.rest) != 1 {
			fmt.Fprintln(os.Stderr, importUsageFull)
			return fmt.Errorf("mcpx adapter import --bare takes exactly one binary")
		}
		spec, err = buildBareImport(o.rest[0], o.description)
		if err != nil {
			return err
		}
	case o.completion != "":
		if len(o.rest) != 0 {
			return fmt.Errorf("--completion takes no binary argument: %s", importUsageFull)
		}
		spec, notes, err = buildCompletionFileImport(o)
		if err != nil {
			return err
		}
	default:
		if len(o.rest) != 1 {
			fmt.Fprintln(os.Stderr, importUsageFull)
			return fmt.Errorf("mcpx adapter import takes exactly one binary (or --completion <file>)")
		}
		spec, notes, err = buildGeneratedImport(o.rest[0], o)
		if err != nil {
			return err
		}
	}
	doc, err := marshalAdapterFile(spec)
	if err != nil {
		return err
	}
	if o.out != "" {
		if err := writeFileAtomic(o.out, doc, 0o644); err != nil {
			return fmt.Errorf("writing %s: %w", o.out, err)
		}
	} else {
		fmt.Print(string(doc))
	}
	if o.bare {
		fmt.Fprint(os.Stderr, bareImportWarning(spec))
	}
	for _, n := range notes {
		fmt.Fprintln(os.Stderr, "note:", n)
	}
	return nil
}

func marshalAdapterFile(spec adapter.Spec) ([]byte, error) {
	doc, err := json.MarshalIndent(AdapterFile{Adapters: []adapter.Spec{spec}}, "", "  ")
	if err != nil {
		return nil, err
	}
	return append(doc, '\n'), nil
}

// ---- generated declarations -------------------------------------------

// importFlag is one option a source described, before it becomes a Param.
type importFlag struct {
	Short      string // one letter, no dash
	Long       string // no dashes
	Value      string // value placeholder, when the source names one
	Desc       string
	TakesValue bool
}

func (f importFlag) flag() string {
	if f.Long != "" {
		return "--" + f.Long
	}
	return "-" + f.Short
}

// paramName is the caller-facing name: the long flag with dashes folded
// to underscores, or the short letter when there is no long form.
func (f importFlag) paramName() string {
	if f.Long != "" {
		return strings.ReplaceAll(f.Long, "-", "_")
	}
	return f.Short
}

type importSub struct {
	Name  string
	Desc  string
	Flags []importFlag
}

// importShape is what any source parses into: subcommands with their own
// flags, flags that belong to the binary itself, and declared
// positionals. The shape is source-neutral so the ladder can fall
// through without the emitter caring which rung produced it.
type importShape struct {
	Description string
	Subs        []importSub
	Global      []importFlag
	Positionals []importFlag // Long holds the name; always TakesValue
}

// empty reports whether the parse found nothing worth declaring: the
// ladder's failure condition.
func (s *importShape) empty() bool {
	return s == nil || (len(s.Subs) == 0 && len(s.Global) == 0 && len(s.Positionals) == 0)
}

func flagParams(flags []importFlag) []adapter.Param {
	var out []adapter.Param
	used := map[string]bool{}
	for _, f := range flags {
		if f.Long == "help" || f.Long == "version" {
			continue // every binary has them; they are not the surface
		}
		p := adapter.Param{
			Name:        f.paramName(),
			Flag:        f.flag(),
			Description: f.Desc,
		}
		if f.TakesValue {
			p.Type = "string"
			if v := strings.ToLower(f.Value); strings.Contains(v, "int") || strings.Contains(v, "num") || strings.Contains(v, "count") {
				p.Type = "integer"
			}
		} else {
			p.Type = "boolean"
		}
		if used[p.Name] {
			p.Name += "_flag"
		}
		used[p.Name] = true
		out = append(out, p)
	}
	return out
}

// shapeToSpec renders a parsed shape as a declaration. Global flags of a
// subcommand-ful binary are deliberately NOT folded into the subcommand
// tools: they belong before the subcommand on a real command line
// (git --git-dir X status, not git status --git-dir X), and ToolSpec
// params render after the fixed args. They are reported as a note
// instead of being silently dropped or silently misplaced.
func shapeToSpec(name, command string, shape *importShape, prov *adapter.Provenance) (adapter.Spec, []string, error) {
	if shape.empty() {
		return adapter.Spec{}, nil, fmt.Errorf("nothing could be parsed: no subcommands, flags or positionals found")
	}
	spec := adapter.Spec{
		Name:        name,
		Command:     command,
		Description: shape.Description,
		Provenance:  prov,
	}
	var notes []string
	if len(shape.Subs) > 0 {
		for _, sub := range shape.Subs {
			spec.Tools = append(spec.Tools, adapter.ToolSpec{
				Name:        sub.Name,
				Description: sub.Desc,
				Args:        []string{sub.Name},
				Params:      flagParams(sub.Flags),
			})
		}
		if len(shape.Global) > 0 {
			var names []string
			for _, f := range shape.Global {
				names = append(names, f.flag())
			}
			notes = append(notes, fmt.Sprintf("global flags are not modelled on subcommand tools (they precede the subcommand on a real command line): %s", strings.Join(names, ", ")))
		}
	} else {
		params := flagParams(shape.Global)
		for _, pos := range shape.Positionals {
			params = append(params, adapter.Param{
				Name:        pos.Long,
				Description: pos.Desc,
				Type:        "string",
				Required:    true,
			})
		}
		params = append(params, adapter.Param{
			Name:        "args",
			Type:        "array",
			Description: "Additional positional arguments, passed in order.",
		})
		spec.Tools = []adapter.ToolSpec{{
			Name:        "run",
			Description: shape.Description,
			Params:      params,
		}}
	}
	if err := spec.Validate(); err != nil {
		return adapter.Spec{}, nil, fmt.Errorf("the generated declaration does not validate: %w", err)
	}
	return spec, notes, nil
}

// adapterName sanitises a binary or file base name the way
// buildBareImport does: dots become dashes so `adapter call <name>.<tool>`
// can address it.
func adapterName(base string) string {
	return strings.ReplaceAll(base, ".", "-")
}

// buildGeneratedImport walks the source ladder for a binary.
func buildGeneratedImport(binary string, o importOptions) (adapter.Spec, []string, error) {
	resolved, err := exec.LookPath(binary)
	if err != nil {
		return adapter.Spec{}, nil, fmt.Errorf("cannot import %q: %w", binary, err)
	}
	name := o.name
	if name == "" {
		name = adapterName(filepath.Base(binary))
	}
	version, verr := binaryVersion(binary)

	var shape *importShape
	var source, detail string
	var tried []string
	try := func(src string, got *importShape, det string) bool {
		if got.empty() {
			tried = append(tried, src)
			return false
		}
		shape, source, detail = got, src, det
		return true
	}
	want := o.source
	if want == "" {
		want = "auto"
	}
	done := false
	if want == "auto" || want == "completion" {
		sh, det := completionForBinary(binary, filepath.Base(binary))
		done = try("completion", sh, det)
		if !done && want == "completion" {
			return adapter.Spec{}, nil, fmt.Errorf("no completion source for %q: no system completion file and the binary printed nothing parseable for `completion fish|zsh|bash`", binary)
		}
	}
	if !done && (want == "auto" || want == "man") {
		sh, merr := manShape(binary)
		if merr == nil {
			done = try("man", sh, "man page")
		}
		if !done && want == "man" {
			if merr != nil {
				return adapter.Spec{}, nil, fmt.Errorf("cannot import %q from its man page: %w", binary, merr)
			}
			return adapter.Spec{}, nil, fmt.Errorf("the man page for %q has no OPTIONS section mcpx can parse", binary)
		}
	}
	if !done && (want == "auto" || want == "help") {
		sh, herr := helpShape(binary)
		if herr != nil {
			return adapter.Spec{}, nil, fmt.Errorf("cannot import %q from --help: %w", binary, herr)
		}
		if !try("help", sh, "--help") {
			return adapter.Spec{}, nil, fmt.Errorf("the --help output of %q has no subcommands, flags or positionals mcpx can parse", binary)
		}
	}
	prov := &adapter.Provenance{
		Source:        source,
		SourceDetail:  detail,
		BinaryPath:    resolved,
		BinaryVersion: version,
		GeneratedAt:   time.Now().UTC().Format(time.RFC3339),
		Generator:     "mcpx adapter import",
	}
	spec, notes, err := shapeToSpec(name, binary, shape, prov)
	if err != nil {
		return adapter.Spec{}, nil, fmt.Errorf("importing %q from %s: %w", binary, source, err)
	}
	if spec.Description == "" {
		spec.Description = fmt.Sprintf("%s, generated from %s by mcpx adapter import", name, source)
	}
	notes = append(notes, fmt.Sprintf("generated %s from %s (%s)", name, source, detail))
	if verr != nil || version == "" {
		notes = append(notes, fmt.Sprintf("could not read a version from `%s --version`; the declaration records none, so adapter check cannot report drift for it", binary))
	} else {
		notes = append(notes, fmt.Sprintf("recorded %s version %s; adapter check will report drift against it", name, version))
	}
	if len(tried) > 0 {
		notes = append(notes, "sources tried first and found empty: "+strings.Join(tried, ", "))
	}
	return spec, notes, nil
}

// buildCompletionFileImport parses a completion script the user hands
// over directly: the parser's testable front door, and the only path for
// programs whose completion exists but whose binary is not here.
func buildCompletionFileImport(o importOptions) (adapter.Spec, []string, error) {
	b, err := os.ReadFile(o.completion)
	if err != nil {
		return adapter.Spec{}, nil, fmt.Errorf("reading completion file: %w", err)
	}
	shell := o.shell
	if shell == "" {
		shell = sniffCompletionShell(string(b), o.completion)
	}
	if shell == "" {
		return adapter.Spec{}, nil, fmt.Errorf("%s does not look like a bash, zsh or fish completion script; say which with --shell", o.completion)
	}
	base := filepath.Base(o.completion)
	name := o.name
	if name == "" {
		name = adapterName(completionName(base, shell))
	}
	if name == "" || name == "-" {
		return adapter.Spec{}, nil, fmt.Errorf("cannot tell which program %s completes; name it with --name", o.completion)
	}
	shape := parseCompletion(string(b), shell, name)
	if shape.empty() {
		return adapter.Spec{}, nil, fmt.Errorf("%s yielded no subcommands or flags as a %s completion; nothing to declare", o.completion, shell)
	}
	prov := &adapter.Provenance{
		Source:       "completion",
		SourceDetail: o.completion,
		GeneratedAt:  time.Now().UTC().Format(time.RFC3339),
		Generator:    "mcpx adapter import",
	}
	spec, notes, err := shapeToSpec(name, name, shape, prov)
	if err != nil {
		return adapter.Spec{}, nil, err
	}
	if spec.Description == "" {
		spec.Description = fmt.Sprintf("%s, generated from a %s completion by mcpx adapter import", name, shell)
	}
	notes = append(notes, fmt.Sprintf("generated %s from the %s completion at %s; the command is the name — edit the file if the binary lives elsewhere", name, shell, o.completion))
	return spec, notes, nil
}

// completionName derives the program name from a completion file name:
// _git (zsh), git.fish, git.bash, or plain git.
func completionName(base, shell string) string {
	switch shell {
	case "zsh":
		return strings.TrimPrefix(base, "_")
	case "fish":
		return strings.TrimSuffix(base, ".fish")
	case "bash":
		base = strings.TrimSuffix(base, ".bash")
		return strings.TrimSuffix(base, ".sh")
	}
	return base
}

// sniffCompletionShell recognises a completion script by its load-bearing
// lines: fish's complete builtin calls, zsh's #compdef header, bash's
// complete builtin with a function.
func sniffCompletionShell(text, path string) string {
	if strings.HasSuffix(path, ".fish") || strings.Contains(text, "\ncomplete -c ") || strings.HasPrefix(text, "complete -c ") {
		return "fish"
	}
	if strings.HasPrefix(text, "#compdef") || strings.Contains(text, "_arguments") {
		return "zsh"
	}
	if strings.Contains(text, "complete -F ") || strings.Contains(text, "complete -o ") {
		return "bash"
	}
	return ""
}

// completionForBinary finds a completion for a program: packager-shipped
// files first (the strongest version of this feature — `import jq`
// finding a completion nobody has to generate), then the binary's own
// `completion <shell>` output, the cobra/clap convention.
func completionForBinary(binary, base string) (*importShape, string) {
	for _, shell := range []string{"fish", "zsh", "bash"} {
		for _, path := range systemCompletionPaths(shell, base) {
			b, err := os.ReadFile(path)
			if err != nil {
				continue
			}
			if shape := parseCompletion(string(b), shell, base); !shape.empty() {
				return shape, path
			}
		}
	}
	for _, shell := range []string{"fish", "zsh", "bash"} {
		out, err := runCapture(binary, "completion", shell)
		if err != nil || out == "" {
			continue
		}
		if shape := parseCompletion(out, shell, base); !shape.empty() {
			return shape, binary + " completion " + shell
		}
	}
	return nil, ""
}

func systemCompletionPaths(shell, base string) []string {
	home, _ := os.UserHomeDir()
	switch shell {
	case "fish":
		return []string{
			"/usr/share/fish/completions/" + base + ".fish",
			"/usr/local/share/fish/completions/" + base + ".fish",
			filepath.Join(home, ".local/share/fish/completions", base+".fish"),
		}
	case "zsh":
		return []string{
			"/usr/share/zsh/site-functions/_" + base,
			"/usr/local/share/zsh/site-functions/_" + base,
		}
	case "bash":
		return []string{
			"/usr/share/bash-completion/completions/" + base,
			"/usr/local/share/bash-completion/completions/" + base,
		}
	}
	return nil
}

func parseCompletion(text, shell, name string) *importShape {
	switch shell {
	case "fish":
		return parseFishCompletion(text, name)
	case "zsh":
		return parseZshCompletion(text, name)
	case "bash":
		return parseBashCompletion(text, name)
	}
	return nil
}

// runCapture runs a program for its text output with a bound: import
// probes binaries, and a probe that hangs would hang the import.
func runCapture(binary string, args ...string) (string, error) {
	ctx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
	defer cancel()
	cmd := exec.CommandContext(ctx, binary, args...)
	var out strings.Builder
	cmd.Stdout = &out
	cmd.Stderr = &out
	err := cmd.Run()
	if out.Len() > 0 {
		return out.String(), nil
	}
	return "", err
}

var versionTokenRE = regexp.MustCompile(`\d+\.\d+(?:\.\d+)?`)

// binaryVersion reads a program's version the one way nearly every
// program offers: `--version`, first line, first version-shaped token.
// The token, not the whole line, so "git version 2.47.0" and a later
// "git version 2.48.0" compare as versions rather than prose.
func binaryVersion(command string) (string, error) {
	out, err := runCapture(command, "--version")
	if err != nil {
		return "", err
	}
	for _, line := range strings.Split(out, "\n") {
		line = strings.TrimSpace(line)
		if line == "" {
			continue
		}
		if tok := versionTokenRE.FindString(line); tok != "" {
			return tok, nil
		}
		return line, nil
	}
	return "", fmt.Errorf("%s --version printed nothing", command)
}

// versionDrift is adapter check's half of provenance: the version the
// declaration was written against, against the one the binary reports
// now. The bool is the answer; the strings are for the message.
func versionDrift(s adapter.Spec) (declared, found string, drift bool) {
	if s.Provenance == nil || s.Provenance.BinaryVersion == "" {
		return "", "", false
	}
	found, err := binaryVersion(s.Command)
	if err != nil || found == "" {
		return s.Provenance.BinaryVersion, "", false
	}
	return s.Provenance.BinaryVersion, found, found != s.Provenance.BinaryVersion
}

// sortedFlagNames is a small deterministic-order helper for notes.
func sortedFlagNames(flags []importFlag) []string {
	var names []string
	for _, f := range flags {
		names = append(names, f.flag())
	}
	sort.Strings(names)
	return names
}
