// Package plugin implements mcpx's one plugin contract (#1870, Approach C):
// a plugin is an MCP server.
//
// A plugin is discovered, not configured one by one. Directories named by
// the paths.plugins setting are scanned at daemon start and on reload:
//
//   - A directory containing mcpx-plugin.json is a plugin. The manifest
//     names it, describes it, picks its namespace, gives the spawn recipe
//     (command/args, or url/transport for a remote plugin), states the
//     mcpx API/capability it requires, and may list hook subscriptions
//     (recorded, not interpreted here -- hooks semantics are #1873).
//   - Any other executable file directly inside a plugin directory is a
//     zero-manifest plugin: the file itself is the spawn recipe and the
//     manifest defaults are derived from its name.
//   - An executable named mcpx-plugin-<name> on PATH is likewise a
//     zero-manifest plugin named <name> -- the git/kubectl subcommand
//     convention, and what makes a shebang script a plugin: chmod +x it,
//     name it, and it loads.
//
// Discovery turns each plugin into an ordinary config.Server, and the CLI
// merges those servers into the daemon's config (augmentPlugins, beside
// augmentAdapters). Plugins therefore route through internal/pool and the
// servers path -- pooling, sharing/scope, idle reap, schema cache,
// `mcpx ls`, catalog, generated TypeScript, live add/remove via reload --
// and NOT through mcpserver.Extra, whose tools reach none of those.
//
// The back-channel is by construction: the generated server's environment
// carries MCPX_SOCKET (the daemon socket, when one is known) and
// MCPX_SESSION_ID, so a plugin process can call /v1 exactly as
// plugin/opencode and an exec'd script already do. Both variables are
// declared in internal/settings (scriptenv.go); this package declares no
// new variable.
package plugin

import (
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"sort"
	"strings"

	"github.com/dezren39/mcpx/internal/config"
)

// ManifestFile is the name of the file that makes a directory a plugin.
const ManifestFile = "mcpx-plugin.json"

// PathPrefix prefixes zero-manifest executables found on PATH.
const PathPrefix = "mcpx-plugin-"

// Manifest is one mcpx-plugin.json document.
type Manifest struct {
	// Name is the plugin's identity. Required in a manifest file; derived
	// from the directory or file name for zero-manifest plugins.
	Name string `json:"name"`
	// Description is surfaced by `mcpx ls` beside the namespace.
	Description string `json:"description,omitempty"`
	// Namespace overrides the generated TypeScript namespace. Empty means
	// the namespace derived from Name, as for any server.
	Namespace string `json:"namespace,omitempty"`
	// Command and Args are the spawn recipe for a local plugin: any
	// executable that speaks MCP on stdio. Relative commands in a
	// manifest directory resolve against that directory, so a manifest
	// can name the executable sitting beside it.
	Command string   `json:"command,omitempty"`
	Args    []string `json:"args,omitempty"`
	// URL and Transport declare a remote plugin instead of a spawn
	// recipe: an MCP server reached over http/sse, exactly as a remote
	// servers entry is.
	URL       string `json:"url,omitempty"`
	Transport string `json:"transport,omitempty"`
	// Env is added to the plugin process's environment. The back-channel
	// variables (MCPX_SOCKET, MCPX_SESSION_ID) are added by Server and
	// win over any values declared here.
	Env map[string]string `json:"env,omitempty"`
	// Requires states what the plugin needs from mcpx: an API version
	// and/or named capabilities. Recorded on the plugin and available to
	// callers; enforcement against the running daemon is a loader-policy
	// question this core deliberately leaves visible rather than silent.
	Requires *Requires `json:"requires,omitempty"`
	// Hooks lists the event kinds the plugin subscribes to. Recorded
	// only: privileged hook semantics are #1873, not this loader.
	Hooks []string `json:"hooks,omitempty"`
}

// Requires is a manifest's statement of the mcpx surface it needs.
type Requires struct {
	// API is the /v1 API version the plugin was written against.
	API string `json:"api,omitempty"`
	// Capabilities names individual capabilities the plugin needs.
	Capabilities []string `json:"capabilities,omitempty"`
}

// Plugin is one discovered plugin: its manifest (explicit or defaulted),
// where it was found, and whether it came from a manifest file.
type Plugin struct {
	Manifest
	// Dir is the plugin directory for manifest and in-directory
	// zero-manifest plugins; empty for PATH plugins.
	Dir string
	// Source identifies the discovery site: the manifest path, the
	// executable path, or "PATH" lookup result.
	Source string
	// ZeroManifest is true when no mcpx-plugin.json was involved and
	// every manifest value is a derived default.
	ZeroManifest bool
}

// Validate checks a manifest is loadable: an identity and exactly one way
// to reach the plugin (a spawn recipe or a remote URL).
func (m Manifest) Validate() error {
	if strings.TrimSpace(m.Name) == "" {
		return fmt.Errorf("plugin manifest has no name")
	}
	if m.URL == "" && strings.TrimSpace(m.Command) == "" {
		return fmt.Errorf("plugin %q declares neither a command nor a url; a plugin is an MCP server, so it needs a spawn recipe or a remote address", m.Name)
	}
	if m.URL != "" && m.Command != "" {
		return fmt.Errorf("plugin %q declares both a command and a url; pick one", m.Name)
	}
	return nil
}

// Discover scans plugin directories and PATH and returns every plugin it
// finds, in stable order (directory order, then name).
//
// A paths.plugins entry naming a file rather than a directory means
// exactly that file: a manifest file, or a single executable treated as
// a zero-manifest plugin. A named entry that does not exist is an error
// -- somebody wrote that path down on purpose -- while a directory that
// simply holds no plugins contributes none.
//
// Duplicate plugin names are an error naming both sources, following the
// adapter rule: two plugins fighting over one namespace is a config
// mistake to surface, not a race to win silently. Collisions with
// configured servers are the caller's check (it owns the config).
func Discover(dirs []string, pathEnv string) ([]Plugin, error) {
	var out []Plugin
	seen := map[string]string{}
	add := func(p Plugin) error {
		if err := p.Manifest.Validate(); err != nil {
			return fmt.Errorf("%s: %w", p.Source, err)
		}
		if prev, dup := seen[p.Name]; dup {
			return fmt.Errorf("two plugins named %q (%s and %s); rename one", p.Name, prev, p.Source)
		}
		seen[p.Name] = p.Source
		out = append(out, p)
		return nil
	}

	for _, d := range dirs {
		if d == "" {
			continue
		}
		// Absolute before anything is derived from it: a discovered
		// plugin's command and working directory are handed to a
		// transport that runs the command FROM that directory, so a
		// relative path would be resolved twice and point nowhere
		// (Codex P2 on #2481).
		if abs, aerr := filepath.Abs(d); aerr == nil {
			d = abs
		}
		info, err := os.Stat(d)
		if err != nil {
			return nil, fmt.Errorf("reading plugins from %s: %w", d, err)
		}
		if !info.IsDir() {
			p, ok, err := discoverFile(d)
			if err != nil {
				return nil, err
			}
			if ok {
				if err := add(p); err != nil {
					return nil, err
				}
			}
			continue
		}
		found, err := discoverDir(d)
		if err != nil {
			return nil, err
		}
		for _, p := range found {
			if err := add(p); err != nil {
				return nil, err
			}
		}
	}

	for _, p := range discoverPATH(pathEnv) {
		// A PATH plugin shadowed by a directory plugin of the same name
		// is not a duplicate to fight over: the explicit search path
		// wins, as it does for every other mcpx search path.
		if _, dup := seen[p.Name]; dup {
			continue
		}
		if err := add(p); err != nil {
			return nil, err
		}
	}

	sort.Slice(out, func(i, j int) bool { return out[i].Name < out[j].Name })
	return out, nil
}

// discoverFile treats one named file as a manifest or a lone executable.
func discoverFile(path string) (Plugin, bool, error) {
	if filepath.Base(path) == ManifestFile {
		m, err := readManifest(path)
		if err != nil {
			return Plugin{}, false, err
		}
		return Plugin{Manifest: m, Dir: filepath.Dir(path), Source: path}, true, nil
	}
	if isExecutable(path) {
		return zeroManifest(path, filepath.Dir(path)), true, nil
	}
	return Plugin{}, false, nil
}

// discoverDir finds the plugins in one directory: itself, if it carries a
// manifest, plus every plugin one level down -- subdirectories carrying a
// manifest, and bare executables. One level is the contract: a plugin
// directory is a flat collection, not a tree to walk.
func discoverDir(dir string) ([]Plugin, error) {
	var out []Plugin
	if _, err := os.Stat(filepath.Join(dir, ManifestFile)); err == nil {
		m, err := readManifest(filepath.Join(dir, ManifestFile))
		if err != nil {
			return nil, err
		}
		out = append(out, Plugin{Manifest: m, Dir: dir, Source: filepath.Join(dir, ManifestFile)})
		// A directory that is itself a plugin is not also a collection:
		// its other executables are the plugin's own files.
		return out, nil
	}
	entries, err := os.ReadDir(dir)
	if err != nil {
		return nil, fmt.Errorf("reading plugins from %s: %w", dir, err)
	}
	for _, e := range entries {
		full := filepath.Join(dir, e.Name())
		if e.IsDir() {
			mp := filepath.Join(full, ManifestFile)
			if _, err := os.Stat(mp); err != nil {
				continue
			}
			m, err := readManifest(mp)
			if err != nil {
				return nil, err
			}
			out = append(out, Plugin{Manifest: m, Dir: full, Source: mp})
			continue
		}
		if isExecutable(full) {
			out = append(out, zeroManifest(full, dir))
		}
	}
	return out, nil
}

// discoverPATH finds mcpx-plugin-<name> executables on PATH.
func discoverPATH(pathEnv string) []Plugin {
	var out []Plugin
	seenDirs := map[string]bool{}
	for _, dir := range filepath.SplitList(pathEnv) {
		if dir == "" || seenDirs[dir] {
			continue
		}
		seenDirs[dir] = true
		entries, err := os.ReadDir(dir)
		if err != nil {
			continue
		}
		if abs, aerr := filepath.Abs(dir); aerr == nil {
			dir = abs
		}
		for _, e := range entries {
			if e.IsDir() || !strings.HasPrefix(e.Name(), PathPrefix) {
				continue
			}
			full := filepath.Join(dir, e.Name())
			if !isExecutable(full) {
				continue
			}
			name := strings.TrimPrefix(e.Name(), PathPrefix)
			out = append(out, Plugin{
				Manifest:     Manifest{Name: name, Command: full},
				Source:       full,
				ZeroManifest: true,
			})
		}
	}
	return out
}

// zeroManifest builds the default manifest for a bare executable: it is
// its own spawn recipe, named for its file, described as what it is.
func zeroManifest(path, dir string) Plugin {
	name := filepath.Base(path)
	name = strings.TrimPrefix(name, PathPrefix)
	return Plugin{
		Manifest: Manifest{
			Name:        name,
			Description: "plugin discovered from " + path + " (no " + ManifestFile + ")",
			Command:     path,
		},
		Dir:          dir,
		Source:       path,
		ZeroManifest: true,
	}
}

// readManifest parses one mcpx-plugin.json. JSONC comments are stripped
// with the same helper config files use, so a manifest can be annotated
// like every other mcpx document.
func readManifest(path string) (Manifest, error) {
	b, err := os.ReadFile(path)
	if err != nil {
		return Manifest{}, fmt.Errorf("reading plugin manifest %s: %w", path, err)
	}
	var m Manifest
	if err := json.Unmarshal(config.StripJSONC(b), &m); err != nil {
		return Manifest{}, fmt.Errorf("%s is not a valid plugin manifest: %w", path, err)
	}
	// A manifest's relative command names a file beside the manifest:
	// "./weather" in a plugin directory means that directory's weather,
	// not whatever the daemon's working directory happens to hold.
	if m.Command != "" && !filepath.IsAbs(m.Command) && strings.ContainsRune(m.Command, filepath.Separator) {
		m.Command = filepath.Join(filepath.Dir(path), m.Command)
	}
	if m.Name == "" {
		m.Name = filepath.Base(filepath.Dir(path))
	}
	return m, nil
}

// isExecutable reports whether path is a regular file with any execute
// bit set. On a filesystem without execute bits this is simply false,
// and the plugin is not discovered -- the honest answer there.
func isExecutable(path string) bool {
	info, err := os.Stat(path)
	if err != nil || info.IsDir() {
		return false
	}
	return info.Mode().Perm()&0o111 != 0
}

// Server converts the plugin to the daemon's one internal representation
// of an MCP server, so the pool path treats it exactly like a configured
// server. socket is the daemon's unix socket path ("" when unknown or
// remote) and session is the session id plugin processes should present
// when calling back; both ride in the process environment as the
// back-channel, under any environment the manifest declared.
func (p Plugin) Server(socket, session string) *config.Server {
	env := map[string]string{}
	for k, v := range p.Env {
		env[k] = v
	}
	if socket != "" {
		env["MCPX_SOCKET"] = socket
	}
	if session != "" {
		env["MCPX_SESSION_ID"] = session
	}
	s := &config.Server{
		Name:      p.Name,
		Command:   p.Command,
		Args:      append([]string(nil), p.Args...),
		URL:       p.URL,
		Transport: p.Transport,
		Env:       env,
	}
	if p.Dir != "" && p.URL == "" {
		// A local plugin runs from its own directory, so relative paths
		// in its own configuration mean what its author meant.
		s.Cwd = p.Dir
	}
	ex := &config.Extras{Description: p.Description}
	if p.Namespace != "" {
		ex.Namespace = p.Namespace
	}
	s.Mcpx = ex
	return s
}
