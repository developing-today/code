package cli

import (
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"regexp"
	"sort"
	"strings"
	"sync"

	"github.com/dezren39/mcpx/internal/recipes"
	"github.com/dezren39/mcpx/internal/runner"
	"github.com/dezren39/mcpx/internal/scriptresolve"
	"github.com/dezren39/mcpx/internal/searchpath"
	"github.com/dezren39/mcpx/internal/settings"
	"github.com/dezren39/mcpx/internal/source"
)

// ScriptsDirNames are the per-project directories mcpx looks in for named
// scripts, most specific first. `.config/mcpx` wins over `.mcpx` because it is
// the more explicit spelling and nests with other tools' config.
// The list itself lives in internal/scriptresolve (#2168), shared with the
// daemon path.
var ScriptsDirNames = scriptresolve.DirNames

// ScriptsDirName is the directory mcpx suggests creating.
var ScriptsDirName = ScriptsDirNames[0]

// ScriptExt is the extension a named script must carry on disk.
const ScriptExt = scriptresolve.Ext

// ScriptExtensions are tried in order for a name given without one.
var ScriptExtensions = scriptresolve.Extensions

// scriptResolver builds the shared resolver from this process's settings.
//
// The resolved settings carry the configured paths.scripts list, wherever
// it was set. When there is no App -- or it configured nothing -- the
// resolver falls back to the MCPX_PATHS_SCRIPTS environment variable, which
// is also what the daemon's resolver sees.
func scriptResolver() scriptresolve.Resolver {
	var configured []string
	if plumbingApp != nil {
		configured = splitPathList(plumbingApp.Settings().String("paths.scripts"))
	}
	return scriptresolve.Resolver{
		Configured:   configured,
		AllowOverlap: allowOverlap(),
		WarnBoth:     warnBothScriptDirs,
	}
}

// scriptPath resolves the configured search path, splicing the built-in list
// wherever the user left a null.
func scriptPath() searchpath.Resolved {
	return scriptResolver().Path()
}

// scriptSearchDirs is the flat list, kept for the environment variable the
// runner passes to scripts.
func scriptSearchDirs() []string {
	return scriptResolver().SearchDirs()
}

// plumbingApp is the App whose settings the free functions in this file
// consult. Script resolution is reached from several places that do not carry
// an App, and threading one through every call site to read two booleans
// would be a worse trade than a package-level pointer set once at startup.
var plumbingApp *App

// SetPlumbingSource tells the script resolver which App to read settings from.
func SetPlumbingSource(a *App) { plumbingApp = a }

func plumbingBool(path string) bool {
	if plumbingApp == nil {
		// No App means no config file and no flags -- a daemon, or a test
		// calling in directly. The declared default is still the right
		// answer, and it used to return false regardless, which silently
		// inverted every switch whose default is on.
		return defaultPlumbing().Bool(path)
	}
	return plumbingApp.Plumbing(path)
}

var (
	defaultPlumbingOnce sync.Once
	defaultPlumbingSet  *settings.Set
)

func defaultPlumbing() *settings.Set {
	defaultPlumbingOnce.Do(func() {
		sch, err := settings.New(settings.Registry())
		if err != nil {
			panic("mcpx: settings registry is invalid: " + err.Error())
		}
		defaultPlumbingSet = settings.NewSet(sch)
	})
	return defaultPlumbingSet
}

func allowOverlap() bool { return plumbingBool("plumbing.allowTsJsOverlap") }

// plumbingSourceOptions builds source.Options from the plumbing settings.
//
// source.Options has always said in its doc comment that it comes from the
// plumbing settings; it did not. Every call site wrote the three booleans as
// literals, so plumbing.sourceDirAllowed, sourceDirRecursive and
// sourceProbePaths were three switches wired to nothing. The literals also
// disagreed with each other -- a launcher refused a directory while a phase
// accepted one -- which is the kind of difference a setting exists to make
// deliberate.
//
// Extensions stays a caller's argument: it is not a policy, it is what this
// particular call is willing to read.
func plumbingSourceOptions(dir string, exts []string) source.Options {
	return source.Options{
		Dir:        dir,
		AllowDir:   plumbingBool("plumbing.sourceDirAllowed"),
		Recursive:  plumbingBool("plumbing.sourceDirRecursive"),
		Probe:      plumbingBool("plumbing.sourceProbePaths"),
		Extensions: exts,
	}
}

// PlaceholderFiles lists every file on the placeholder search path.
//
// Empty by default: declaring placeholders is opt-in, because scanning every
// script directory for declarations would make an ordinary script's filename
// quietly meaningful.
func PlaceholderFiles() []string {
	if plumbingApp == nil {
		return nil
	}
	configured := splitPathList(plumbingApp.Settings().String("paths.placeholders"))
	if len(configured) == 0 {
		return nil
	}
	wd, _ := os.Getwd()
	resolved := searchpath.Resolve(configured, settings.NullMarker, searchpath.Options{
		Dir:           wd,
		Builtin:       builtinPlaceholderDirs(),
		RequireExists: true,
	})
	var out []string
	for _, e := range resolved.Entries {
		if e.IsFile {
			out = append(out, e.Path)
			continue
		}
		entries, err := os.ReadDir(e.Path)
		if err != nil {
			continue
		}
		for _, de := range entries {
			if de.IsDir() {
				continue
			}
			switch strings.ToLower(filepath.Ext(de.Name())) {
			case ".ts", ".js", ".mts", ".mjs":
				out = append(out, filepath.Join(e.Path, de.Name()))
			}
		}
	}
	sort.Strings(out)
	return out
}

func builtinPlaceholderDirs() []string {
	var out []string
	if wd, err := os.Getwd(); err == nil {
		out = append(out,
			filepath.Join(wd, ".config", "mcpx", "placeholders"),
			filepath.Join(wd, ".mcpx", "placeholders"))
	}
	if home, err := os.UserHomeDir(); err == nil {
		out = append(out, filepath.Join(home, ".config", "mcpx", "placeholders"))
	}
	return out
}

// splitPathList accepts either a JSON array, as a configuration file holds
// it, or a separator-joined string, as an environment variable must. "-" and
// "null" both stand for the built-in list, because one of them is what
// somebody will type. The implementation lives in internal/scriptresolve,
// shared with the daemon path (#2168).
func splitPathList(v string) []string {
	return scriptresolve.SplitPathList(v)
}

// looksLikePath reports whether an argument should be used verbatim rather
// than resolved as a script name.
func looksLikePath(arg string) bool {
	return scriptresolve.LooksLikePath(arg)
}

// resolveScript turns `mcpx run report` into a concrete file. A name that
// already looks like a path is returned unchanged so existing usage and
// absolute paths keep working. The rule itself lives in
// internal/scriptresolve, so the daemon resolves /v1/exec's file field
// exactly the same way (#2168).
func resolveScript(arg string) (string, error) {
	return scriptResolver().Resolve(arg)
}

// paramDeclHead matches the head of one `@param` declaration line, the
// same spelling internal/recipes parses. Only the head is needed here:
// whether a script declares any holes at all, not what they are.
var paramDeclHead = regexp.MustCompile(`^\s*(?://|#|\*)\s*@param\s+[A-Za-z_]`)

// declaresParams reports whether a script declares recipe holes. A bare
// `@name` in a body is not enough -- an ordinary script may contain one
// for its own reasons -- but a declaration is a recipe saying so.
func declaresParams(src string) bool {
	for _, line := range strings.Split(src, "\n") {
		if paramDeclHead.MatchString(line) {
			return true
		}
	}
	return false
}

// onScriptPath reports whether file lives in one of the script search
// directories, which is what makes it findable as a recipe by name.
func onScriptPath(file string) bool {
	abs, err := filepath.Abs(file)
	if err != nil {
		return false
	}
	dir := filepath.Dir(abs)
	for _, d := range scriptSearchDirs() {
		if ad, err := filepath.Abs(d); err == nil && ad == dir {
			return true
		}
	}
	return false
}

// refuseRecipeHoles stops `mcpx run <recipe>` before the runtime does.
//
// A recipe's @param holes are filled only by `recipes run`, which
// substitutes the values before anything executes. `run` executes the
// file as written, so a hole reaches the JavaScript parser untouched and
// the run dies with "SyntaxError: Expression expected" -- a message that
// names neither the hole nor the command that fills it (#276). Say which
// command fills them instead. A script that cannot be resolved or read
// is left alone: the run's own error paths report those.
func refuseRecipeHoles(arg string) error {
	file, err := resolveScript(arg)
	if err != nil {
		return nil
	}
	b, err := os.ReadFile(file)
	if err != nil {
		return nil
	}
	src := string(b)
	if !declaresParams(src) {
		return nil
	}
	name := strings.TrimSuffix(filepath.Base(file),
		filepath.Ext(file))
	rec := recipes.Parse(name, file, src)
	if len(rec.Placeholders) == 0 {
		return nil
	}
	var assign []string
	for _, p := range rec.Placeholders {
		if p.Required {
			assign = append(assign, p.Name+"=<value>")
		} else {
			assign = append(assign, "["+p.Name+"=<value>]")
		}
	}
	head := fmt.Sprintf("%s is a recipe, not a plain script: it declares @param holes (%s) "+
		"that `mcpx run` does not fill, so a hole would reach the runtime as-is and fail "+
		"with a syntax error.", name, placeholderNames(rec))
	if onScriptPath(file) {
		return fmt.Errorf("%s\nRun it with:\n  mcpx recipes run %s %s",
			head, name, strings.Join(assign, " "))
	}
	return fmt.Errorf("%s\nSave it as a recipe, then run it with values:\n"+
		"  mcpx recipes save %s %s\n  mcpx recipes run %s %s",
		head, name, file, name, strings.Join(assign, " "))
}

// ScriptEntry is one discoverable script.
type ScriptEntry struct {
	Name     string `json:"name"`
	Path     string `json:"path"`
	Dir      string `json:"dir"`
	Summary  string `json:"summary,omitempty"`
	Shadowed bool   `json:"shadowed,omitempty"`
}

// discoverScripts lists every named script on the search path. Scripts found
// in a nearer directory shadow ones with the same name further out; the
// shadowed entries are still returned so `mcpx scripts` can show them.
func discoverScripts() ([]ScriptEntry, error) {
	var out []ScriptEntry
	claimed := map[string]bool{}
	for _, dir := range scriptSearchDirs() {
		entries, err := os.ReadDir(dir)
		if err != nil {
			continue
		}
		names := make([]string, 0, len(entries))
		for _, e := range entries {
			if e.IsDir() || !strings.HasSuffix(e.Name(), ScriptExt) {
				continue
			}
			// The generated client lives beside the scripts that import it; it
			// is machinery, not something a user would run.
			if e.Name() == runner.ClientFileName {
				continue
			}
			names = append(names, e.Name())
		}
		sort.Strings(names)
		for _, n := range names {
			name := strings.TrimSuffix(n, ScriptExt)
			path := filepath.Join(dir, n)
			entry := ScriptEntry{Name: name, Path: path, Dir: dir, Summary: scriptSummary(path)}
			if claimed[name] {
				entry.Shadowed = true
			}
			claimed[name] = true
			out = append(out, entry)
		}
	}
	if len(out) == 0 {
		return nil, errors.New("no scripts found")
	}
	return out, nil
}

// scriptSummary reads the first line of leading comment from a script, so
// `mcpx scripts` can describe each one without opening it.
func scriptSummary(path string) string {
	f, err := os.Open(path)
	if err != nil {
		return ""
	}
	defer f.Close()
	buf := make([]byte, 4096)
	n, _ := f.Read(buf)
	for _, line := range strings.Split(string(buf[:n]), "\n") {
		line = strings.TrimSpace(line)
		if line == "" {
			continue
		}
		if s, ok := strings.CutPrefix(line, "//"); ok {
			return strings.TrimSpace(s)
		}
		if s, ok := strings.CutPrefix(line, "/**"); ok {
			return strings.TrimSpace(strings.TrimSuffix(s, "*/"))
		}
		if s, ok := strings.CutPrefix(line, "/*"); ok {
			return strings.TrimSpace(strings.TrimSuffix(s, "*/"))
		}
		return ""
	}
	return ""
}

var warnedScriptDirs sync.Map

// warnBothScriptDirs complains once per directory when a project carries both
// spellings. Silently preferring one hides scripts the author expected to run.
func warnBothScriptDirs(dir string) {
	if _, seen := warnedScriptDirs.LoadOrStore(dir, true); seen {
		return
	}
	fmt.Fprintf(os.Stderr,
		"mcpx: %s has both %s and %s; %s wins. Consolidate to one.\n",
		dir, ScriptsDirNames[0], ScriptsDirNames[1], ScriptsDirNames[0])
}
