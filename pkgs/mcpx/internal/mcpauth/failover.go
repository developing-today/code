package mcpauth

// Failover across a list of GitHub identities for one server (#7948).
//
// The github namespace used to ride on a single user-token bucket: when
// that bucket hit a rate limit, every session's GitHub tools failed at
// once, even though the fleet owns several GitHub App identities with
// separate buckets. This file is the machinery that fixes that:
//
//   - Failover (config): an ordered list of Identity values. An identity
//     names its credential by SECRET NAME ONLY -- the value lives in the
//     daemon's secret store and is fetched through a SecretFunc at call
//     time. No secret value ever appears in config, logs, or this file.
//   - Classify: soft blocks (429, or 403 carrying rate-limit markers such
//     as the secondary-limit "submitted too quickly") rotate to the next
//     identity. Hard blocks (401, a marker-less 403, 404, 422) surface
//     immediately with their real error and never rotate.
//   - Rotator: owns the current identity and per-identity cooldowns,
//     persists them to a state file so they survive a server restart,
//     and logs every rotation with the identity name and the reason.
//   - Minter: exchanges a GitHub App's private key (by secret name) for
//     a short-lived installation token, so App identities need no stored
//     token at all.
//   - The single user token stays supported as the last list entry (kind
//     "token"), so a bad App configuration cannot take the namespace
//     down: rotation simply lands on it.

import (
	"context"
	"crypto"
	"crypto/rand"
	"crypto/rsa"
	"crypto/sha256"
	"crypto/x509"
	"encoding/base64"
	"encoding/json"
	"encoding/pem"
	"errors"
	"fmt"
	"io"
	"net/http"
	"os"
	"path/filepath"
	"strings"
	"sync"
	"time"
)

// Identity is one entry in a failover list.
type Identity struct {
	// Name identifies the identity in logs and rotation state. It is a
	// label, never a credential: "app-org", "app-personal", "user-token".
	Name string `json:"name"`

	// Kind is "app" (a GitHub App; Secret names the secret holding its
	// PEM private key, AppID and InstallationID say where to mint) or
	// "token" (Secret names the secret holding the token itself -- the
	// shape the single user token already has).
	Kind string `json:"kind"`

	// Secret is the NAME of the secret in the daemon's store, never a
	// value. For kind "app" the value is the App's PEM private key; for
	// kind "token" it is the token.
	Secret string `json:"secret"`

	// AppID and InstallationID are kind "app" only. They are public
	// identifiers, safe to keep in configuration.
	AppID          int64 `json:"appId,omitempty"`
	InstallationID int64 `json:"installationId,omitempty"`
}

// Failover configures identity failover for a server's auth block.
type Failover struct {
	// Identities is the ordered list to try. Rotation walks it in order
	// and wraps; the user token, when present, belongs last.
	Identities []Identity `json:"identities"`

	// StatePath is where the rotator persists its current identity and
	// cooldowns, so rotation state survives a server restart. Empty
	// keeps state in memory only.
	StatePath string `json:"statePath,omitempty"`

	// CooldownSeconds is how long a soft-blocked identity is skipped.
	// A response's Retry-After wins when it is longer. Zero means 60.
	CooldownSeconds int `json:"cooldownSeconds,omitempty"`
}

func (f *Failover) cooldown() time.Duration {
	if f == nil || f.CooldownSeconds <= 0 {
		return 60 * time.Second
	}
	return time.Duration(f.CooldownSeconds) * time.Second
}

// SecretFunc fetches a secret value by name: the daemon's store in
// production, a map in tests. It is the only way credential material
// enters this package, and only at call time.
type SecretFunc func(name string) (string, error)

// APIError is a GitHub API failure with the parts classification needs.
type APIError struct {
	Status     int
	Body       string
	RetryAfter time.Duration
}

func (e *APIError) Error() string {
	body := strings.TrimSpace(e.Body)
	if len(body) > 200 {
		body = body[:200] + "..."
	}
	if body == "" {
		return fmt.Sprintf("github api: status %d", e.Status)
	}
	return fmt.Sprintf("github api: status %d: %s", e.Status, body)
}

// Block is the classification of an APIError.
type Block int

const (
	// BlockHard surfaces immediately; no rotation.
	BlockHard Block = iota
	// BlockSoft rotates to the next identity.
	BlockSoft
)

// softMarkers are the body substrings that mark a 403 as a rate-limit
// (soft) block rather than a permissions (hard) one. GitHub's primary
// and secondary limit responses both carry "rate limit"; the secondary
// limit's tell is "submitted too quickly" (and its abuse-detection
// sibling). A 403 with none of these is a real refusal.
var softMarkers = []string{
	"rate limit",
	"submitted too quickly",
	"secondary rate",
	"abuse detection",
	"too many requests",
}

// Classify decides whether an error rotates (soft) or surfaces (hard).
func Classify(err error) Block {
	var ae *APIError
	if !errors.As(err, &ae) {
		return BlockHard
	}
	switch {
	case ae.Status == 429:
		return BlockSoft
	case ae.Status == 403:
		if ae.RetryAfter > 0 {
			return BlockSoft
		}
		body := strings.ToLower(ae.Body)
		for _, m := range softMarkers {
			if strings.Contains(body, m) {
				return BlockSoft
			}
		}
	}
	return BlockHard
}

// rotatorState is the persisted shape: which identity is current and
// when each blocked identity becomes usable again.
type rotatorState struct {
	Current   int                  `json:"current"`
	Cooldowns map[string]time.Time `json:"cooldowns"`
}

// Rotator walks a Failover list. Safe for concurrent use.
type Rotator struct {
	cfg   *Failover
	logf  func(format string, args ...any)
	now   func() time.Time
	mu    sync.Mutex
	state rotatorState
	// minted caches App installation tokens until near expiry, keyed by
	// identity name. Tokens are credentials: memory only, never the
	// state file.
	minted map[string]mintedToken
}

type mintedToken struct {
	token   string
	expires time.Time
}

// NewRotator loads any persisted state from the Failover's StatePath.
// logf receives one line per rotation (identity name + reason, never a
// credential); nil discards.
func NewRotator(cfg *Failover, logf func(format string, args ...any)) *Rotator {
	r := &Rotator{
		cfg:    cfg,
		logf:   logf,
		now:    time.Now,
		minted: map[string]mintedToken{},
	}
	r.state.Cooldowns = map[string]time.Time{}
	if cfg != nil && cfg.StatePath != "" {
		if b, err := os.ReadFile(cfg.StatePath); err == nil {
			var s rotatorState
			if json.Unmarshal(b, &s) == nil {
				r.state = s
				if r.state.Cooldowns == nil {
					r.state.Cooldowns = map[string]time.Time{}
				}
				if r.state.Current < 0 || r.state.Current >= len(cfg.Identities) {
					r.state.Current = 0
				}
			}
		}
	}
	return r
}

func (r *Rotator) log(format string, args ...any) {
	if r.logf != nil {
		r.logf(format, args...)
	}
}

// Current returns the identity new calls start from.
func (r *Rotator) Current() Identity {
	r.mu.Lock()
	defer r.mu.Unlock()
	if r.cfg == nil || len(r.cfg.Identities) == 0 {
		return Identity{}
	}
	return r.cfg.Identities[r.state.Current]
}

// Cooldowns returns a copy of the per-identity cooldown deadlines.
func (r *Rotator) Cooldowns() map[string]time.Time {
	r.mu.Lock()
	defer r.mu.Unlock()
	out := make(map[string]time.Time, len(r.state.Cooldowns))
	for k, v := range r.state.Cooldowns {
		out[k] = v
	}
	return out
}

func (r *Rotator) persistLocked() {
	if r.cfg == nil || r.cfg.StatePath == "" {
		return
	}
	b, err := json.Marshal(r.state)
	if err != nil {
		return
	}
	if dir := filepath.Dir(r.cfg.StatePath); dir != "" {
		_ = os.MkdirAll(dir, 0o755)
	}
	tmp := r.cfg.StatePath + ".tmp"
	if os.WriteFile(tmp, b, 0o600) == nil {
		_ = os.Rename(tmp, r.cfg.StatePath)
	}
}

// Token resolves the credential for one identity: a stored token for
// kind "token", a freshly minted (or cached) installation token for
// kind "app". The Minter may be nil when no identity is an App.
func (r *Rotator) Token(ctx context.Context, id Identity, secrets SecretFunc, minter *Minter) (string, error) {
	switch strings.ToLower(id.Kind) {
	case "token":
		if secrets == nil {
			return "", fmt.Errorf("identity %s: no secret store configured", id.Name)
		}
		return secrets(id.Secret)
	case "app":
		r.mu.Lock()
		if mt, ok := r.minted[id.Name]; ok && r.now().Before(mt.expires.Add(-5*time.Minute)) {
			r.mu.Unlock()
			return mt.token, nil
		}
		r.mu.Unlock()
		if secrets == nil || minter == nil {
			return "", fmt.Errorf("identity %s: app identity needs a secret store and a minter", id.Name)
		}
		key, err := secrets(id.Secret)
		if err != nil {
			return "", fmt.Errorf("identity %s: %w", id.Name, err)
		}
		tok, exp, err := minter.Mint(ctx, []byte(key), id.AppID, id.InstallationID)
		if err != nil {
			return "", fmt.Errorf("identity %s: %w", id.Name, err)
		}
		r.mu.Lock()
		r.minted[id.Name] = mintedToken{token: tok, expires: exp}
		r.mu.Unlock()
		return tok, nil
	}
	return "", fmt.Errorf("identity %s: unknown kind %q (app or token)", id.Name, id.Kind)
}

// Do runs fn with the current identity's token. On a soft block it
// rotates -- cooldown, log line, persisted state -- and retries with
// the next identity, so the caller sees success. On a hard block the
// real error returns at once and nothing rotates. When every identity
// is soft-blocked or cooling down, the last soft error returns.
//
// fn receives the identity and its token and does the actual call,
// returning nil on success or the call's error (an *APIError for API
// failures, so Classify can read the status and body).
func (r *Rotator) Do(ctx context.Context, secrets SecretFunc, minter *Minter, fn func(id Identity, token string) error) error {
	if r.cfg == nil || len(r.cfg.Identities) == 0 {
		return fmt.Errorf("failover: no identities configured")
	}
	n := len(r.cfg.Identities)
	r.mu.Lock()
	startIdx := r.state.Current
	r.mu.Unlock()
	var lastErr error
	tried := make(map[int]bool, n)
	for k := 0; k < n; k++ {
		idx := (startIdx + k) % n
		if tried[idx] {
			continue
		}
		id := r.cfg.Identities[idx]
		r.mu.Lock()
		until, cooling := r.state.Cooldowns[id.Name]
		r.mu.Unlock()
		if cooling && r.now().Before(until) {
			// Still serving a cooldown from an earlier block: skip it
			// unless it is the only identity not yet tried this call.
			remaining := false
			for j := 0; j < n; j++ {
				if !tried[j] && j != idx {
					remaining = true
				}
			}
			if remaining {
				continue
			}
		}
		tok, err := r.Token(ctx, id, secrets, minter)
		if err != nil {
			// A credential that cannot be produced (missing secret, a
			// mint the API refused) is this identity's hard failure:
			// surface it rather than silently skipping to a bucket the
			// operator did not choose as a fallback for config errors.
			return err
		}
		tried[idx] = true
		err = fn(id, tok)
		if err == nil {
			r.mu.Lock()
			r.state.Current = idx
			r.persistLocked()
			r.mu.Unlock()
			return nil
		}
		if Classify(err) == BlockHard {
			return err
		}
		// Soft block: cool this identity down and rotate.
		var ae *APIError
		_ = errors.As(err, &ae)
		cd := r.cfg.cooldown()
		if ae != nil && ae.RetryAfter > cd {
			cd = ae.RetryAfter
		}
		r.mu.Lock()
		r.state.Cooldowns[id.Name] = r.now().Add(cd)
		r.state.Current = (idx + 1) % n
		r.persistLocked()
		r.mu.Unlock()
		r.log("mcpauth: failover: identity %s soft-blocked (%v); rotating to %s, cooldown %s",
			id.Name, err, r.cfg.Identities[(idx+1)%n].Name, cd)
		lastErr = err
	}
	if lastErr != nil {
		return fmt.Errorf("failover: all %d identities soft-blocked: %w", n, lastErr)
	}
	return fmt.Errorf("failover: all %d identities are in cooldown", n)
}

// ResolveCurrent resolves the rotator's current identity into the
// credential a connection carries, so a caller that spawns a server
// per identity -- the pool resolving a server's auth at connection
// start -- hands it the CURRENT identity's credential, and a restart
// after rotation resumes on the identity the state file names. The
// token is returned as a bearer header and, for each name in
// envNames, as an environment value: a stdio server's auth block
// declares the env vars it reads (for github, the token variable the
// child consumes), and those vars carry the current identity's token
// instead of any single stored one.
func (r *Rotator) ResolveCurrent(ctx context.Context, envNames []string, secrets SecretFunc, minter *Minter) (Identity, *Resolved, error) {
	id := r.Current()
	tok, err := r.Token(ctx, id, secrets, minter)
	if err != nil {
		return id, nil, err
	}
	out := &Resolved{
		Headers: map[string]string{"Authorization": "Bearer " + tok},
		Query:   map[string]string{},
		Env:     map[string]string{},
	}
	for _, name := range envNames {
		out.Env[name] = tok
	}
	return id, out, nil
}

// Minter exchanges a GitHub App private key for an installation token.
type Minter struct {
	// BaseURL is the GitHub API root; empty means https://api.github.com.
	BaseURL string
	// HTTP is the client to use; nil means http.DefaultClient.
	HTTP *http.Client
	// Now overrides the clock in tests.
	Now func() time.Time
}

func (m *Minter) now() time.Time {
	if m != nil && m.Now != nil {
		return m.Now()
	}
	return time.Now()
}

func (m *Minter) base() string {
	if m != nil && m.BaseURL != "" {
		return strings.TrimRight(m.BaseURL, "/")
	}
	return "https://api.github.com"
}

func (m *Minter) client() *http.Client {
	if m != nil && m.HTTP != nil {
		return m.HTTP
	}
	return http.DefaultClient
}

// Mint signs a short-lived App JWT with keyPEM and exchanges it for an
// installation access token. It returns the token and its expiry. The
// key is used in memory only.
func (m *Minter) Mint(ctx context.Context, keyPEM []byte, appID, installationID int64) (string, time.Time, error) {
	jwt, err := signAppJWT(keyPEM, appID, m.now())
	if err != nil {
		return "", time.Time{}, err
	}
	url := fmt.Sprintf("%s/app/installations/%d/access_tokens", m.base(), installationID)
	req, err := http.NewRequestWithContext(ctx, http.MethodPost, url, nil)
	if err != nil {
		return "", time.Time{}, err
	}
	req.Header.Set("Authorization", "Bearer "+jwt)
	req.Header.Set("Accept", "application/vnd.github+json")
	resp, err := m.client().Do(req)
	if err != nil {
		return "", time.Time{}, err
	}
	defer resp.Body.Close()
	body, _ := io.ReadAll(io.LimitReader(resp.Body, 1<<20))
	if resp.StatusCode < 200 || resp.StatusCode >= 300 {
		return "", time.Time{}, &APIError{Status: resp.StatusCode, Body: string(body)}
	}
	var out struct {
		Token     string    `json:"token"`
		ExpiresAt time.Time `json:"expires_at"`
	}
	if err := json.Unmarshal(body, &out); err != nil {
		return "", time.Time{}, fmt.Errorf("mint: decode response: %w", err)
	}
	if out.Token == "" {
		return "", time.Time{}, fmt.Errorf("mint: response carried no token")
	}
	return out.Token, out.ExpiresAt, nil
}

// signAppJWT builds the RS256 JWT GitHub App authentication starts
// with: iss is the App ID, iat is backdated a minute for clock skew,
// exp is nine minutes out (GitHub's cap is ten).
func signAppJWT(keyPEM []byte, appID int64, now time.Time) (string, error) {
	block, _ := pem.Decode(keyPEM)
	if block == nil {
		return "", fmt.Errorf("mint: private key is not PEM")
	}
	var key *rsa.PrivateKey
	if parsed, err := x509.ParsePKCS1PrivateKey(block.Bytes); err == nil {
		key = parsed
	} else if parsed, err := x509.ParsePKCS8PrivateKey(block.Bytes); err == nil {
		var ok bool
		key, ok = parsed.(*rsa.PrivateKey)
		if !ok {
			return "", fmt.Errorf("mint: private key is not RSA")
		}
	} else {
		return "", fmt.Errorf("mint: cannot parse private key: %w", err)
	}
	enc := func(b []byte) string { return base64.RawURLEncoding.EncodeToString(b) }
	header := enc([]byte(`{"alg":"RS256","typ":"JWT"}`))
	claims, _ := json.Marshal(map[string]any{
		"iss": appID,
		"iat": now.Add(-60 * time.Second).Unix(),
		"exp": now.Add(9 * time.Minute).Unix(),
	})
	signing := header + "." + enc(claims)
	sum := sha256.Sum256([]byte(signing))
	sig, err := rsa.SignPKCS1v15(rand.Reader, key, crypto.SHA256, sum[:])
	if err != nil {
		return "", err
	}
	return signing + "." + enc(sig), nil
}
