package web

import (
	"bytes"
	"net/http"
	"net/http/httptest"
	"net/url"
	"os"
	"strings"
	"testing"

	"github.com/charmbracelet/log"
	"github.com/developing-today/code/src/identity/auth"
	"github.com/golang-jwt/jwt"
)

func captureAuthLogs(t *testing.T) *bytes.Buffer {
	t.Helper()

	var output bytes.Buffer
	log.SetOutput(&output)
	t.Cleanup(func() { log.SetOutput(os.Stderr) })
	return &output
}

func TestSetCookieHandlerDoesNotLogJWT(t *testing.T) {
	const (
		claimSentinel = "jwt-claim-secret-sentinel"
		signingKey    = "test-key-for-auth-log-test"
	)

	token, err := jwt.NewWithClaims(jwt.SigningMethodHS256, jwt.MapClaims{"sub": claimSentinel}).SignedString([]byte(signingKey))
	if err != nil {
		t.Fatal(err)
	}

	logs := captureAuthLogs(t)
	keyFunc := func(*jwt.Token) (any, error) { return []byte(signingKey), nil }
	handler := setCookieHandlerWithConnectionFactory(keyFunc, "Authorization", func(string, jwt.Keyfunc) (*auth.Connection, error) {
		return &auth.Connection{}, nil
	})
	request := httptest.NewRequest(http.MethodPost, "/set-cookie", strings.NewReader(url.Values{"Authorization": {token}}.Encode()))
	request.Header.Set("Content-Type", "application/x-www-form-urlencoded")
	response := httptest.NewRecorder()
	handler(response, request)

	if response.Code != http.StatusOK {
		t.Fatalf("status = %d, want %d; body=%q", response.Code, http.StatusOK, response.Body.String())
	}
	for _, secret := range []string{token, claimSentinel} {
		if strings.Contains(logs.String(), secret) {
			t.Fatalf("authentication log contains JWT material %q: %s", secret, logs.String())
		}
	}
}

func TestSetCookieHandlerDoesNotLogInvalidJWT(t *testing.T) {
	const tokenSentinel = "invalid-jwt-secret-sentinel"

	logs := captureAuthLogs(t)
	keyFunc := func(*jwt.Token) (any, error) { return []byte("unused"), nil }
	handler := setCookieHandlerWithConnectionFactory(keyFunc, "Authorization", func(string, jwt.Keyfunc) (*auth.Connection, error) {
		t.Fatal("connection factory called for an invalid JWT")
		return nil, nil
	})
	request := httptest.NewRequest(http.MethodPost, "/set-cookie", strings.NewReader(url.Values{"Authorization": {tokenSentinel}}.Encode()))
	request.Header.Set("Content-Type", "application/x-www-form-urlencoded")
	response := httptest.NewRecorder()
	handler(response, request)

	if response.Code != http.StatusInternalServerError {
		t.Fatalf("status = %d, want %d; body=%q", response.Code, http.StatusInternalServerError, response.Body.String())
	}
	if strings.Contains(logs.String(), tokenSentinel) {
		t.Fatalf("authentication log contains invalid JWT: %s", logs.String())
	}
}
