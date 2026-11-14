package mcpauth_test

// Tests for the #7948 identity failover. Every test names its oracle:
// the observable sequence or value that proves the behaviour, checked
// exactly, not by resemblance.

import (
	"context"
	"crypto/rand"
	"crypto/rsa"
	"crypto/x509"
	"encoding/base64"
	"encoding/json"
	"encoding/pem"
	"errors"
	"fmt"
	"net/http"
	"net/http/httptest"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"github.com/developing-today/code/pkgs/mcpx/internal/mcpauth"
)

func testSecrets(values map[string]string) mcpauth.SecretFunc {
	return func(name string) (string, error) {
		v, ok := values[name]
		if !ok {
			return "", fmt.Errorf("secret %s not found", name)
		}
		return v, nil
	}
}

func twoTokens() *mcpauth.Failover {
	return &mcpauth.Failover{Identities: []mcpauth.Identity{
		{Name: "app-one", Kind: "token", Secret: "TOK_ONE"},
		{Name: "user", Kind: "token", Secret: "TOK_USER"},
	}}
}

// Oracle: the exact sequence of (identity, token) pairs fn is called
// with, and the rotation log lines. A forced 429 on identity #1 and a
// forced secondary-limit 403 on a re-run must both end in success via
// identity #2, with the session-visible result being plain success.
func TestSoftBlockFailsOverToSecondIdentity(t *testing.T) {
	soft := map[string]*mcpauth.APIError{
		"429":       {Status: 429, Body: `{"message":"too many requests"}`},
		"secondary": {Status: 403, Body: `{"message":"You have exceeded a secondary rate limit. You have been submitted too quickly."}`},
	}
	for name, blockErr := range soft {
		t.Run(name, func(t *testing.T) {
			var calls []string
			var logs []string
			r := mcpauth.NewRotator(twoTokens(), func(f string, a ...any) {
				logs = append(logs, fmt.Sprintf(f, a...))
			})
			err := r.Do(context.Background(), testSecrets(map[string]string{
				"TOK_ONE": "token-one", "TOK_USER": "token-user",
			}), nil, func(id mcpauth.Identity, token string) error {
				calls = append(calls, id.Name+"="+token)
				if id.Name == "app-one" {
					return blockErr
				}
				return nil
			})
			if err != nil {
				t.Fatalf("session-visible result must be success, got %v", err)
			}
			want := []string{"app-one=token-one", "user=token-user"}
			if fmt.Sprint(calls) != fmt.Sprint(want) {
				t.Errorf("oracle: call sequence = %v, want %v", calls, want)
			}
			if len(logs) != 1 || !strings.Contains(logs[0], "app-one") || !strings.Contains(logs[0], "user") {
				t.Errorf("oracle: rotation log = %v, want one line naming app-one -> user", logs)
			}
			if got := r.Current().Name; got != "user" {
				t.Errorf("oracle: current identity = %q, want user", got)
			}
		})
	}
}

// Oracle: exactly one fn call and zero rotation log lines. Hard blocks
// (bad credentials, a marker-less 403 refusal, a validation error)
// surface with their real error and trigger no rotation.
func TestHardBlockSurfacesImmediatelyWithNoRotation(t *testing.T) {
	hard := map[string]*mcpauth.APIError{
		"401":            {Status: 401, Body: `{"message":"Bad credentials"}`},
		"403-no-markers": {Status: 403, Body: `{"message":"Resource not accessible by integration"}`},
		"404":            {Status: 404, Body: `{"message":"Not Found"}`},
		"422":            {Status: 422, Body: `{"message":"Validation Failed"}`},
	}
	for name, blockErr := range hard {
		t.Run(name, func(t *testing.T) {
			calls := 0
			logs := 0
			r := mcpauth.NewRotator(twoTokens(), func(f string, a ...any) { logs++ })
			err := r.Do(context.Background(), testSecrets(map[string]string{
				"TOK_ONE": "token-one", "TOK_USER": "token-user",
			}), nil, func(id mcpauth.Identity, token string) error {
				calls++
				return blockErr
			})
			var ae *mcpauth.APIError
			if !errors.As(err, &ae) || ae.Status != blockErr.Status {
				t.Fatalf("oracle: the real error must surface, got %v", err)
			}
			if calls != 1 {
				t.Errorf("oracle: fn calls = %d, want exactly 1 (no rotation)", calls)
			}
			if logs != 0 {
				t.Errorf("oracle: rotation log lines = %d, want 0", logs)
			}
		})
	}
}

// Oracle: a second Rotator constructed over the same state file (the
// "restart") reports the identical current identity and cooldown
// deadline as the first one held in memory.
func TestRotationStateSurvivesRestart(t *testing.T) {
	statePath := filepath.Join(t.TempDir(), "failover-state.json")
	cfg := twoTokens()
	cfg.StatePath = statePath
	cfg.CooldownSeconds = 600

	r1 := mcpauth.NewRotator(cfg, nil)
	err := r1.Do(context.Background(), testSecrets(map[string]string{
		"TOK_ONE": "token-one", "TOK_USER": "token-user",
	}), nil, func(id mcpauth.Identity, token string) error {
		if id.Name == "app-one" {
			return &mcpauth.APIError{Status: 429}
		}
		return nil
	})
	if err != nil {
		t.Fatal(err)
	}

	r2 := mcpauth.NewRotator(cfg, nil) // the restart: fresh process state
	if got := r2.Current().Name; got != "user" {
		t.Errorf("oracle: current after restart = %q, want user", got)
	}
	cd1, cd2 := r1.Cooldowns(), r2.Cooldowns()
	if len(cd2) != 1 || !cd2["app-one"].Equal(cd1["app-one"]) {
		t.Errorf("oracle: cooldowns after restart = %v, want identical to %v", cd2, cd1)
	}
}

// Oracle: when every identity soft-blocks, Do returns an error that
// wraps the last soft block and names the exhaustion -- the caller
// must see a rate-limit failure, not a silent success or a hang.
func TestAllIdentitiesSoftBlockedReturnsLastError(t *testing.T) {
	r := mcpauth.NewRotator(twoTokens(), nil)
	err := r.Do(context.Background(), testSecrets(map[string]string{
		"TOK_ONE": "token-one", "TOK_USER": "token-user",
	}), nil, func(id mcpauth.Identity, token string) error {
		return &mcpauth.APIError{Status: 429, Body: "slow down"}
	})
	var ae *mcpauth.APIError
	if !errors.As(err, &ae) || ae.Status != 429 {
		t.Fatalf("oracle: want wrapped 429, got %v", err)
	}
	if !strings.Contains(err.Error(), "all 2 identities") {
		t.Errorf("oracle: error should name exhaustion, got %v", err)
	}
}

// Oracle: the fake API receives a POST to the installation-token path
// carrying an RS256 JWT whose iss is the App ID and whose lifetime is
// under GitHub's ten-minute cap, and Mint returns exactly the token
// the API issued.
func TestMintInstallationToken(t *testing.T) {
	key, err := rsa.GenerateKey(rand.Reader, 2048)
	if err != nil {
		t.Fatal(err)
	}
	pemBytes := pem.EncodeToMemory(&pem.Block{
		Type:  "RSA PRIVATE KEY",
		Bytes: x509.MarshalPKCS1PrivateKey(key),
	})

	var gotPath, gotJWT string
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, req *http.Request) {
		gotPath = req.URL.Path
		gotJWT = strings.TrimPrefix(req.Header.Get("Authorization"), "Bearer ")
		w.Header().Set("Content-Type", "application/json")
		fmt.Fprint(w, `{"token":"installation-token-xyz","expires_at":"2030-01-01T00:00:00Z"}`)
	}))
	defer srv.Close()

	m := &mcpauth.Minter{BaseURL: srv.URL, Now: func() time.Time { return time.Unix(1700000000, 0) }}
	tok, exp, err := m.Mint(context.Background(), pemBytes, 5454422, 4242)
	if err != nil {
		t.Fatal(err)
	}
	if tok != "installation-token-xyz" {
		t.Errorf("oracle: token = %q, want the issued installation-token-xyz", tok)
	}
	if exp.Year() != 2030 {
		t.Errorf("oracle: expiry = %v, want the issued 2030 expiry", exp)
	}
	if want := "/app/installations/4242/access_tokens"; gotPath != want {
		t.Errorf("oracle: path = %q, want %q", gotPath, want)
	}
	parts := strings.Split(gotJWT, ".")
	if len(parts) != 3 {
		t.Fatalf("oracle: JWT has %d parts, want 3", len(parts))
	}
	claimsRaw, err := base64.RawURLEncoding.DecodeString(parts[1])
	if err != nil {
		t.Fatal(err)
	}
	var claims map[string]any
	if err := json.Unmarshal(claimsRaw, &claims); err != nil {
		t.Fatal(err)
	}
	if claims["iss"] != float64(5454422) {
		t.Errorf("oracle: JWT iss = %v, want the App ID 5454422", claims["iss"])
	}
	if lt := claims["exp"].(float64) - claims["iat"].(float64); lt <= 0 || lt > 660 {
		t.Errorf("oracle: JWT lifetime = %v seconds, want within GitHub's cap", lt)
	}
}

// Oracle: an App identity in the list mints through the Minter (the
// fake API is hit exactly once across two Do calls -- the token is
// cached), and the token fn receives is the minted one.
func TestAppIdentityMintsAndCaches(t *testing.T) {
	key, err := rsa.GenerateKey(rand.Reader, 2048)
	if err != nil {
		t.Fatal(err)
	}
	pemBytes := pem.EncodeToMemory(&pem.Block{
		Type:  "RSA PRIVATE KEY",
		Bytes: x509.MarshalPKCS1PrivateKey(key),
	})
	mints := 0
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, req *http.Request) {
		mints++
		fmt.Fprint(w, `{"token":"minted-1","expires_at":"2999-01-01T00:00:00Z"}`)
	}))
	defer srv.Close()

	cfg := &mcpauth.Failover{Identities: []mcpauth.Identity{
		{Name: "app-org", Kind: "app", Secret: "APP_KEY", AppID: 1, InstallationID: 2},
	}}
	r := mcpauth.NewRotator(cfg, nil)
	m := &mcpauth.Minter{BaseURL: srv.URL}
	secrets := testSecrets(map[string]string{"APP_KEY": string(pemBytes)})
	for i := 0; i < 2; i++ {
		var saw string
		err := r.Do(context.Background(), secrets, m, func(id mcpauth.Identity, token string) error {
			saw = token
			return nil
		})
		if err != nil {
			t.Fatal(err)
		}
		if saw != "minted-1" {
			t.Fatalf("oracle: fn token = %q, want minted-1", saw)
		}
	}
	if mints != 1 {
		t.Errorf("oracle: mint calls = %d, want 1 (second call served from cache)", mints)
	}
}

// Classifier table: the status/marker combinations the issue names,
// each with its expected block class.
func TestClassify(t *testing.T) {
	cases := []struct {
		err  *mcpauth.APIError
		want mcpauth.Block
	}{
		{&mcpauth.APIError{Status: 429}, mcpauth.BlockSoft},
		{&mcpauth.APIError{Status: 403, Body: "API rate limit exceeded"}, mcpauth.BlockSoft},
		{&mcpauth.APIError{Status: 403, Body: "You have been submitted too quickly"}, mcpauth.BlockSoft},
		{&mcpauth.APIError{Status: 403, RetryAfter: time.Minute}, mcpauth.BlockSoft},
		{&mcpauth.APIError{Status: 403, Body: "Forbidden"}, mcpauth.BlockHard},
		{&mcpauth.APIError{Status: 401, Body: "Bad credentials"}, mcpauth.BlockHard},
		{&mcpauth.APIError{Status: 404}, mcpauth.BlockHard},
		{&mcpauth.APIError{Status: 422}, mcpauth.BlockHard},
		{&mcpauth.APIError{Status: 500}, mcpauth.BlockHard},
	}
	for i, c := range cases {
		if got := mcpauth.Classify(c.err); got != c.want {
			t.Errorf("case %d (%v): Classify = %v, want %v", i, c.err, got, c.want)
		}
	}
}
