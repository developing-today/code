// Package mcpauth resolves credentials for a server.
//
// The specification's position is short and worth repeating, because it is
// the opposite of what people assume:
//
//   - Authorization is OPTIONAL.
//   - stdio transports SHOULD NOT use OAuth. They "retrieve credentials from
//     the environment" -- which is to say, a child process is trusted because
//     you started it.
//   - HTTP transports SHOULD use OAuth 2.1 when the server is protected.
//
// So most servers need nothing. A local one inherits the environment. A
// remote one usually needs a token in a header, and only occasionally needs
// the full OAuth dance.
//
// This package covers the first two cases completely and gives the third a
// place to attach, because building OAuth before anything needs it would be
// guessing at a shape.
//
// Expansion contract for credential values (Token, Username, Password,
// header and query values, and non-empty Env values): only the braced
// shape ${VAR} is a reference and expands through the lookup. Every other
// $ is literal text, byte-for-byte — $VAR, $2, $$, a trailing $, and a
// ${...} whose contents are not a plain variable name all reach the
// server exactly as written, so a credential like "hunter$2" or
// "pa$$word" is never rewritten and never contributes a name to
// Resolved.Missing. To write a literal ${VAR}, double the dollar:
// $${VAR} renders the literal text ${VAR}. Missing names only variables
// that a genuine ${VAR} reference named and the lookup could not supply.
package mcpauth

import (
	"encoding/base64"
	"fmt"
	"os"
	"strings"
)

// Auth declares how to authenticate to one server.
type Auth struct {
	// Type is bearer, basic, header, query, env or none. Empty means none.
	Type string `json:"type,omitempty"`

	// Token is used by bearer. Expanded, so ${GITHUB_TOKEN} works and the
	// secret itself never has to be in the file. Only the braced ${VAR}
	// shape expands — see the package doc for the expansion contract,
	// including the $${VAR} escape for writing a literal ${VAR}.
	Token string `json:"token,omitempty"`

	// Username and Password are used by basic.
	Username string `json:"username,omitempty"`
	Password string `json:"password,omitempty"`

	// Header and Value are used by header, for the many services that
	// invented their own scheme before Bearer existed.
	Header string `json:"header,omitempty"`
	Value  string `json:"value,omitempty"`

	// Param and ParamValue are used by query, for the same reason.
	Param      string `json:"param,omitempty"`
	ParamValue string `json:"paramValue,omitempty"`

	// Env names variables to pass to a stdio server. The value may be
	// omitted, which means "take it from mcpx's own environment" -- the
	// common case, and the one that keeps a token out of the file entirely.
	Env map[string]string `json:"env,omitempty"`

	// OAuth configures the full flow, for a server that demands it.
	OAuth *OAuth `json:"oauth,omitempty"`
}

// OAuth is the 2.1 flow the specification describes for HTTP transports.
//
// Declared here so a configuration can express it and `mcpx doctor` can say
// what is missing. The flow itself needs a browser and a callback listener
// and is not yet implemented; a server requiring it reports that clearly
// rather than failing at the first request with a 401 nobody can interpret.
type OAuth struct {
	// ClientID may be a plain identifier or, preferably, an HTTPS URL
	// pointing at a Client ID Metadata Document.
	ClientID string `json:"clientId,omitempty"`
	// Scopes requested up front. The server's WWW-Authenticate challenge is
	// authoritative at runtime and may ask for more.
	Scopes []string `json:"scopes,omitempty"`
	// Resource is the canonical URI of the MCP server, required by RFC 8707
	// and sent whether or not the authorization server understands it.
	Resource string `json:"resource,omitempty"`
	// Redirect is where the authorization code comes back to.
	Redirect string `json:"redirect,omitempty"`
}

// Resolved is what a transport should actually apply.
type Resolved struct {
	Headers map[string]string
	Query   map[string]string
	Env     map[string]string
	// NeedsOAuth reports that this server is configured for a flow mcpx
	// cannot yet perform.
	NeedsOAuth bool
	// Missing names variables that were referenced and are not set, so the
	// failure can be reported before a request rather than as a 401.
	Missing []string
}

// Resolve turns a declaration into headers, query parameters and environment,
// reading the variables it names from the daemon's environment.
func (a *Auth) Resolve() (*Resolved, error) {
	return a.ResolveWith(os.Getenv)
}

// ResolveWith is Resolve reading each variable through get. A caller that
// holds a value in memory for a name, as the daemon does for a secret, answers
// for it here, so the credential a server is given is the one it was waiting
// for.
func (a *Auth) ResolveWith(get func(name string) string) (*Resolved, error) {
	out := &Resolved{
		Headers: map[string]string{},
		Query:   map[string]string{},
		Env:     map[string]string{},
	}
	if a == nil {
		return out, nil
	}

	// expand applies the expansion contract from the package doc: only
	// the braced ${VAR} shape is a reference. Every other $ is literal
	// text, and $${VAR} is the escape that writes a literal ${VAR}.
	expand := func(v string) string {
		if !strings.Contains(v, "$") {
			return v
		}
		var b strings.Builder
		b.Grow(len(v))
		for i := 0; i < len(v); {
			if v[i] != '$' {
				b.WriteByte(v[i])
				i++
				continue
			}
			rest := v[i:]
			if strings.HasPrefix(rest, "$${") {
				// The escape: $${VAR} writes the literal text ${VAR}.
				if _, n, ok := bracedName(rest[1:]); ok {
					b.WriteString(rest[1 : 1+n])
					i += 1 + n
					continue
				}
			}
			if strings.HasPrefix(rest, "${") {
				if name, n, ok := bracedName(rest); ok {
					got := get(name)
					if got == "" {
						out.Missing = append(out.Missing, name)
					}
					b.WriteString(got)
					i += n
					continue
				}
			}
			b.WriteByte('$')
			i++
		}
		return b.String()
	}

	for k, v := range a.Env {
		if v == "" {
			// An empty value means "pass mine through". That is the shape
			// that keeps a secret out of the configuration file, so it is
			// worth supporting explicitly rather than by accident.
			if got := get(k); got != "" {
				out.Env[k] = got
			} else {
				out.Missing = append(out.Missing, k)
			}
			continue
		}
		out.Env[k] = expand(v)
	}

	switch strings.ToLower(a.Type) {
	case "", "none":
		// Nothing, which is the common case and worth having a name for.

	case "bearer":
		token := expand(a.Token)
		if token != "" {
			out.Headers["Authorization"] = "Bearer " + token
		}

	case "basic":
		user, pass := expand(a.Username), expand(a.Password)
		if user != "" || pass != "" {
			out.Headers["Authorization"] = "Basic " +
				base64.StdEncoding.EncodeToString([]byte(user+":"+pass))
		}

	case "header":
		if a.Header == "" {
			return nil, fmt.Errorf(`auth type "header" needs a header name`)
		}
		if v := expand(a.Value); v != "" {
			out.Headers[a.Header] = v
		}

	case "query":
		if a.Param == "" {
			return nil, fmt.Errorf(`auth type "query" needs a param name`)
		}
		// Permitted here because plenty of services require it, but the
		// specification forbids access tokens in a query string and it is
		// worth knowing that is what you are doing.
		if v := expand(a.ParamValue); v != "" {
			out.Query[a.Param] = v
		}

	case "env":
		// Everything was handled above; the type exists so a reader can see
		// the intent rather than inferring it from an otherwise empty block.

	case "oauth":
		out.NeedsOAuth = true

	default:
		return nil, fmt.Errorf("no auth type %q; bearer, basic, header, query, env, oauth or none",
			a.Type)
	}

	if a.OAuth != nil {
		out.NeedsOAuth = true
	}
	return out, nil
}

// Describe summarises an auth block without revealing anything.
//
// Used by `mcpx config` and `mcpx doctor`, both of which people paste into
// issues.
func (a *Auth) Describe() string {
	if a == nil || a.Type == "" || strings.EqualFold(a.Type, "none") {
		if a != nil && len(a.Env) > 0 {
			return fmt.Sprintf("env (%d variables)", len(a.Env))
		}
		return "none"
	}
	switch strings.ToLower(a.Type) {
	case "bearer":
		return "bearer " + redactRef(a.Token)
	case "basic":
		return "basic " + redactRef(a.Username)
	case "header":
		return "header " + a.Header + ": " + redactRef(a.Value)
	case "query":
		return "query " + a.Param + "=" + redactRef(a.ParamValue)
	case "oauth":
		return "oauth 2.1 (not yet implemented)"
	}
	return a.Type
}

// redactRef shows a ${VAR} reference but never a literal.
//
// A reference is safe and useful to print: it tells the reader which variable
// to set. A literal is the secret itself, and the whole point of printing
// anything is that somebody will paste it somewhere.
//
// Only a value made up ENTIRELY of variable references is shown. A mixed
// value such as ${TOKEN_PREFIX}.private-secret-suffix expands (os.Expand)
// to a string whose literal half is secret, so showing any part of the raw
// text would print that half; it is redacted whole.
func redactRef(v string) string {
	v = strings.TrimSpace(v)
	if v == "" {
		return "(unset)"
	}
	if isPureRef(v) {
		return v
	}
	return "(literal, redacted)"
}

// isPureRef reports whether v consists solely of $VAR / ${VAR} references
// (the two forms os.Expand resolves), with no literal characters.
//
// The name scan mirrors os.Expand's own (getShellName), because the two
// must agree about where a reference ends: an unbraced name that starts
// with a shell special variable character -- a digit or one of * # $ @
// ! ? - -- is exactly ONE character long there, so $1privateSecret
// expands $1 and leaves privateSecret as literal text. A scan that let
// the name run on would call the whole value a reference and print a
// secret suffix.
func isPureRef(v string) bool {
	for i := 0; i < len(v); {
		if v[i] != '$' {
			return false
		}
		i++
		braced := i < len(v) && v[i] == '{'
		if braced {
			i++
		}
		start := i
		if !braced && i < len(v) && isShellSpecialVar(v[i]) {
			i++
		} else {
			for i < len(v) && (v[i] == '_' || '0' <= v[i] && v[i] <= '9' || 'a' <= v[i] && v[i] <= 'z' || 'A' <= v[i] && v[i] <= 'Z') {
				i++
			}
		}
		if i == start {
			return false
		}
		if braced {
			if i >= len(v) || v[i] != '}' {
				return false
			}
			i++
		}
	}
	return len(v) > 0
}

// bracedName parses a ${NAME} reference at the start of s, where NAME is
// letters, digits and underscores. It returns the name and the length of
// the whole reference. Anything else — an empty name, a name with other
// characters (such as a shell ${VAR:-default}), or a missing closing
// brace — is not a reference under the expansion contract and is left
// literal by the caller.
func bracedName(s string) (string, int, bool) {
	if !strings.HasPrefix(s, "${") {
		return "", 0, false
	}
	i := 2
	for i < len(s) && (s[i] == '_' || '0' <= s[i] && s[i] <= '9' || 'a' <= s[i] && s[i] <= 'z' || 'A' <= s[i] && s[i] <= 'Z') {
		i++
	}
	if i == 2 || i >= len(s) || s[i] != '}' {
		return "", 0, false
	}
	return s[2:i], i + 1, true
}

// isShellSpecialVar is os.Expand's set: the characters that, directly
// after $, form a complete one-character variable reference.
func isShellSpecialVar(c byte) bool {
	switch c {
	case '*', '#', '$', '@', '!', '?', '-':
		return true
	}
	return '0' <= c && c <= '9'
}
