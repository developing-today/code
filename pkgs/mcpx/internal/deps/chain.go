package deps

import (
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"sort"
	"strings"

	"github.com/dezren39/mcpx/internal/searchpath"
)

// Manifest is a project's declaration of the packages its scripts may import.
type Manifest struct {
	// Path is the file the declaration was read from, absolute when the caller
	// gave an absolute directory.
	Path string
	// Dir is the directory the runtime should treat as the project.
	Dir string
	// Packages maps a package name to the version specifier the manifest names.
	Packages map[string]string
	// JSONC records that comments had to be stripped, so the CLI can say why
	// the file the user wrote is not the file the runtime saw.
	JSONC bool
	// Lockfiles are the lockfiles sitting beside the manifest. A present
	// lockfile is honoured frozen and never rewritten: the lockfile is the
	// record of what the project actually resolved to.
	Lockfiles []string
}

// lockfileNames are the lockfiles a runtime may honour in a project, in the
// order they are reported.
var lockfileNames = []string{
	"bun.lockb", "bun.lock", "package-lock.json", "npm-shrinkwrap.json",
	"pnpm-lock.yaml", "yarn.lock", "deno.lock",
}

// manifestNames are the manifest filenames accepted in a project directory,
// in the order they are preferred. A project holding both is not something to
// guess about, so the plain one wins and the .jsonc is the fallback.
var manifestNames = []string{"package.json", "package.jsonc"}

// ErrNoManifest says discovery found nothing, which is normal: a script need
// not live in a project.
var ErrNoManifest = errors.New("deps: no package manifest found")

// Find locates the manifest a run should use.
//
// An explicit Config.Manifest wins outright. Otherwise the search starts at
// startDir and walks up, because that is how a person finds their own
// project, and only then falls back to the configured search path -- a
// configured path is where to look when walking up found nothing, not a
// reason to ignore the project the script is sitting in.
func Find(cfg Config, startDir string) (*Manifest, error) {
	if cfg.Manifest != "" {
		return Load(cfg.Manifest)
	}
	if startDir != "" {
		if m, err := walkUp(startDir); err == nil {
			return m, nil
		} else if !errors.Is(err, ErrNoManifest) {
			return nil, err
		}
	}
	for _, e := range cfg.Paths.Entries {
		if e.IsFile {
			if m, err := Load(e.Path); err == nil {
				return m, nil
			} else if !errors.Is(err, ErrNoManifest) {
				return nil, err
			}
			continue
		}
		if m, err := LoadDir(e.Path); err == nil {
			return m, nil
		} else if !errors.Is(err, ErrNoManifest) {
			return nil, err
		}
	}
	return nil, ErrNoManifest
}

// walkUp looks for a manifest in dir and each parent, nearest first.
func walkUp(dir string) (*Manifest, error) {
	abs, err := filepath.Abs(dir)
	if err != nil {
		return nil, err
	}
	for {
		if m, err := LoadDir(abs); err == nil {
			return m, nil
		} else if !errors.Is(err, ErrNoManifest) {
			return nil, err
		}
		parent := filepath.Dir(abs)
		if parent == abs {
			return nil, ErrNoManifest
		}
		abs = parent
	}
}

// LoadDir reads the manifest in dir, if there is one.
func LoadDir(dir string) (*Manifest, error) {
	for _, name := range manifestNames {
		if m, err := Load(filepath.Join(dir, name)); err == nil {
			return m, nil
		} else if !errors.Is(err, ErrNoManifest) {
			return nil, err
		}
	}
	return nil, ErrNoManifest
}

// Load reads one manifest file.
func Load(path string) (*Manifest, error) {
	info, err := os.Stat(path)
	if err != nil {
		if os.IsNotExist(err) {
			return nil, ErrNoManifest
		}
		return nil, err
	}
	if info.IsDir() {
		return LoadDir(path)
	}
	raw, err := os.ReadFile(path)
	if err != nil {
		return nil, err
	}
	jsonc := strings.HasSuffix(path, ".jsonc")
	if jsonc {
		raw = StripJSONC(raw)
	}
	var doc struct {
		Dependencies    map[string]string `json:"dependencies"`
		DevDependencies map[string]string `json:"devDependencies"`
	}
	if err := json.Unmarshal(raw, &doc); err != nil {
		return nil, fmt.Errorf("deps: %s: %w", path, err)
	}
	pkgs := make(map[string]string, len(doc.Dependencies)+len(doc.DevDependencies))
	for name, want := range doc.DevDependencies {
		pkgs[name] = want
	}
	// A plain dependency is what the script may import at run time, so it
	// overrides a dev entry of the same name.
	for name, want := range doc.Dependencies {
		pkgs[name] = want
	}
	dir := filepath.Dir(path)
	return &Manifest{
		Path:      path,
		Dir:       dir,
		Packages:  pkgs,
		JSONC:     jsonc,
		Lockfiles: lockfilesIn(dir),
	}, nil
}

// lockfilesIn reports the lockfiles present in dir.
func lockfilesIn(dir string) []string {
	var found []string
	for _, name := range lockfileNames {
		p := filepath.Join(dir, name)
		if st, err := os.Stat(p); err == nil && !st.IsDir() {
			found = append(found, p)
		}
	}
	return found
}

// StripJSONC removes // and /* */ comments and trailing commas from a JSONC
// document, leaving a plain JSON document any runtime can read.
//
// It is a scanner rather than a regular expression because comments are not a
// lexical property of a line: a // inside a string is a url, and a /* inside a
// string is text. Getting that wrong would corrupt a dependency on a git url,
// which is exactly the kind of package a manifest with comments holds.
func StripJSONC(raw []byte) []byte {
	return stripTrailingCommas(stripComments(raw))
}

// stripComments removes // and /* */ comments. A comment marker inside a
// string is text, so this is a scanner and not a regular expression.
func stripComments(raw []byte) []byte {
	out := make([]byte, 0, len(raw))
	inString := false
	escaped := false
	for i := 0; i < len(raw); i++ {
		c := raw[i]
		if inString {
			out = append(out, c)
			switch {
			case escaped:
				escaped = false
			case c == '\\':
				escaped = true
			case c == '"':
				inString = false
			}
			continue
		}
		switch c {
		case '"':
			inString = true
			out = append(out, c)
		case '/':
			if i+1 < len(raw) && raw[i+1] == '/' {
				for i < len(raw) && raw[i] != '\n' {
					i++
				}
				if i < len(raw) {
					out = append(out, '\n')
				}
				continue
			}
			if i+1 < len(raw) && raw[i+1] == '*' {
				i += 2
				for i+1 < len(raw) && !(raw[i] == '*' && raw[i+1] == '/') {
					i++
				}
				i++
				continue
			}
			out = append(out, c)
		default:
			out = append(out, c)
		}
	}
	return out
}

// stripTrailingCommas removes commas that sit immediately before a closing
// brace or bracket, which JSONC allows and JSON does not. It runs after
// comments are gone, so a comment between the comma and the brace no longer
// hides one.
func stripTrailingCommas(raw []byte) []byte {
	out := make([]byte, 0, len(raw))
	inString := false
	escaped := false
	for i := 0; i < len(raw); i++ {
		c := raw[i]
		if inString {
			out = append(out, c)
			switch {
			case escaped:
				escaped = false
			case c == '\\':
				escaped = true
			case c == '"':
				inString = false
			}
			continue
		}
		if c == '"' {
			inString = true
			out = append(out, c)
			continue
		}
		if c == ',' {
			j := i + 1
			for j < len(raw) && (raw[j] == ' ' || raw[j] == '\t' || raw[j] == '\n' || raw[j] == '\r') {
				j++
			}
			if j < len(raw) && (raw[j] == '}' || raw[j] == ']') {
				continue
			}
		}
		out = append(out, c)
	}
	return out
}

// Package is one package a script may import, with where the requirement came
// from so a surprising version can be explained.
type Package struct {
	Name    string
	Version string
	// From is the layer that asked for it: "explicit", "project", "config" or
	// "host".
	From string
}

// Set is the resolved dependency set one run acts on.
type Set struct {
	Packages []Package
	// Manifest is the project manifest the run resolved against, if any.
	Manifest *Manifest
	// Conflicts are explicit pins that contradict the project manifest. They
	// are reported, never resolved.
	Conflicts []Conflict
	// Frozen is true when a lockfile is present, in which case the runtime is
	// asked to honour it and never rewrite it.
	Frozen bool
}

// Resolve computes the set from the chain: an explicit request, then the
// project manifest, then the configuration directory's own manifest, then the
// host.
//
// The host layer contributes nothing to the set: it is whatever the runtime
// already finds, and enumerating it would mean reading the machine's package
// tree, which is the cache's problem (#1908), not this one.
func Resolve(cfg Config, startDir string) (Set, error) {
	if cfg.Source == SourceNone {
		return Set{}, nil
	}
	var set Set
	seen := map[string]bool{}

	add := func(p Package) {
		if p.Name == "" {
			return
		}
		// A later, less specific layer never replaces a more specific one.
		if _, ok := seen[p.Name]; ok {
			return
		}
		seen[p.Name] = true
		set.Packages = append(set.Packages, p)
	}

	for _, spec := range cfg.Packages {
		s := Parse(spec)
		add(Package{Name: s.Name, Version: s.Version, From: "explicit"})
	}

	var manifest *Manifest
	if cfg.Source != SourceExplicit {
		m, err := Find(cfg, startDir)
		if err == nil {
			manifest = m
		} else if !errors.Is(err, ErrNoManifest) {
			return Set{}, err
		}
	}
	if manifest != nil {
		set.Manifest = manifest
		set.Frozen = len(manifest.Lockfiles) > 0
		set.Conflicts = Conflicts(cfg.Packages, manifest.Packages)
		for _, name := range sortedKeys(manifest.Packages) {
			add(Package{Name: name, Version: manifest.Packages[name], From: "project"})
		}
	}

	if cfg.Source == SourceConfig {
		if dir := ConfigDir(); dir != "" {
			if m, err := LoadDir(dir); err == nil {
				for _, name := range sortedKeys(m.Packages) {
					add(Package{Name: name, Version: m.Packages[name], From: "config"})
				}
			}
		}
	}
	return set, nil
}

// ConfigDir is mcpx's configuration directory: $XDG_CONFIG_HOME/mcpx, else
// ~/.config/mcpx. It is where the configuration layer's own packages live.
func ConfigDir() string {
	base := os.Getenv("XDG_CONFIG_HOME")
	if base == "" {
		home, err := os.UserHomeDir()
		if err != nil {
			return ""
		}
		base = filepath.Join(home, ".config")
	}
	return filepath.Join(base, "mcpx")
}

func sortedKeys(m map[string]string) []string {
	keys := make([]string, 0, len(m))
	for k := range m {
		keys = append(keys, k)
	}
	sort.Strings(keys)
	return keys
}

// Describe renders the set for a person asking where a script's packages came
// from, which is the question behind every "why is this version wrong".
func (s Set) Describe() string {
	if len(s.Packages) == 0 {
		return "no packages resolved"
	}
	var b strings.Builder
	for _, p := range s.Packages {
		v := p.Version
		if v == "" {
			v = "(host)"
		}
		fmt.Fprintf(&b, "%s@%s from %s\n", p.Name, v, p.From)
	}
	if len(s.Conflicts) > 0 {
		b.WriteString("conflicts:\n")
		for _, c := range s.Conflicts {
			fmt.Fprintf(&b, "  %s: explicit %s contradicts manifest %s\n", c.Name, c.Explicit, c.Manifest)
		}
	}
	if s.Frozen {
		b.WriteString("lockfile present: honoured frozen\n")
	}
	return b.String()
}

// SearchPath is the resolved paths.deps, exposed so the CLI can print what was
// searched when discovery found nothing.
func SearchPath(cfg Config) searchpath.Resolved { return cfg.Paths }
