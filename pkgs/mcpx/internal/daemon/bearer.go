package daemon

import (
	"context"
	"crypto/subtle"
	"fmt"
	"net/http"
	"strings"

	"github.com/developing-today/code/pkgs/mcpx/internal/api"
	"github.com/developing-today/code/pkgs/mcpx/internal/settings"
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

// Auth modes (#13659, docs/daemon-auth.md section 2.5).
const (
	// AuthModeSetting is the registry path of the mode.
	AuthModeSetting = "auth.mode"
	// AuthModeOff is today's behaviour exactly: only transport.bearerToken,
	// when set, gates the daemon, and per-user keys are not consulted.
	AuthModeOff = "off"
	// AuthModePermissive honours a presented key, refuses a presented
	// mcpx key that is unknown or revoked, and still serves an
	// unauthenticated loopback caller.
	AuthModePermissive = "permissive"
	// AuthModeRequired refuses every HTTP caller without a valid key (or
	// the legacy token) with 401 and a WWW-Authenticate header. The unix
	// socket stays filesystem-permission authentication and needs no key.
	AuthModeRequired = "required"
)

// AuthMode resolves the configured mode; anything unset is off.
func AuthMode(set *settings.Set) string {
	if set == nil {
		return AuthModeOff
	}
	switch m := set.String("auth.mode"); m {
	case AuthModePermissive, AuthModeRequired:
		return m
	default:
		return AuthModeOff
	}
}

type keyIdentityCtx struct{}
type keyCredentialCtx struct{}

// WithKeyIdentity records the authenticated key on a request context.
func WithKeyIdentity(ctx context.Context, id KeyIdentity) context.Context {
	return context.WithValue(ctx, keyIdentityCtx{}, id)
}

// KeyIdentityFrom returns the key a request authenticated with, if any.
// A request served without a credential (auth off, permissive mode, or
// the owner's unix socket) carries none.
func KeyIdentityFrom(ctx context.Context) (KeyIdentity, bool) {
	id, ok := ctx.Value(keyIdentityCtx{}).(KeyIdentity)
	return id, ok
}

// WithKeyCredential carries an already-validated key through mcpx's local
// MCP-to-daemon call path so the nested daemon request resolves the same
// identity and ceiling. The value is never logged or returned to a caller.
func WithKeyCredential(ctx context.Context, credential string) context.Context {
	return context.WithValue(ctx, keyCredentialCtx{}, credential)
}

// KeyCredentialFrom returns the credential carried by a daemon-authenticated
// request, if the request presented a valid per-user key.
func KeyCredentialFrom(ctx context.Context) (string, bool) {
	credential, ok := ctx.Value(keyCredentialCtx{}).(string)
	return credential, ok
}

// legacyIdentity is the virtual key transport.bearerToken resolves to:
// the daemon owner with the full ceiling, labelled so audit can tell a
// legacy caller from a keyed one until the final migration slice drops
// the import.
func legacyIdentity() KeyIdentity {
	return KeyIdentity{KeyID: LegacyTokenLabel, User: "owner", Label: LegacyTokenLabel, Ceiling: CeilingFull}
}

// keyStore is the daemon's key store, or nil for a Server built without
// a state directory (tests that only want a gate).
func (s *Server) keyStore() *KeyStore { return s.keys }

// isOwnerSocket reports whether the request arrived on the daemon's unix
// socket, whose 0600 file permissions make the caller the owner.
func isOwnerSocket(r *http.Request) bool {
	p, ok := api.PrincipalFrom(r.Context())
	return ok && p.Surface == "unix"
}

// requireBearer wraps the daemon's whole HTTP surface (every /v1 and
// /mcp route, on every listener, since both handler chains pass
// through here).
//
// auth.mode off (the default) is the #1975 gate unchanged: with no token
// configured it is a pass-through, and with one every request on every
// listener must carry it. permissive and required add per-user keys
// from the state directory's key store; required refuses unauthenticated
// TCP callers, and in both the owner's unix socket needs no key.
func (s *Server) requireBearer(next http.Handler) http.Handler {
	return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		tok := BearerToken(s.set)
		mode := AuthMode(s.set)
		got, hasBearer := strings.CutPrefix(r.Header.Get("Authorization"), "Bearer ")
		presented := hasBearer && got != ""

		// The legacy shared token, honoured in every mode.
		if tok != "" && presented && subtle.ConstantTimeCompare([]byte(got), []byte(tok)) == 1 {
			next.ServeHTTP(w, r.WithContext(WithKeyIdentity(r.Context(), legacyIdentity())))
			return
		}

		if mode == AuthModeOff {
			if tok == "" {
				next.ServeHTTP(w, r)
				return
			}
			refuseUnauthenticated(w, BearerTokenSetting)
			return
		}

		if presented && strings.HasPrefix(got, KeyPrefix) {
			if rec, ok := s.keyStore().Lookup(got); ok {
				ctx := WithKeyIdentity(r.Context(), KeyIdentity{
					KeyID: rec.ID, User: rec.User, Label: rec.Label,
					Ceiling: rec.Ceiling, CapabilityRef: rec.CapabilityRef,
				})
				ctx = WithKeyCredential(ctx, got)
				next.ServeHTTP(w, r.WithContext(ctx))
				return
			}
			// A daemon key that does not resolve is refused in every
			// mode: an unknown or revoked key is never served as anonymous.
			refuseUnauthenticated(w, AuthModeSetting)
			return
		}

		// No usable credential. The owner's socket is the owner by
		// construction (design section 2.4).
		if isOwnerSocket(r) {
			next.ServeHTTP(w, r)
			return
		}
		if mode == AuthModeRequired || tok != "" {
			setting := AuthModeSetting
			if mode != AuthModeRequired {
				setting = BearerTokenSetting
			}
			refuseUnauthenticated(w, setting)
			return
		}
		next.ServeHTTP(w, r)
	})
}

// refuseUnauthenticated is the one refusal shape: 401, a Bearer
// challenge, and an error naming the setting that asked for it. No
// presented value is echoed.
func refuseUnauthenticated(w http.ResponseWriter, setting string) {
	w.Header().Set("WWW-Authenticate", "Bearer")
	writeErr(w, http.StatusUnauthorized, fmt.Errorf(
		"this daemon requires a bearer credential (%s)", setting))
}
