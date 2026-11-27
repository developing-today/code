package daemon

import (
	"bytes"
	"context"
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"github.com/developing-today/code/pkgs/mcpx/internal/api"
	"github.com/developing-today/code/pkgs/mcpx/internal/settings"
)

// #13659: per-user daemon keys. Fixture secrets only, drawn from a
// deterministic reader so no test ever holds a real credential.

func fixtureRand(b byte) *bytes.Reader {
	return bytes.NewReader(bytes.Repeat([]byte{b}, keySecretBytes))
}

var fixtureNow = time.Date(2026, 11, 26, 0, 0, 0, 0, time.UTC)

func authSet(t *testing.T, mode, tok string) *settings.Set {
	t.Helper()
	sch, err := settings.New(settings.Registry())
	if err != nil {
		t.Fatal(err)
	}
	ov := map[string]string{}
	if mode != "" {
		ov[AuthModeSetting] = mode
	}
	if tok != "" {
		ov[BearerTokenSetting] = tok
	}
	return settings.NewSet(sch).WithOverrides(ov, "test")
}

func TestKeyStoreKeepsOnlyHashesAtRest(t *testing.T) {
	dir := t.TempDir()
	ks := OpenKeyStore(dir)
	secret, rec, err := ks.Create(KeySpec{User: "owner", Label: "laptop-cli", Ceiling: CeilingFull}, fixtureRand(7), fixtureNow)
	if err != nil {
		t.Fatal(err)
	}
	if !strings.HasPrefix(secret, KeyPrefix) {
		t.Fatalf("secret %q lacks the %s prefix", secret, KeyPrefix)
	}
	if rec.Hash != HashKey(secret) || rec.ID != rec.Hash[:keyIDLen] {
		t.Fatalf("record does not derive from the secret's hash: %+v", rec)
	}
	b, err := os.ReadFile(filepath.Join(dir, KeyStoreFile))
	if err != nil {
		t.Fatal(err)
	}
	if strings.Contains(string(b), secret) || strings.Contains(string(b), strings.TrimPrefix(secret, KeyPrefix)) {
		t.Fatal("auth.json holds the secret; only its hash may be stored")
	}
	st, err := os.Stat(filepath.Join(dir, KeyStoreFile))
	if err != nil {
		t.Fatal(err)
	}
	if st.Mode().Perm() != 0o600 {
		t.Fatalf("auth.json mode %v, want 0600", st.Mode().Perm())
	}
	var f keyFile
	if err := json.Unmarshal(b, &f); err != nil || len(f.Keys) != 1 {
		t.Fatalf("auth.json does not round-trip: %v %s", err, b)
	}
}

func TestKeyStoreDefaultsToTheDiscoveryCeiling(t *testing.T) {
	ks := OpenKeyStore(t.TempDir())
	_, rec, err := ks.Create(KeySpec{User: "door", Label: "door-machine"}, fixtureRand(1), fixtureNow)
	if err != nil {
		t.Fatal(err)
	}
	if rec.Ceiling != CeilingDiscovery {
		t.Fatalf("a key with no ceiling named got %q, want %q", rec.Ceiling, CeilingDiscovery)
	}
	for _, spec := range []KeySpec{{Label: "x"}, {User: "x"}} {
		if _, _, err := ks.Create(spec, fixtureRand(2), fixtureNow); err == nil {
			t.Errorf("create accepted %+v without a user and a label", spec)
		}
	}
}

func TestKeyStoreLookupAndRevokeWithoutRestart(t *testing.T) {
	ks := OpenKeyStore(t.TempDir())
	if _, ok := ks.Lookup(KeyPrefix + "nothing-stored-yet"); ok {
		t.Fatal("an empty store resolved a key")
	}
	secret, rec, err := ks.Create(KeySpec{User: "owner", Label: "laptop-cli", Ceiling: CeilingFull}, fixtureRand(3), fixtureNow)
	if err != nil {
		t.Fatal(err)
	}
	got, ok := ks.Lookup(secret)
	if !ok || got.ID != rec.ID || got.User != "owner" {
		t.Fatalf("lookup of a live key: %+v %v", got, ok)
	}
	if _, ok := ks.Lookup(secret + "x"); ok {
		t.Fatal("a near-miss secret resolved")
	}
	if _, ok := ks.Lookup(strings.TrimPrefix(secret, KeyPrefix)); ok {
		t.Fatal("a secret without its prefix resolved")
	}
	if err := ks.Revoke(rec.ID, fixtureNow); err != nil {
		t.Fatal(err)
	}
	if _, ok := ks.Lookup(secret); ok {
		t.Fatal("a revoked key still resolves")
	}
	// A second store instance (another reader of the same file) sees
	// the revocation too: the file, not process memory, is the record.
	if _, ok := OpenKeyStore(filepath.Dir(ks.Path())).Lookup(secret); ok {
		t.Fatal("a fresh reader resolves a revoked key")
	}
	if err := ks.Revoke("no-such-key", fixtureNow); err == nil {
		t.Fatal("revoking an unknown ID succeeded")
	}
	list, err := ks.List()
	if err != nil || len(list) != 1 || !list[0].Revoked || list[0].RevokedAt == nil {
		t.Fatalf("list after revoke: %+v %v", list, err)
	}
}

func TestKeyStoreFailsClosedOnAnUnreadableStore(t *testing.T) {
	dir := t.TempDir()
	if err := os.WriteFile(filepath.Join(dir, KeyStoreFile), []byte("{not json"), 0o600); err != nil {
		t.Fatal(err)
	}
	if _, ok := OpenKeyStore(dir).Lookup(KeyPrefix + "anything"); ok {
		t.Fatal("a corrupt store resolved a key")
	}
}

func TestAuthModeResolves(t *testing.T) {
	for _, tc := range []struct{ in, want string }{
		{"", AuthModeOff}, {"off", AuthModeOff}, {"permissive", AuthModePermissive}, {"required", AuthModeRequired},
	} {
		if got := AuthMode(authSet(t, tc.in, "")); got != tc.want {
			t.Errorf("auth.mode %q resolved to %q, want %q", tc.in, got, tc.want)
		}
	}
	if AuthMode(nil) != AuthModeOff {
		t.Error("a nil set should resolve to off")
	}
}

// TestRequireBearerAcrossAuthModes is the gate's whole truth table:
// every mode, with and without the legacy token, on TCP and on the
// owner's socket, with each kind of credential.
func TestRequireBearerAcrossAuthModes(t *testing.T) {
	dir := t.TempDir()
	ks := OpenKeyStore(dir)
	live, liveRec, err := ks.Create(KeySpec{User: "owner", Label: "laptop-cli", Ceiling: CeilingFull, CapabilityRef: "grant-1"}, fixtureRand(9), fixtureNow)
	if err != nil {
		t.Fatal(err)
	}
	revoked, revRec, err := ks.Create(KeySpec{User: "door", Label: "door-machine"}, fixtureRand(8), fixtureNow)
	if err != nil {
		t.Fatal(err)
	}
	if err := ks.Revoke(revRec.ID, fixtureNow); err != nil {
		t.Fatal(err)
	}
	unknown := KeyPrefix + strings.Repeat("Z", 43)
	upstream := "sk-upstream-provider-key"

	tcp := api.TCPPrincipal("127.0.0.1:5555")
	sock := api.UnixPeerPrincipal(1000)

	type want struct {
		code  int
		keyID string // "" when the request must carry no identity
	}
	cases := []struct {
		name      string
		mode, tok string
		who       api.Principal
		header    string
		want      want
	}{
		// off: the #1975 gate, unchanged; keys are not consulted.
		{"off, tcp, nothing", "", "", tcp, "", want{200, ""}},
		{"off, tcp, live key ignored", "", "", tcp, "Bearer " + live, want{200, ""}},
		{"off, tcp, upstream key passes through", "", "", tcp, "Bearer " + upstream, want{200, ""}},
		{"off+token, tcp, nothing", "", fixtureToken, tcp, "", want{401, ""}},
		{"off+token, socket, nothing", "", fixtureToken, sock, "", want{401, ""}},
		{"off+token, tcp, token", "", fixtureToken, tcp, "Bearer " + fixtureToken, want{200, LegacyTokenLabel}},
		{"off+token, tcp, live key is not the token", "", fixtureToken, tcp, "Bearer " + live, want{401, ""}},
		// permissive: keys honoured, anonymous loopback still served.
		{"permissive, tcp, nothing", "permissive", "", tcp, "", want{200, ""}},
		{"permissive, tcp, live key", "permissive", "", tcp, "Bearer " + live, want{200, liveRec.ID}},
		{"permissive, tcp, revoked key", "permissive", "", tcp, "Bearer " + revoked, want{401, ""}},
		{"permissive, tcp, unknown key", "permissive", "", tcp, "Bearer " + unknown, want{401, ""}},
		{"permissive, tcp, non-mcpx header is anonymous", "permissive", "", tcp, "Bearer " + upstream, want{200, ""}},
		{"permissive+token, tcp, nothing", "permissive", fixtureToken, tcp, "", want{401, ""}},
		{"permissive+token, socket, nothing", "permissive", fixtureToken, sock, "", want{200, ""}},
		{"permissive+token, tcp, live key", "permissive", fixtureToken, tcp, "Bearer " + live, want{200, liveRec.ID}},
		// required: no valid credential, no service -- except the socket.
		{"required, tcp, nothing", "required", "", tcp, "", want{401, ""}},
		{"required, tcp, wrong scheme", "required", "", tcp, "Basic " + live, want{401, ""}},
		{"required, tcp, empty bearer", "required", "", tcp, "Bearer ", want{401, ""}},
		{"required, tcp, upstream key", "required", "", tcp, "Bearer " + upstream, want{401, ""}},
		{"required, tcp, revoked key", "required", "", tcp, "Bearer " + revoked, want{401, ""}},
		{"required, tcp, unknown key", "required", "", tcp, "Bearer " + unknown, want{401, ""}},
		{"required, tcp, live key", "required", "", tcp, "Bearer " + live, want{200, liveRec.ID}},
		{"required, socket, nothing", "required", "", sock, "", want{200, ""}},
		{"required, socket, revoked key still refused", "required", "", sock, "Bearer " + revoked, want{401, ""}},
		{"required+token, tcp, token", "required", fixtureToken, tcp, "Bearer " + fixtureToken, want{200, LegacyTokenLabel}},
		{"required+token, tcp, live key", "required", fixtureToken, tcp, "Bearer " + live, want{200, liveRec.ID}},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			var seen *KeyIdentity
			next := http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
				if id, ok := KeyIdentityFrom(r.Context()); ok {
					seen = &id
				}
				w.WriteHeader(http.StatusOK)
			})
			s := &Server{set: authSet(t, tc.mode, tc.tok), keys: ks}
			req := httptest.NewRequest(http.MethodPost, "/mcp", nil)
			req = req.WithContext(api.WithPrincipal(context.Background(), tc.who))
			if tc.header != "" {
				req.Header.Set("Authorization", tc.header)
			}
			rec := httptest.NewRecorder()
			s.requireBearer(next).ServeHTTP(rec, req)
			if rec.Code != tc.want.code {
				t.Fatalf("status %d, want %d (body %q)", rec.Code, tc.want.code, rec.Body.String())
			}
			if tc.want.code == http.StatusUnauthorized {
				if rec.Header().Get("WWW-Authenticate") != "Bearer" {
					t.Errorf("401 without a WWW-Authenticate: Bearer header")
				}
				for _, secret := range []string{live, revoked, unknown, fixtureToken, upstream} {
					if strings.Contains(rec.Body.String(), secret) {
						t.Errorf("401 body echoes a presented credential: %q", rec.Body.String())
					}
				}
				return
			}
			switch {
			case tc.want.keyID == "" && seen != nil:
				t.Errorf("request carries identity %+v, want none", *seen)
			case tc.want.keyID != "" && (seen == nil || seen.KeyID != tc.want.keyID):
				t.Errorf("request identity %+v, want key %s", seen, tc.want.keyID)
			}
			if seen != nil && seen.KeyID == liveRec.ID && (seen.User != "owner" || seen.CapabilityRef != "grant-1" || seen.Ceiling != CeilingFull) {
				t.Errorf("identity does not carry the key's record: %+v", *seen)
			}
		})
	}
}

// The identity the gate records is what the chat proxy reads to keep a
// daemon key out of the upstream Authorization header.
func TestKeyIdentityRoundTripsThroughTheContext(t *testing.T) {
	ctx := WithKeyIdentity(context.Background(), KeyIdentity{KeyID: "abc"})
	if _, keyed := KeyIdentityFrom(ctx); !keyed {
		t.Fatal("identity did not round-trip through the context")
	}
	if _, keyed := KeyIdentityFrom(context.Background()); keyed {
		t.Fatal("a bare context carries an identity")
	}
}
