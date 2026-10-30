// Package deps decides where a script's packages come from (#1913).
//
// The question is small and the answer is not, because a script has three
// possible homes. It may sit in a project with a manifest and an installed
// tree, it may be one of mcpx's own scripts with no project at all, or it may
// be handed to a daemon that shares no filesystem with the caller. The chain
// below is ordered so that the more specific claim wins: what the script
// itself pins, then what this run explicitly asked for, then the nearest
// project manifest, then mcpx's configuration directory, then the host.
//
// Two rules keep that from being a guessing game. A version conflict is
// reported rather than merged -- a pin that contradicts the project manifest
// is a question for the person who wrote the manifest, and silently replacing
// it makes the manifest a lie. And mcpx never installs into a user's project:
// a missing package fails the run unless auto-install was explicitly asked
// for, and even then the materialised tree lives in the exec workdir.
package deps

import (
	"os"
	"strconv"
	"strings"

	"github.com/dezren39/mcpx/internal/searchpath"
	"github.com/dezren39/mcpx/internal/settings"
)

// Source names where a script's packages may be found. It is the resolved
// deps.source setting, and it governs only the layers below an explicit
// request: a script's own pins and an explicit --package always apply.
type Source string

const (
	// SourceHost uses whatever the machine already provides, the way running
	// the script by hand would.
	SourceHost Source = "host"
	// SourceConfig uses only the packages declared in mcpx's configuration
	// directory, so a run is reproducible without the machine's own tree.
	SourceConfig Source = "config"
	// SourceCWD uses the project the script sits in.
	SourceCWD Source = "cwd"
	// SourceExplicit uses only what the caller asked for.
	SourceExplicit Source = "explicit"
	// SourceNone provides nothing, letting the runtime fail, which is what you
	// want when a missing package should be loud.
	SourceNone Source = "none"
)

// AutoInstall is the resolved deps.autoInstall setting: whether the runtime
// may fetch a missing package on its own.
type AutoInstall string

const (
	// AutoInstallOff fails the run on a missing package. It is the default
	// because fetching whatever a script asks for is how a pinned project
	// quietly stops being pinned.
	AutoInstallOff AutoInstall = "off"
	// AutoInstallFallback lets the runtime install only when nothing else
	// satisfied the specifier.
	AutoInstallFallback AutoInstall = "fallback"
	// AutoInstallAlways leaves the runtime's own resolution alone.
	AutoInstallAlways AutoInstall = "always"
)

// Config is the dependency configuration one run acts on: the four settings
// plus the resolved manifest search path.
//
// It is deliberately a value rather than a live view of the settings Set, so
// that the chain can be computed and reported without the rest of the program
// holding a Set, and so that the same Config answers a plugin and a CLI run
// identically.
type Config struct {
	// Source is where packages may be found, from deps.source.
	Source Source
	// Manifest names an explicit manifest file, from deps.manifest. Empty
	// means the manifest is discovered from the script's directory along
	// Paths.
	Manifest string
	// Packages are the explicit specifiers from deps.packages, each a bare
	// name or name@version.
	Packages []string
	// AutoInstall is the resolved deps.autoInstall setting.
	AutoInstall AutoInstall
	// Paths is the resolved paths.deps search path, nearest first. It is empty
	// when nothing was configured, which means discovery walks up from the
	// script as it would by hand.
	Paths searchpath.Resolved
}

// From reads the dependency configuration out of a resolved settings Set.
//
// Every accessor here is the reason the corresponding setting is declared:
// this is the one place that sees all three layers, so a value that reached
// the Set from a file, an environment variable or a flag is honoured
// identically.
func From(set *settings.Set) Config {
	wd, _ := os.Getwd()
	c := Config{
		Source:      Source(set.String("deps.source")),
		Manifest:    set.String("deps.manifest"),
		Packages:    set.List("deps.packages"),
		AutoInstall: AutoInstall(set.String("deps.autoInstall")),
		// No built-in list: with nothing configured, discovery walks up from
		// the script exactly as it would by hand, which is what an empty
		// search path means here.
		Paths: searchpath.Resolve(set.List("paths.deps"), settings.NullMarker,
			searchpath.Options{Dir: wd}),
	}
	return c
}

// Specifier is one package a script should be able to import: a name and,
// when it was pinned, the version it was pinned to.
type Specifier struct {
	Name    string
	Version string
}

// Parse splits a name@version specifier.
//
// The separator is the first @ that is not the leading one, not the last: a
// scope is written @scope/pkg, so the leading @ belongs to the name, and a
// version may itself contain one (npm:@scope/x@2.0). Splitting on the last @
// would read that version as "2.0" and leave the alias in the name.
func Parse(s string) Specifier {
	for i := 1; i < len(s); i++ {
		if s[i] == '@' {
			return Specifier{Name: s[:i], Version: s[i+1:]}
		}
	}
	return Specifier{Name: s}
}

// Conflict says an explicit specifier contradicts a version the project
// manifest already names.
//
// It is reported, never resolved: resolving it would mean picking one of two
// claims and calling the result the user's intent.
type Conflict struct {
	Name        string
	Explicit    string
	Manifest    string
	ManifestRef string
}

// Conflicts returns every explicit specifier whose pin contradicts want, the
// versions already known from the project manifest. want may be nil.
func Conflicts(packages []string, want map[string]string) []Conflict {
	if len(want) == 0 {
		return nil
	}
	var out []Conflict
	for _, p := range packages {
		spec := Parse(p)
		if spec.Version == "" {
			continue
		}
		have, ok := want[spec.Name]
		if !ok || have == spec.Version {
			continue
		}
		// A manifest declares ranges, not just pins: "^1.3.0" is a
		// claim that 1.3.0 satisfies, and reporting it as a conflict
		// would reject a run the manifest itself allows. Only a pin
		// the declared range does not satisfy contradicts it.
		if satisfiesRange(have, spec.Version) {
			continue
		}
		out = append(out, Conflict{
			Name:     spec.Name,
			Explicit: spec.Version,
			Manifest: have,
		})
	}
	return out
}

// satisfiesRange reports whether version satisfies the manifest's declared
// requirement, for the range forms a package.json actually uses: an exact
// pin, ^, ~, >=, >, <=, <, *, and space-separated conjunctions of them.
// Anything it cannot parse — a dist-tag like "latest", a URL, a file: path —
// is not a range it understands, and it answers false so the caller falls
// back to the textual comparison (a contradiction is reported, never waved
// through on a guess).
func satisfiesRange(requirement, version string) bool {
	want, wok := parseVersion(version)
	if !wok {
		return false
	}
	tokens := strings.Fields(strings.TrimSpace(requirement))
	if len(tokens) == 0 {
		return false
	}
	for _, tok := range tokens {
		if !satisfiesToken(tok, want) {
			return false
		}
	}
	return true
}

func satisfiesToken(tok string, v [3]int) bool {
	if tok == "*" || strings.EqualFold(tok, "x") || tok == "" {
		return true
	}
	op := ""
	rest := tok
	for _, p := range []string{">=", "<=", "^", "~", ">", "<", "="} {
		if strings.HasPrefix(rest, p) {
			op, rest = p, strings.TrimPrefix(rest, p)
			break
		}
	}
	base, ok := parseVersion(rest)
	if !ok {
		return false
	}
	cmp := compareVersion(v, base)
	switch op {
	case "", "=":
		return cmp == 0
	case ">":
		return cmp > 0
	case ">=":
		return cmp >= 0
	case "<":
		return cmp < 0
	case "<=":
		return cmp <= 0
	case "~":
		return cmp >= 0 && v[0] == base[0] && v[1] == base[1]
	case "^":
		if cmp < 0 {
			return false
		}
		// Standard caret semantics: the left-most non-zero component
		// of the base is the one that may not change.
		if base[0] > 0 {
			return v[0] == base[0]
		}
		if base[1] > 0 {
			return v[0] == 0 && v[1] == base[1]
		}
		return v[0] == 0 && v[1] == 0 && v[2] == base[2]
	}
	return false
}

// parseVersion reads a plain dotted version ("1.3", "v1.3.0", "1.3.0-beta")
// into its numeric components; a range or tag is not a version.
func parseVersion(s string) ([3]int, bool) {
	var out [3]int
	s = strings.TrimSpace(strings.TrimPrefix(s, "v"))
	if i := strings.IndexAny(s, "-+"); i >= 0 {
		s = s[:i]
	}
	parts := strings.Split(s, ".")
	if len(parts) > 3 {
		return out, false
	}
	for i, p := range parts {
		n, err := strconv.Atoi(p)
		if err != nil || n < 0 {
			return out, false
		}
		out[i] = n
	}
	return out, true
}

func compareVersion(a, b [3]int) int {
	for i := 0; i < 3; i++ {
		if a[i] != b[i] {
			if a[i] < b[i] {
				return -1
			}
			return 1
		}
	}
	return 0
}
