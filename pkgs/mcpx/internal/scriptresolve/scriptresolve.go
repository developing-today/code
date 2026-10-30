// Package scriptresolve is the one script-resolution rule for every mcpx
// surface (#2168, split of #1919 part 4).
//
// Before this package, resolveScript lived in internal/cli: `mcpx run report`
// resolved a bare name against the script search path, while POST /v1/exec
// passed its file field through as a raw path on the daemon's filesystem —
// so a name meant a search on one surface and a literal path on the other,
// and binary-less callers (the opencode plugin) could only name absolute or
// cwd-relative paths. The rule now lives here, callable from both the CLI
// and internal/execsvc, so the two cannot fork.
package scriptresolve

import (
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"strings"

	"github.com/dezren39/mcpx/internal/recipes"
	"github.com/dezren39/mcpx/internal/searchpath"
	"github.com/dezren39/mcpx/internal/settings"
)

// DirNames are the per-project directories mcpx looks in for named scripts,
// most specific first. `.config/mcpx` wins over `.mcpx` because it is the
// more explicit spelling and nests with other tools' config.
var DirNames = recipes.DirNames

// DirName is the directory mcpx suggests creating.
var DirName = DirNames[0]

// Ext is the extension a named script must carry on disk.
const Ext = ".ts"

// Extensions are tried in order for a name given without one. The order is
// deliberate: a project holding both foo.ts and foo.js is almost always
// compiling one into the other, and the source is what someone means to run.
var Extensions = []string{".ts", ".mts", ".js", ".mjs"}

// SplitPathList accepts either a JSON array, as a configuration file holds
// it, or a separator-joined string, as an environment variable must. "-" and
// "null" both stand for the built-in list, because one of them is what
// somebody will type.
func SplitPathList(v string) []string {
	v = strings.TrimSpace(v)
	if v == "" {
		return nil
	}
	var parts []string
	if strings.HasPrefix(v, "[") {
		var arr []any
		if json.Unmarshal([]byte(v), &arr) == nil {
			for _, e := range arr {
				if e == nil {
					parts = append(parts, settings.NullMarker)
					continue
				}
				parts = append(parts, fmt.Sprint(e))
			}
			return parts
		}
	}
	for _, part := range strings.Split(v, string(os.PathListSeparator)) {
		part = strings.TrimSpace(part)
		switch part {
		case "":
		case "-", "null":
			parts = append(parts, settings.NullMarker)
		default:
			parts = append(parts, part)
		}
	}
	return parts
}

// Resolver resolves script names against one search path.
//
// Configured is the caller's resolved paths.scripts list, already split
// (see SplitPathList). When it is empty, the MCPX_PATHS_SCRIPTS environment
// variable is read instead: resolution happens on paths that do not always
// have a resolved settings set — notably inside the daemon, whose settings
// arrive through ExecService and are passed in as Configured when present.
//
// The paths.scripts scope question from #1919 is answered here: there is
// ONE setting, shared by the CLI and the daemon. The daemon resolves it
// from the same configuration file and environment it was started with;
// there is no daemon-specific variant, because two lists would make the
// same name resolve differently depending on who was asked — the exact
// divergence this package exists to remove.
type Resolver struct {
	// Configured is the split paths.scripts list; a settings.NullMarker
	// entry splices the built-in list in at that point.
	Configured []string
	// AllowOverlap permits a name that matches more than one extension.
	AllowOverlap bool
	// Dir is the working directory the search walks up from. Empty means
	// the process working directory.
	Dir string
	// WarnBoth, when set, is called once per directory that carries both
	// script-dir spellings. The CLI passes its stderr warning; the daemon
	// leaves it nil, because a package that prints is a package a daemon
	// cannot use.
	WarnBoth func(dir string)
}

func (r Resolver) dir() string {
	if r.Dir != "" {
		return r.Dir
	}
	wd, _ := os.Getwd()
	return wd
}

// BuiltinDirs is the built-in search list: every scripts directory from the
// working directory up to the filesystem root (nearest wins, so a repo
// overrides a parent), then the user directory. It mirrors how the config
// file is discovered, so a project can keep its scripts and its server list
// together.
func (r Resolver) BuiltinDirs() []string {
	wd := r.dir()
	home, _ := os.UserHomeDir()
	dirs, both := recipes.Dirs(wd, home, os.Getenv("XDG_CONFIG_HOME"))
	if r.WarnBoth != nil {
		for _, dir := range both {
			r.WarnBoth(dir)
		}
	}
	return dirs
}

// Path resolves the configured search path, splicing the built-in list
// wherever the user left a null.
func (r Resolver) Path() searchpath.Resolved {
	configured := r.Configured
	if len(configured) == 0 {
		configured = SplitPathList(os.Getenv("MCPX_PATHS_SCRIPTS"))
	}
	return searchpath.Resolve(configured, settings.NullMarker, searchpath.Options{
		Dir:     r.dir(),
		Builtin: r.BuiltinDirs(),
	})
}

// SearchDirs is the flat list of directories searched, in order.
func (r Resolver) SearchDirs() []string {
	var out []string
	for _, e := range r.Path().Entries {
		out = append(out, e.Path)
	}
	return out
}

// LooksLikePath reports whether an argument should be used verbatim rather
// than resolved as a script name.
func LooksLikePath(arg string) bool {
	return strings.ContainsRune(arg, filepath.Separator) ||
		strings.HasSuffix(arg, Ext) ||
		strings.HasSuffix(arg, ".js") ||
		strings.HasSuffix(arg, ".mts") ||
		arg == "-"
}

// Resolve turns a bare script name (`report`) into a concrete file. A name
// that already looks like a path is returned unchanged so existing usage and
// absolute paths keep working. A miss is an error naming every directory
// searched — the same error on the CLI and on the daemon.
func (r Resolver) Resolve(arg string) (string, error) {
	if LooksLikePath(arg) {
		return arg, nil
	}
	path := r.Path()
	found, _, err := path.Find(arg, searchpath.FindOptions{
		Extensions:   Extensions,
		AllowOverlap: r.AllowOverlap,
	})
	if err != nil {
		return "", err
	}
	if found != "" {
		return found, nil
	}
	return "", fmt.Errorf("no script named %q; looked in:\n%s\nCreate one with:\n  mkdir -p %s && $EDITOR %s",
		arg, path.Describe(), DirName, filepath.Join(DirName, arg+Ext))
}
