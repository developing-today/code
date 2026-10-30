package daemon

import (
	"crypto/subtle"
	"fmt"
	"net/http"
	"strings"

	"github.com/dezren39/mcpx/internal/settings"
)

// Optional bearer-token auth for the daemon's HTTP surfaces (#1975).
//
// The daemon otherwise trusts any caller that can reach its socket or
// loopback port: the socket's file permissions are the only access
// control, and a TCP port has none. transport.bearerToken is the cheap,
// deliberate first step -- a single static shared token, not OAuth
// (#1950) and not per-server upstream auth (#1926).
//
// The token's value is a secret in everything but storage: it is never
// logged, never echoed in an error, and errors name only the setting.
// Comparison is constant-time so the check itself is not an oracle.

// BearerTokenSetting is the registry path of the token.
const BearerTokenSetting = "transport.bearerToken"

// MinBearerTokenLength is the shortest token the daemon accepts at
// startup. A static token is only as strong as its entropy; 32
// characters is the floor the issue set.
const MinBearerTokenLength = 32

// BearerToken resolves the configured token from a settings set, or ""
// when auth is off. First-party clients read the same setting through
// here so the daemon and its callers can never drift on where the
// token lives.
func BearerToken(set *settings.Set) string {
	if set == nil {
		return ""
	}
	return set.String("transport.bearerToken")
}

// ValidateBearerToken is the startup check: a token that is set but
// too short is a configuration error, reported with the setting name
// and the minimum -- never the value.
func ValidateBearerToken(set *settings.Set) error {
	tok := BearerToken(set)
	if tok != "" && len(tok) < MinBearerTokenLength {
		return fmt.Errorf("%s: token of %d characters is too short; at least %d characters required (the value is not shown)",
			BearerTokenSetting, len(tok), MinBearerTokenLength)
	}
	return nil
}

// requireBearer wraps the daemon's whole HTTP surface (every /v1 and
// /mcp route, on every listener, since both handler chains pass
// through here). With no token configured it is a pass-through: the
// daemon behaves exactly as it did before the setting existed.
func (s *Server) requireBearer(next http.Handler) http.Handler {
	return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		tok := BearerToken(s.set)
		if tok == "" {
			next.ServeHTTP(w, r)
			return
		}
		got, ok := strings.CutPrefix(r.Header.Get("Authorization"), "Bearer ")
		if !ok || subtle.ConstantTimeCompare([]byte(got), []byte(tok)) != 1 {
			w.Header().Set("WWW-Authenticate", "Bearer")
			writeErr(w, http.StatusUnauthorized, fmt.Errorf(
				"this daemon requires a bearer token (%s)", BearerTokenSetting))
			return
		}
		next.ServeHTTP(w, r)
	})
}
