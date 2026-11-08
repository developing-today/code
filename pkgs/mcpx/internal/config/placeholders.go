package config

import (
	"fmt"
	"sort"
	"strings"
)

// Spawn-time placeholders (decision 0004 §4, #960): at instance spawn
// only, ${mcpx.root} and ${mcpx.cwd} in a server's Args, Env values and
// Cwd are replaced with the instance's scope-resolved root and the
// creating caller's cwd. The stored and merged config is never touched:
// mcpx config output keeps showing the placeholders, and the effective
// (substituted) definition exists only for the life of one instance —
// it is also what pool.Identity hashes for that instance's era record,
// so two instances differing only by substituted root never share an
// era entry.
//
// Only the mcpx. namespace is mcpx's: every other ${...} form passes
// through verbatim, exactly as before. Placeholder names are validated
// at config load (ValidatePlaceholders, called from Resolve): an
// unknown ${mcpx.<name>} or a malformed ${mcpx. sequence is a config
// error naming the server and the field, not a surprise at spawn time.

const (
	placeholderRoot = "mcpx.root"
	placeholderCwd  = "mcpx.cwd"
)

// placeholderField is one spawn-definition field a placeholder scan or
// substitution walks: a display name for errors, plus the raw value.
type placeholderField struct {
	name  string
	value string
}

// spawnFields lists the fields decision 0004 §4 substitutes, in a
// deterministic order (env keys sorted) so errors are stable.
func (s *Server) spawnFields() []placeholderField {
	fields := make([]placeholderField, 0, len(s.Args)+len(s.Env)+1)
	for i, a := range s.Args {
		fields = append(fields, placeholderField{fmt.Sprintf("args[%d]", i), a})
	}
	keys := make([]string, 0, len(s.Env))
	for k := range s.Env {
		keys = append(keys, k)
	}
	sort.Strings(keys)
	for _, k := range keys {
		fields = append(fields, placeholderField{"env " + k, s.Env[k]})
	}
	if s.Cwd != "" {
		fields = append(fields, placeholderField{"cwd", s.Cwd})
	}
	return fields
}

// scanPlaceholders returns the placeholder names a value references, in
// order of appearance. A malformed ${mcpx. sequence (no closing brace,
// or a name that is not a plain dotted identifier) is an error: at load
// time that is a config error; the caller adds the server/field context.
func scanPlaceholders(value string) ([]string, error) {
	var names []string
	rest := value
	for {
		i := strings.Index(rest, "${mcpx.")
		if i < 0 {
			// A dangling "${mcpx" with no dot is not the mcpx namespace
			// token form (${mcpx.<name>}); it passes through like any
			// other non-mcpx text.
			return names, nil
		}
		tail := rest[i+2:] // starts at "mcpx."
		end := strings.IndexByte(tail, '}')
		if end < 0 {
			return nil, fmt.Errorf("malformed placeholder %q: missing closing }", rest[i:])
		}
		name := tail[:end]
		if name == "mcpx." || !validPlaceholderName(name) {
			return nil, fmt.Errorf("malformed placeholder ${%s}", name)
		}
		names = append(names, name)
		rest = tail[end+1:]
	}
}

// validPlaceholderName reports whether name is a plain dotted
// identifier (letters, digits, '_', '.', '-'), the only shape a
// ${mcpx.<name>} token may take.
func validPlaceholderName(name string) bool {
	if name == "" {
		return false
	}
	for _, r := range name {
		switch {
		case r >= 'a' && r <= 'z', r >= 'A' && r <= 'Z', r >= '0' && r <= '9':
		case r == '.' || r == '_' || r == '-':
		default:
			return false
		}
	}
	return true
}

// ValidatePlaceholders checks every ${mcpx.*} token in the server's
// spawn definition (Args, Env values, Cwd) at config load: unknown
// names and malformed sequences are errors naming the server and the
// field. Resolve calls it, so every load path is covered.
func ValidatePlaceholders(name string, s *Server) error {
	if s == nil {
		return nil
	}
	for _, f := range s.spawnFields() {
		refs, err := scanPlaceholders(f.value)
		if err != nil {
			return fmt.Errorf("server %q: %s: %w", name, f.name, err)
		}
		for _, ref := range refs {
			if ref != placeholderRoot && ref != placeholderCwd {
				return fmt.Errorf("server %q: %s: unknown placeholder ${%s}; want ${mcpx.root} or ${mcpx.cwd}",
					name, f.name, ref)
			}
		}
	}
	return nil
}

// substitutePlaceholders replaces the known placeholders in one field
// value. A reference to a placeholder whose value is empty fails
// loudly, naming the server and field: silently substituting an empty
// root or cwd is the failure mode decision 0004 §4 exists to remove.
func substitutePlaceholders(server, field, value, root, cwd string) (string, error) {
	refs, err := scanPlaceholders(value)
	if err != nil {
		// Unreachable for a config that passed ValidatePlaceholders at
		// load; kept loud rather than silently passing the raw text on.
		return "", fmt.Errorf("server %q: %s: %w", server, field, err)
	}
	out := value
	for _, ref := range refs {
		var replacement string
		switch ref {
		case placeholderRoot:
			replacement = root
		case placeholderCwd:
			replacement = cwd
		default:
			return "", fmt.Errorf("server %q: %s: unknown placeholder ${%s}; want ${mcpx.root} or ${mcpx.cwd}",
				server, field, ref)
		}
		if replacement == "" {
			what := "resolved root"
			if ref == placeholderCwd {
				what = "caller cwd"
			}
			return "", fmt.Errorf("server %q: %s: references ${%s} but the instance has no %s to substitute",
				server, field, ref, what)
		}
		out = strings.ReplaceAll(out, "${"+ref+"}", replacement)
	}
	return out, nil
}

// Effective returns the per-instance spawn definition: a copy of this
// resolved config with ${mcpx.root}/${mcpx.cwd} substituted in Args,
// Env values and Cwd (decision 0004 §4). The receiver — the stored,
// merged config — is left exactly as written. A definition with no
// placeholders yields a copy equal to the original.
func (r *Resolved) Effective(root, callerCwd string) (*Resolved, error) {
	srv := *r.Server
	out := *r
	out.Server = &srv
	argsCopied := false
	for i, a := range srv.Args {
		v, err := substitutePlaceholders(r.Name, fmt.Sprintf("args[%d]", i), a, root, callerCwd)
		if err != nil {
			return nil, err
		}
		if v != a {
			if !argsCopied {
				srv.Args = append([]string(nil), r.Args...)
				argsCopied = true
			}
			srv.Args[i] = v
		}
	}
	if len(srv.Env) > 0 {
		keys := make([]string, 0, len(srv.Env))
		for k := range srv.Env {
			keys = append(keys, k)
		}
		sort.Strings(keys)
		envCopied := false
		for _, k := range keys {
			v, err := substitutePlaceholders(r.Name, "env "+k, srv.Env[k], root, callerCwd)
			if err != nil {
				return nil, err
			}
			if v != srv.Env[k] {
				if !envCopied {
					cp := make(map[string]string, len(srv.Env))
					for ek, ev := range srv.Env {
						cp[ek] = ev
					}
					srv.Env = cp
					envCopied = true
				}
				srv.Env[k] = v
			}
		}
	}
	if srv.Cwd != "" {
		v, err := substitutePlaceholders(r.Name, "cwd", srv.Cwd, root, callerCwd)
		if err != nil {
			return nil, err
		}
		srv.Cwd = v
	}
	return &out, nil
}
