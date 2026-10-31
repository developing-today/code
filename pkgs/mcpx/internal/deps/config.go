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

// CompatiblePin reports whether version is an exact pin (a plain
// semver, not a range or a dist-tag) that satisfies the manifest's
// declared requirement. The runner's materialisation uses it to decide
// which of two agreeing declarations governs: a caller who pinned
// foo@1.3.0 against a manifest's ^1.0.0 asked for 1.3.0 exactly, and
// treating compatibility as identity would silently run a different
// version than the one pinned (Codex review, PR #2589). A pin that
// contradicts the requirement is not compatible -- that case is a
// conflict, reported by Conflicts before materialisation ever runs.
func CompatiblePin(requirement, version string) bool {
	if _, ok := parseSemver(version); !ok {
		return false
	}
	if strings.ContainsAny(version, " 	^~<>=|") {
		return false
	}
	return satisfiesRange(requirement, version)
}

// satisfiesRange reports whether version satisfies the manifest's declared
// requirement, for the range forms a package.json actually uses: an exact
// pin, ^, ~, >=, >, <=, <, *, and space-separated conjunctions of them.
// Anything it cannot parse — a dist-tag like "latest", a URL, a file: path —
// is not a range it understands, and it answers false so the caller falls
// back to the textual comparison (a contradiction is reported, never waved
// through on a guess).
func satisfiesRange(requirement, version string) bool {
	want, wok := parseSemver(version)
	if !wok {
		return false
	}
	tokens := strings.Fields(strings.TrimSpace(requirement))
	if len(tokens) == 0 {
		return false
	}
	if want.pre != "" {
		// npm's prerelease rule: a prerelease version satisfies a
		// range only when some comparator in it names the same
		// release tuple WITH a prerelease — ^1.3.0 does not admit
		// 1.4.0-beta.1, and a plain 1.3.0 pin does not admit
		// 1.3.0-beta.1. Stripping the suffix instead (the old
		// parse) made a prerelease indistinguishable from its
		// release, so contradictory pins waved through (Codex
		// review, PR #2589).
		admitted := false
		for _, tok := range tokens {
			_, rest := splitToken(tok)
			if base, ok := parseSemver(rest); ok && base.nums == want.nums && base.pre != "" {
				admitted = true
				break
			}
		}
		if !admitted {
			return false
		}
	}
	for _, tok := range tokens {
		if !satisfiesToken(tok, want) {
			return false
		}
	}
	return true
}

// splitToken separates a range token's comparison operator from its
// version part (">=1.3.0" -> ">=", "1.3.0"; a bare version has none).
func splitToken(tok string) (string, string) {
	for _, p := range []string{">=", "<=", "^", "~", ">", "<", "="} {
		if strings.HasPrefix(tok, p) {
			return p, strings.TrimPrefix(tok, p)
		}
	}
	return "", tok
}

func satisfiesToken(tok string, v semver) bool {
	if tok == "*" || strings.EqualFold(tok, "x") || tok == "" {
		return true
	}
	op, rest := splitToken(tok)
	base, ok := parseSemver(rest)
	if !ok {
		return false
	}
	cmp := compareSemver(v, base)
	switch op {
	case "", "=":
		// An abbreviated pin is a range at its written precision:
		// "1" admits any 1.x, "1.2" any 1.2.x (npm's desugaring);
		// only a fully written version is an exact pin.
		if base.arity >= 3 {
			return cmp == 0
		}
		if cmp < 0 {
			return false
		}
		if base.arity == 1 {
			return v.nums[0] == base.nums[0]
		}
		return v.nums[0] == base.nums[0] && v.nums[1] == base.nums[1]
	case ">":
		// An abbreviated bound desugars at its written precision:
		// >1 is >=2.0.0 and >1.2 is >=1.3.0 (npm) -- zero-filling
		// the bound to 1.0.0 / 1.2.0 would admit versions the
		// range excludes (foo: ">1" must not admit 1.0.1).
		if base.arity == 1 {
			return compareSemver(v, semver{nums: [3]int{base.nums[0] + 1, 0, 0}}) >= 0
		}
		if base.arity == 2 {
			return compareSemver(v, semver{nums: [3]int{base.nums[0], base.nums[1] + 1, 0}}) >= 0
		}
		return cmp > 0
	case ">=":
		return cmp >= 0
	case "<":
		return cmp < 0
	case "<=":
		// <=1 is <2.0.0 and <=1.2 is <1.3.0 (the same npm
		// desugaring), not <= of the zero-filled tuple.
		if base.arity == 1 {
			return compareSemver(v, semver{nums: [3]int{base.nums[0] + 1, 0, 0}}) < 0
		}
		if base.arity == 2 {
			return compareSemver(v, semver{nums: [3]int{base.nums[0], base.nums[1] + 1, 0}}) < 0
		}
		return cmp <= 0
	case "~":
		// ~1 is ~ at major precision: any 1.x. ~1.2 and ~1.2.3 pin
		// the minor as well.
		if base.arity <= 1 {
			return cmp >= 0 && v.nums[0] == base.nums[0]
		}
		return cmp >= 0 && v.nums[0] == base.nums[0] && v.nums[1] == base.nums[1]
	case "^":
		if cmp < 0 {
			return false
		}
		// Abbreviated carets stop at their written precision:
		// ^0.0 admits any 0.0.x (it is NOT ^0.0.0, which pins the
		// patch), and ^1 / ^0 behave as their full forms.
		if base.arity == 1 {
			return v.nums[0] == base.nums[0]
		}
		if base.arity == 2 {
			if base.nums[0] > 0 {
				return v.nums[0] == base.nums[0]
			}
			return v.nums[0] == 0 && v.nums[1] == base.nums[1]
		}
		// Standard caret semantics: the left-most non-zero component
		// of the base is the one that may not change.
		if base.nums[0] > 0 {
			return v.nums[0] == base.nums[0]
		}
		if base.nums[1] > 0 {
			return v.nums[0] == 0 && v.nums[1] == base.nums[1]
		}
		return v.nums[0] == 0 && v.nums[1] == 0 && v.nums[2] == base.nums[2]
	}
	return false
}

// semver is a parsed version: the numeric release tuple plus the
// prerelease identifiers, preserved rather than stripped — a
// prerelease is a DIFFERENT version from its release (1.3.0-beta.1
// precedes 1.3.0), and conflict checking that cannot see the
// difference cannot report the contradiction. Build metadata (+...)
// is not part of precedence and is dropped.
type semver struct {
	nums [3]int
	pre  string
	// arity is how many release components were actually written:
	// "~1" and "~1.0.0" zero-fill to the same tuple but are NOT the
	// same range — npm reads the abbreviated form at its written
	// precision (~1 admits any 1.x, ^0.0 admits any 0.0.x), so the
	// count must survive parsing for the bounds to come out right
	// (Codex review, PR #2589).
	arity int
}

// parseSemver reads a plain dotted version ("1.3", "v1.3.0",
// "1.3.0-beta.1") into its release tuple and prerelease; a range or
// tag is not a version.
func parseSemver(s string) (semver, bool) {
	var out semver
	s = strings.TrimSpace(strings.TrimPrefix(s, "v"))
	if i := strings.Index(s, "+"); i >= 0 {
		s = s[:i]
	}
	if i := strings.Index(s, "-"); i >= 0 {
		out.pre = s[i+1:]
		s = s[:i]
		if out.pre == "" {
			return out, false
		}
	}
	parts := strings.Split(s, ".")
	if len(parts) > 3 {
		return out, false
	}
	out.arity = len(parts)
	for i, p := range parts {
		n, err := strconv.Atoi(p)
		if err != nil || n < 0 {
			return semver{}, false
		}
		out.nums[i] = n
	}
	return out, true
}

func compareSemver(a, b semver) int {
	for i := 0; i < 3; i++ {
		if a.nums[i] != b.nums[i] {
			if a.nums[i] < b.nums[i] {
				return -1
			}
			return 1
		}
	}
	switch {
	case a.pre == b.pre:
		return 0
	case a.pre == "":
		return 1 // a release follows its own prereleases
	case b.pre == "":
		return -1
	}
	return comparePrerelease(a.pre, b.pre)
}

// comparePrerelease orders two prerelease strings by semver's rules:
// dot-separated identifiers compare numerically when both are numeric
// (and numeric identifiers rank below alphanumeric ones), lexically
// otherwise; a shorter list that is a prefix of a longer one is lower.
func comparePrerelease(a, b string) int {
	as, bs := strings.Split(a, "."), strings.Split(b, ".")
	for i := 0; i < len(as) && i < len(bs); i++ {
		if as[i] == bs[i] {
			continue
		}
		an, aerr := strconv.Atoi(as[i])
		bn, berr := strconv.Atoi(bs[i])
		switch {
		case aerr == nil && berr == nil:
			if an < bn {
				return -1
			}
			return 1
		case aerr == nil:
			return -1
		case berr == nil:
			return 1
		case as[i] < bs[i]:
			return -1
		default:
			return 1
		}
	}
	switch {
	case len(as) < len(bs):
		return -1
	case len(as) > len(bs):
		return 1
	}
	return 0
}
