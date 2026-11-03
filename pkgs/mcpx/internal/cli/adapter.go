package cli

import (
	"context"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"flag"
	"fmt"
	"net/http"
	"os"
	"path/filepath"
	"sort"
	"strings"

	"github.com/developing-today/code/pkgs/mcpx/internal/adapter"
	"github.com/developing-today/code/pkgs/mcpx/internal/config"
	"github.com/developing-today/code/pkgs/mcpx/internal/mcpserver"
)

// AdapterFile is a document declaring command-line programs as servers.
type AdapterFile struct {
	Adapters []adapter.Spec `json:"adapters"`
}

// loadAdapters reads declarations from the configured paths.
func (a *App) loadAdapters() ([]adapter.Spec, error) {
	return loadAdapterFiles(splitPathList(a.Settings().String("paths.adapters")))
}

// loadAdapterFiles reads and validates declarations from these files.
func loadAdapterFiles(paths []string) ([]adapter.Spec, error) {
	if len(paths) == 0 {
		return nil, nil
	}
	var out []adapter.Spec
	for _, p := range paths {
		if p == "" {
			continue
		}
		b, err := os.ReadFile(p)
		if err != nil {
			// A named file that is not there is an error, unlike a search
			// directory: somebody wrote this path down on purpose.
			return nil, fmt.Errorf("reading adapters from %s: %w", p, err)
		}
		var doc AdapterFile
		if err := json.Unmarshal(config.StripJSONC(b), &doc); err != nil {
			// A bare array is the obvious other thing somebody writes.
			var bare []adapter.Spec
			if json.Unmarshal(config.StripJSONC(b), &bare) != nil {
				return nil, fmt.Errorf("%s is not a valid adapter file: %w", p, err)
			}
			doc.Adapters = bare
		}
		out = append(out, doc.Adapters...)
	}
	seen := map[string]string{}
	for _, s := range out {
		if err := s.Validate(); err != nil {
			return nil, err
		}
		if prev, dup := seen[s.Name]; dup {
			return nil, fmt.Errorf("two adapters named %q (%s and here)", s.Name, prev)
		}
		seen[s.Name] = s.Name
	}
	return out, nil
}

// CmdAdapter inspects and runs adapted programs.
//
// A subcommand rather than a flag because the three things somebody wants --
// what is declared, does it work, run one -- are different questions and a
// flag would have to guess which.
func (a *App) CmdAdapter(ctx context.Context, args []string) error {
	sub := ""
	rest := args
	if len(args) > 0 && !strings.HasPrefix(args[0], "-") {
		sub, rest = args[0], args[1:]
	}
	if sub == "import" {
		return adapterImport(rest)
	}
	// add and remove go through the daemon, like servers add/remove: the
	// write has to land where the running daemon reads, and the local
	// declaration files are not the authority for a live change.
	if sub == "add" {
		return a.adapterAdd(ctx, rest)
	}
	if sub == "remove" {
		return a.adapterRemove(ctx, rest)
	}
	fs := newFlagSet("adapter")
	if err := parseFlags(a, fs, rest); err != nil {
		return err
	}
	specs, err := a.loadAdapters()
	if err != nil {
		return err
	}

	switch sub {
	case "", "list":
		if a.JSON {
			return a.out(specs)
		}
		if len(specs) == 0 {
			fmt.Println("No adapters declared. Point paths.adapters at a file:\n" +
				`  { "adapters": [ { "name": "jq", "command": "jq",` + "\n" +
				`      "tools": [ { "name": "run", "params": [{"name":"filter"}] } ] } ] }`)
			return nil
		}
		for _, s := range specs {
			state := "ok"
			if err := s.Probe(); err != nil {
				state = "missing"
			}
			marker := ""
			if s.Bare {
				marker = " bare"
			}
			tools := s.ExposedTools()
			fmt.Printf("%-16s %-10s%s %s (%d tools)\n", s.Name, state, marker, s.Command, len(tools))
			for _, t := range tools {
				fmt.Printf("    %-14s %s\n", t.Name, firstLine(t.Description))
			}
		}
		return nil

	case "check":
		var problems []string
		for _, s := range specs {
			if err := s.Probe(); err != nil {
				problems = append(problems, err.Error())
			}
		}
		if len(problems) > 0 {
			sort.Strings(problems)
			return fmt.Errorf("adapters are not runnable:\n  %s", strings.Join(problems, "\n  "))
		}
		fmt.Printf("%d adapters, all runnable.\n", len(specs))
		return nil

	case "call":
		if len(fs.Args()) < 1 {
			return fmt.Errorf("usage: mcpx adapter call <name>.<tool> '<json>'")
		}
		name, tool, ok := strings.Cut(fs.Arg(0), ".")
		if !ok {
			return fmt.Errorf("name the tool as <adapter>.<tool>")
		}
		var payload map[string]any
		if len(fs.Args()) > 1 {
			if err := json.Unmarshal([]byte(fs.Arg(1)), &payload); err != nil {
				return fmt.Errorf("arguments must be a JSON object: %w", err)
			}
		}
		for _, s := range specs {
			if s.Name != name {
				continue
			}
			res, err := s.Call(ctx, tool, payload)
			if err != nil {
				return err
			}
			if a.JSON {
				return a.out(res)
			}
			if res.Stdout != "" {
				fmt.Print(res.Stdout)
			}
			if res.Stderr != "" {
				fmt.Fprint(os.Stderr, res.Stderr)
			}
			if res.ExitCode != 0 {
				return fmt.Errorf("%s exited %d", name, res.ExitCode)
			}
			return nil
		}
		return fmt.Errorf("no adapter named %q", name)

	case "tools":
		// The MCP tool list an adapter would present, so it can be checked
		// against what a host will see without starting anything.
		var out []mcpserver.Tool
		for _, s := range specs {
			for _, t := range s.ExposedTools() {
				out = append(out, mcpserver.Tool{
					Name:        s.Name + "_" + t.Name,
					Description: t.Description,
					InputSchema: t.Schema(),
				})
			}
		}
		return a.out(out)

	case "serve":
		// One adapter as a stdio MCP server. This is what the daemon spawns
		// for every declared adapter (see augmentAdapters), so an adapted
		// program is an upstream server like any other.
		// Files after the name replace paths.adapters, so the daemon can
		// say exactly which declarations it meant whatever the child's
		// environment or working directory.
		if len(fs.Args()) < 1 {
			return fmt.Errorf("usage: mcpx adapter serve <name> [file...]")
		}
		if len(fs.Args()) > 1 {
			if specs, err = loadAdapterFiles(fs.Args()[1:]); err != nil {
				return err
			}
		}
		for _, s := range specs {
			if s.Name != fs.Arg(0) {
				continue
			}
			srv := mcpserver.New(nil, s.Name, a.Version)
			srv.ExtrasOnly = true
			srv.OwnInstructions = s.Instructions
			srv = srv.WithExtras(adapterTools([]adapter.Spec{s}, false))
			return srv.ServeStdio(ctx, os.Stdin, os.Stdout)
		}
		return fmt.Errorf("no adapter named %q", fs.Arg(0))
	}
	return fmt.Errorf("no adapter subcommand %q; list, check, call, tools, serve, import, add or remove", sub)
}

const importUsage = "usage: mcpx adapter import --bare <binary> --description \"what it is for\" [--out FILE]"

// adapterImport is parsed apart from the adapter command's flags: --bare
// and friends are import's own, not registry settings, and the registry
// flag set would refuse them before the switch ever ran.
func adapterImport(args []string) error {
	// Only the bare passthrough exists so far. Declaration generation
	// from --help, completions and man pages is #1962 steps 2-5, held
	// for the #2118 reconciliation, so anything else is refused loudly
	// rather than emitted silently.
	bare, description, outPath, rest, err := splitImportArgs(args)
	if err == flag.ErrHelp {
		fmt.Fprintln(os.Stderr, importUsage)
		return err
	}
	if err != nil {
		return err
	}
	if !bare {
		return fmt.Errorf("mcpx adapter import only implements --bare so far: declaration generation from --help, completions and man pages is #1962 steps 2-5, pending reconciliation with #2118")
	}
	if len(rest) != 1 {
		fmt.Fprintln(os.Stderr, importUsage)
		return fmt.Errorf("mcpx adapter import --bare takes exactly one binary")
	}
	spec, err := buildBareImport(rest[0], description)
	if err != nil {
		return err
	}
	doc, err := json.MarshalIndent(AdapterFile{Adapters: []adapter.Spec{spec}}, "", "  ")
	if err != nil {
		return err
	}
	doc = append(doc, '\n')
	if outPath != "" {
		if err := writeFileAtomic(outPath, doc, 0o644); err != nil {
			return fmt.Errorf("writing %s: %w", outPath, err)
		}
	} else {
		fmt.Print(string(doc))
	}
	fmt.Fprint(os.Stderr, bareImportWarning(spec))
	return nil
}

// writeFileAtomic writes path via a temporary file in the same
// directory and a rename, so an interrupted write never leaves an
// existing adapter file truncated and unparseable — loadAdapterFiles
// would then fail for every adapter in that file, not just this one.
func writeFileAtomic(path string, doc []byte, perm os.FileMode) error {
	tmp, err := os.CreateTemp(filepath.Dir(path), ".mcpx-import-*")
	if err != nil {
		return err
	}
	tmpName := tmp.Name()
	defer os.Remove(tmpName)
	if err := tmp.Chmod(perm); err != nil {
		tmp.Close()
		return err
	}
	if _, err := tmp.Write(doc); err != nil {
		tmp.Close()
		return err
	}
	if err := tmp.Close(); err != nil {
		return err
	}
	return os.Rename(tmpName, path)
}

// splitImportArgs partitions import's command line by hand: the standard
// flag parser stops at the first positional, so `import --bare echo
// --description ...` would leave --description unparsed. Import takes
// exactly three flags, so they are lifted wherever they appear and the
// rest must be the single binary.
func splitImportArgs(args []string) (bare bool, description, out string, rest []string, err error) {
	for i := 0; i < len(args); i++ {
		a := args[i]
		switch {
		case a == "--":
			rest = append(rest, args[i+1:]...)
			i = len(args)
		case a == "--bare":
			bare = true
		case a == "--description" || a == "--out":
			if i+1 >= len(args) {
				return false, "", "", nil, fmt.Errorf("%s needs a value: %s", a, importUsage)
			}
			i++
			if a == "--description" {
				description = args[i]
			} else {
				out = args[i]
			}
		case strings.HasPrefix(a, "--description="):
			description = strings.TrimPrefix(a, "--description=")
		case strings.HasPrefix(a, "--out="):
			out = strings.TrimPrefix(a, "--out=")
		case a == "--help" || a == "-h":
			return false, "", "", nil, flag.ErrHelp
		case strings.HasPrefix(a, "-") && a != "-":
			return false, "", "", nil, fmt.Errorf("unknown flag %q: %s", a, importUsage)
		default:
			rest = append(rest, a)
		}
	}
	return bare, description, out, rest, nil
}

// buildBareImport returns the passthrough declaration `import --bare`
// emits: the binary's basename as the name, bare set, and the caller's
// description. A bare declaration without one is refused here, and again
// by Spec.Validate on load, because nothing else constrains what a caller
// can reach.
func buildBareImport(binary, description string) (adapter.Spec, error) {
	if strings.TrimSpace(binary) == "" {
		return adapter.Spec{}, fmt.Errorf("usage: mcpx adapter import --bare <binary> --description \"what it is for\" [--out FILE]")
	}
	if strings.TrimSpace(description) == "" {
		return adapter.Spec{}, fmt.Errorf("a description is required for --bare: say what %q is for, since bare exposes its whole argv surface", binary)
	}
	// The name must survive `adapter call <name>.<tool>`, which
	// splits at the first dot: a dotted basename (python3.12) would
	// name an adapter no call can address, so dots become dashes.
	// The command is the binary as given, except that a path-like
	// binary is stored absolute: the adapter server resolves a
	// relative command against its own working directory, not the
	// importer's, so ./bin/tool would probe (or run) whatever
	// happens to sit at that relative path there.
	command := binary
	if strings.ContainsRune(binary, '/') {
		if abs, err := filepath.Abs(binary); err == nil {
			command = abs
		}
	}
	spec := adapter.Spec{
		Name:        strings.ReplaceAll(filepath.Base(binary), ".", "-"),
		Command:     command,
		Bare:        true,
		Description: description,
	}
	if err := spec.Validate(); err != nil {
		return adapter.Spec{}, err
	}
	return spec, nil
}

// bareImportWarning is the loud cost of a passthrough, printed to stderr
// whenever `import --bare` emits: what the caller gives up by skipping
// declared subcommands.
func bareImportWarning(spec adapter.Spec) string {
	return fmt.Sprintf(`!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!
WARNING: %s is a BARE passthrough to %s: no per-subcommand validation.
Any caller can pass ANY arguments straight through, so the binary's whole
argv surface is exposed exactly as if they ran it themselves. Binaries with
argv-level execution escapes (for example git -c core.pager=...) hand a
caller the equivalent of a shell escape. Only declare programs you would let
every caller run directly.
!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!
`, spec.Name, spec.Command)
}

// augmentAdapters adds every declared adapter to cfg as a server of its
// own: `mcpx adapter serve <name>`, under the adapter's name.
//
// That is the whole of making an adapter first-class (#83, #102). Attached
// only as mcpserver extras, adapters reached mcpx's own tools/list and
// nothing else -- not the pool, so not `mcpx ls`, `types`, `search`,
// `catalog`, /v1 or the generated client a script calls. As a server they
// get all of those by the same path a configured server does.
func (a *App) augmentAdapters(cfg *config.Config) error {
	specs, err := a.loadAdapters()
	if err != nil || len(specs) == 0 {
		return err
	}
	self, err := os.Executable()
	if err != nil {
		return fmt.Errorf("adapters need mcpx's own path to serve them: %w", err)
	}
	// The child reads the same files whatever its working directory.
	var files []string
	for _, p := range splitPathList(a.Settings().String("paths.adapters")) {
		if p == "" {
			continue
		}
		if abs, aerr := filepath.Abs(p); aerr == nil {
			p = abs
		}
		files = append(files, p)
	}
	if cfg.MCPServers == nil {
		cfg.MCPServers = map[string]*config.Server{}
	}
	for _, s := range specs {
		if _, dup := cfg.MCPServers[s.Name]; dup {
			return fmt.Errorf("adapter %q has the same name as a configured server; rename one", s.Name)
		}
		cfg.MCPServers[s.Name] = &config.Server{
			Name:    s.Name,
			Command: self,
			Args:    append([]string{"adapter", "serve", s.Name}, files...),
			// The declaration's fingerprint rides in the environment
			// because PoolID hashes the environment: replacing the
			// declaration (command, env, tools) and reloading then
			// replaces this adapter's pool instead of silently keeping
			// the child that captured the old declaration at startup
			// (Codex review, PR #2565; the same shape augmentAPIs uses
			// for APIs since PR #2514).
			Env: map[string]string{"MCPX_ADAPTER_DECLARATION_SHA256": adapterDeclarationFingerprint(s)},
			// Instructions arrive from initialize, as a real server's do.
			Mcpx: &config.Extras{Description: s.Description},
		}
	}
	return nil
}

// adapterDeclarationFingerprint identifies one adapter declaration's
// content, for the process identity augmentAdapters builds.
func adapterDeclarationFingerprint(s adapter.Spec) string {
	h := sha256.New()
	if raw, err := json.Marshal(s); err == nil {
		h.Write(raw)
	}
	return hex.EncodeToString(h.Sum(nil))[:32]
}

// adapterAddValuedFlags are adapter add's flags that take a value, for
// hoistFlags: a flag and its value move together, so --tool status stays
// a pair wherever it started (Codex review, PR #2565).
var adapterAddValuedFlags = map[string]bool{"runtime": true, "description": true, "tool": true}

// adapterToolEntries builds the declaration's tools from the subcommands
// named on the command line.
func adapterToolEntries(values []string) []map[string]any {
	tools := []map[string]any{}
	for _, t := range values {
		// The subcommand is the tool's fixed argument, not just its name:
		// ToolSpec.buildArgs starts from ToolSpec.Args, so a tool with no
		// args would run the bare command — every tool the same command
		// (Codex review, PR #2565).
		tools = append(tools, map[string]any{"name": t, "args": []string{t}})
	}
	return tools
}

// adapterAdd declares a new adapter through the daemon, the way servers
// add does: the write lands in the declaration file the running daemon
// reads and the reload makes it callable at once. The tools named here
// are a starting declaration; the file is the editable source, and
// `mcpx adapter check` validates whatever it grows into.
func (a *App) adapterAdd(ctx context.Context, args []string) error {
	fs := newFlagSet("adapter")
	var after []string
	for i, v := range args {
		if v == "--" {
			args, after = args[:i], args[i+1:]
			break
		}
	}
	args = hoistFlags(args, adapterAddValuedFlags)
	runtime := fs.String("runtime", "", "the interpreter a script command needs: bash, js, ts or py")
	desc := fs.String("description", "", "one line about what it wraps")
	tool := newRepeated()
	fs.Var(tool, "tool", "a subcommand worth exposing (repeatable)")
	replace := fs.Bool("replace", false, "overwrite a declaration of the same name")
	if err := parseFlags(a, fs, args); err != nil {
		return err
	}
	if fs.NArg() == 0 {
		return fmt.Errorf("usage: mcpx adapter add <name> [--tool <subcommand>]... -- <command> [args...]")
	}
	name := fs.Arg(0)
	rest := append(fs.Args()[1:], after...)
	if len(rest) == 0 {
		return fmt.Errorf("an adapter needs a command after --")
	}
	tools := adapterToolEntries(tool.values)
	body := map[string]any{
		"name": name, "command": rest[0], "replace": *replace, "tools": tools,
	}
	if len(rest) > 1 {
		body["args"] = rest[1:]
	}
	if *runtime != "" {
		body["runtime"] = *runtime
	}
	if *desc != "" {
		body["description"] = *desc
	}
	c, err := a.ensure(ctx)
	if err != nil {
		return err
	}
	raw, err := c.Do(ctx, http.MethodPost, "/v1/adapters", mustMarshal(body))
	if err != nil {
		return err
	}
	var out map[string]any
	if err := json.Unmarshal(raw, &out); err != nil {
		return err
	}
	if a.JSON {
		return a.out(out)
	}
	fmt.Printf("added adapter %s to %v; it is live now\n", name, out["written"])
	return nil
}

// adapterRemove removes a declaration through the daemon, mirroring
// servers remove: the file that defines it is the one edited.
func (a *App) adapterRemove(ctx context.Context, args []string) error {
	fs := newFlagSet("adapter")
	if err := parseFlags(a, fs, args); err != nil {
		return err
	}
	if fs.NArg() == 0 {
		return fmt.Errorf("usage: mcpx adapter remove <name>")
	}
	name := fs.Arg(0)
	c, err := a.ensure(ctx)
	if err != nil {
		return err
	}
	raw, err := c.Do(ctx, http.MethodDelete, "/v1/adapters/"+urlEscape(name), nil)
	if err != nil {
		return err
	}
	var out map[string]any
	if err := json.Unmarshal(raw, &out); err != nil {
		return err
	}
	if a.JSON {
		return a.out(out)
	}
	fmt.Printf("removed adapter %s from %v; it is gone from the running daemon\n", name, out["written"])
	return nil
}
