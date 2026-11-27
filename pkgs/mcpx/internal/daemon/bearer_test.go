package daemon

import (
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"

	"github.com/developing-today/code/pkgs/mcpx/internal/settings"
)

// #1975: with transport.bearerToken set, every daemon request needs
// the token; unset, nothing changes. Fixture tokens only -- never a
// real credential.

const fixtureToken = "fixture-token-0123456789abcdef01" // exactly 32 chars

func bearerSet(t *testing.T, tok string) *settings.Set {
	t.Helper()
	sch, err := settings.New(settings.Registry())
	if err != nil {
		t.Fatal(err)
	}
	ov := map[string]string{}
	if tok != "" {
		ov[BearerTokenSetting] = tok
	}
	return settings.NewSet(sch).WithOverrides(ov, "test")
}

func TestRequireBearer(t *testing.T) {
	ok := http.HandlerFunc(func(w http.ResponseWriter, _ *http.Request) {
		w.WriteHeader(http.StatusOK)
	})
	cases := []struct {
		name   string
		set    string
		header string
		want   int
	}{
		{"unset, no header", "", "", http.StatusOK},
		{"unset, stray header ignored", "", "Bearer whatever", http.StatusOK},
		{"set, no header", fixtureToken, "", http.StatusUnauthorized},
		{"set, wrong token", fixtureToken, "Bearer wrong-token-0123456789abcdef0", http.StatusUnauthorized},
		{"set, wrong scheme", fixtureToken, "Basic " + fixtureToken, http.StatusUnauthorized},
		{"set, correct token", fixtureToken, "Bearer " + fixtureToken, http.StatusOK},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			s := &Server{set: bearerSet(t, tc.set)}
			req := httptest.NewRequest(http.MethodGet, "/v1/log", nil)
			if tc.header != "" {
				req.Header.Set("Authorization", tc.header)
			}
			rec := httptest.NewRecorder()
			s.requireBearer(ok).ServeHTTP(rec, req)
			if rec.Code != tc.want {
				t.Fatalf("status %d, want %d (body %q)", rec.Code, tc.want, rec.Body.String())
			}
			if tc.want == http.StatusUnauthorized {
				if rec.Header().Get("WWW-Authenticate") != "Bearer" {
					t.Errorf("401 without a WWW-Authenticate: Bearer header")
				}
				if strings.Contains(rec.Body.String(), fixtureToken) {
					t.Errorf("401 body leaks the token: %q", rec.Body.String())
				}
			}
		})
	}
}

func TestRequireBearerCarriesValidatedKeyForNestedDaemonCalls(t *testing.T) {
	ks := OpenKeyStore(t.TempDir())
	secret, rec, err := ks.Create(KeySpec{User: "owner", Label: "mcp-test", Ceiling: "restricted"}, fixtureRand(19), fixtureNow)
	if err != nil {
		t.Fatal(err)
	}
	sch, err := settings.New(settings.Registry())
	if err != nil {
		t.Fatal(err)
	}
	set := settings.NewSet(sch).WithOverrides(map[string]string{AuthModeSetting: AuthModeRequired}, "test")
	s := &Server{set: set, keys: ks}
	passed := false
	next := http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		identity, ok := KeyIdentityFrom(r.Context())
		if !ok || identity.KeyID != rec.ID || identity.Ceiling != "restricted" {
			t.Errorf("key identity = %+v, %v", identity, ok)
		}
		credential, ok := KeyCredentialFrom(r.Context())
		if !ok || credential != secret {
			t.Errorf("nested-call key credential was not preserved")
		}
		passed = true
		w.WriteHeader(http.StatusOK)
	})
	req := httptest.NewRequest(http.MethodPost, "/mcp", nil)
	req.Header.Set("Authorization", "Bearer "+secret)
	recorder := httptest.NewRecorder()
	s.requireBearer(next).ServeHTTP(recorder, req)
	if recorder.Code != http.StatusOK || !passed {
		t.Fatalf("validated key request: status %d, next called %v", recorder.Code, passed)
	}
}

func TestValidateBearerToken(t *testing.T) {
	if err := ValidateBearerToken(bearerSet(t, "")); err != nil {
		t.Fatalf("unset token refused: %v", err)
	}
	if err := ValidateBearerToken(bearerSet(t, fixtureToken)); err != nil {
		t.Fatalf("32-char token refused: %v", err)
	}
	short := "short-fixture"
	err := ValidateBearerToken(bearerSet(t, short))
	if err == nil {
		t.Fatal("short token accepted")
	}
	if !strings.Contains(err.Error(), BearerTokenSetting) || !strings.Contains(err.Error(), "32") {
		t.Errorf("error should name the setting and the minimum length: %v", err)
	}
	if strings.Contains(err.Error(), short) {
		t.Errorf("error leaks the token value: %v", err)
	}
}

func TestTakeoverRequest(t *testing.T) {
	// Unset: byte-for-byte the request the successor always wrote.
	got := takeoverRequest("")
	want := "POST " + takeoverRoute + " HTTP/1.1\r\nHost: mcpx\r\nContent-Length: 0\r\n\r\n"
	if got != want {
		t.Fatalf("unset rendering changed:\n%q\nwant:\n%q", got, want)
	}
	// Set: the predecessor's gate protects /v1/takeover too, so the
	// successor presents the token.
	got = takeoverRequest(fixtureToken)
	want = "POST " + takeoverRoute + " HTTP/1.1\r\nHost: mcpx\r\nContent-Length: 0\r\n" +
		"Authorization: Bearer " + fixtureToken + "\r\n\r\n"
	if got != want {
		t.Fatalf("set rendering wrong:\n%q\nwant:\n%q", got, want)
	}
}
