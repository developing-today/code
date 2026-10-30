package logging_test

import (
	"strings"
	"testing"

	"github.com/dezren39/mcpx/internal/logging"
)

func TestRedactElidesByNameAndByValue(t *testing.T) {
	// Matching only on the name was not enough. MCPX_SCRIPT_ENV carries
	// KEY=VALUE pairs for the script, so an innocuous name holds a secret as
	// its value.
	cases := []struct {
		name, value string
		leaks       string // must not appear in the output
	}{
		{"MCPX_API_TOKEN", "abcdef123456", "abcdef123456"},
		{"MCPX_SECRET_THING", "hunter2", "hunter2"},
		{"MCPX_AUTH", "Bearer xyzzy", "xyzzy"},
		{"MCPX_SCRIPT_ENV", "API_TOKEN=abcdef123456", "abcdef123456"},
		{"MCPX_SCRIPT_ENV", "PASSWORD=hunter2", "hunter2"},
		{"MCPX_HEADERS", "Authorization: Bearer abcdef123456", "abcdef123456"},
		{"MCPX_ANYTHING", "ghp_abcdefghijklmnopqrst", "ghp_abcdefghijklmnopqrst"},
		{"MCPX_ANYTHING", "sk-abcdefghijklmnopqrstuv", "sk-abcdefghijklmnopqrstuv"},
		{"MCPX_ANYTHING", "xoxb-1234567890-abcdef", "xoxb-1234567890-abcdef"},
		{"MCPX_ANYTHING", "AKIAIOSFODNN7EXAMPLE", "AKIAIOSFODNN7EXAMPLE"},
	}
	for _, c := range cases {
		got := logging.Redact(c.name, c.value)
		if strings.Contains(got, c.leaks) {
			t.Errorf("%s=%q leaked %q through as %q", c.name, c.value, c.leaks, got)
		}
	}
}

func TestRedactKeepsValuesWorthSeeing(t *testing.T) {
	// Over-eager elision makes the block useless; these are the values the
	// ambient env block exists to show.
	for _, c := range []struct{ name, value string }{
		{"MCPX_STATE_DIR", "/home/x/.local/state/mcpx"},
		{"MCPX_ENDPOINT", "http://127.0.0.1:54501"},
		{"MCPX_LOGGING_LEVEL", "debug"},
		{"MCPX_SCRIPT_ENV", "DEBUG=1"},
	} {
		if got := logging.Redact(c.name, c.value); got != c.value {
			t.Errorf("%s=%q should have been kept, got %q", c.name, c.value, got)
		}
	}
}

func TestRedactKeepsTheSurroundingContextOfAPartialMatch(t *testing.T) {
	// A variable holding several pairs is still worth seeing, minus the one
	// that matters.
	got := logging.Redact("MCPX_SCRIPT_ENV", "DEBUG=1 API_TOKEN=abcdef123456")
	if !strings.Contains(got, "DEBUG=1") {
		t.Errorf("the harmless pair should survive: %q", got)
	}
	if strings.Contains(got, "abcdef123456") {
		t.Errorf("the secret should not: %q", got)
	}
	if !strings.Contains(got, "<elided>") {
		t.Errorf("the elision should be visible: %q", got)
	}
}

func TestRedactElidesFullJWTAndReconciledKeywords(t *testing.T) {
	// Regression for #1710: three leak shapes survived Redact.
	// (1) A JWT match ended at the second dot, leaving the signature
	// segment verbatim. (2) The KEY=VALUE keyword list had drifted from
	// sensitiveName, so SESSION=/COOKIE= and friends passed through.
	// (3) URL userinfo passwords were not matched at all.
	jwt := "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.SflKxwRJSMeKKF2QT4fwpMeJf36POk6yJVadQssw5c"
	cases := []struct {
		name, value string
		leaks       string // must not appear in the output
	}{
		{"MCPX_ANYTHING", jwt, "SflKxwRJSMeKKF2QT4fwpMeJf36POk6yJVadQssw5c"},
		{"MCPX_SCRIPT_ENV", "SESSION=abc123sessionvalue", "abc123sessionvalue"},
		{"MCPX_SCRIPT_ENV", "SESSION_ID=abc123sessionvalue", "abc123sessionvalue"},
		{"MCPX_SCRIPT_ENV", "COOKIE=abc123cookievalue", "abc123cookievalue"},
		{"MCPX_SCRIPT_ENV", "PWD=abc123pwdvalue", "abc123pwdvalue"},
		{"MCPX_SCRIPT_ENV", "SIGNATURE=abc123sigvalue", "abc123sigvalue"},
		{"MCPX_SCRIPT_ENV", "PRIVATE=abc123privatevalue", "abc123privatevalue"},
		{"MCPX_SCRIPT_ENV", "AUTH=abc123authvalue", "abc123authvalue"},
		{"MCPX_ANYTHING", "https://deploy:s3cr3tpw@example.com:8443/path", "s3cr3tpw"},
	}
	for _, c := range cases {
		got := logging.Redact(c.name, c.value)
		if strings.Contains(got, c.leaks) {
			t.Errorf("%s=%q leaked %q through as %q", c.name, c.value, c.leaks, got)
		}
	}
}

func TestRedactURLUserinfoKeepsTheRestOfTheURL(t *testing.T) {
	got := logging.Redact("MCPX_ANYTHING", "https://deploy:s3cr3tpw@example.com:8443/path")
	want := "https://deploy:<elided>@example.com:8443/path"
	if got != want {
		t.Errorf("userinfo URL should elide only the password: got %q, want %q", got, want)
	}
}

func TestRedactKeepsCredentialFreeURLs(t *testing.T) {
	for _, value := range []string{
		"https://example.com:8443/path?q=1",
		"https://user@example.com/path",
		"http://127.0.0.1:54501",
	} {
		if got := logging.Redact("MCPX_ENDPOINT", value); got != value {
			t.Errorf("credential-free URL %q should have been kept, got %q", value, got)
		}
	}
}

func TestRedactKeywordPairURLValues(t *testing.T) {
	// Regression for #2463: the keyword-pair alternative matches a
	// whole KEY=VALUE token before the URL userinfo alternative can
	// run, so a URL carried under an AUTH_/TOKEN_ name went through
	// the userinfo reconstruction and came out mangled — the port
	// printed as an elided password, a fabricated "@", and the
	// host/port/path dropped.
	cases := []struct{ name, value, want string }{
		// Credential-free ported URL under a keyword: kept readable,
		// byte for byte, with no fabricated userinfo.
		{"MCPX_SCRIPT_ENV", "AUTH_URL=https://auth.example.com:8443/callback", "AUTH_URL=https://auth.example.com:8443/callback"},
		// Userinfo without a password carries no credential either.
		{"MCPX_SCRIPT_ENV", "AUTH_URL=https://user@example.com/path", "AUTH_URL=https://user@example.com/path"},
		// Userinfo URL under a keyword: only the password is elided;
		// scheme, user, host, port and path all survive.
		{"MCPX_SCRIPT_ENV", "TOKEN_URL=https://deploy:s3cr3tpw@example.com:8443/path", "TOKEN_URL=https://deploy:<elided>@example.com:8443/path"},
		// The pair sits in surrounding context, which survives too.
		{"MCPX_SCRIPT_ENV", "DEBUG=1 AUTH_URL=https://auth.example.com:8443/callback", "DEBUG=1 AUTH_URL=https://auth.example.com:8443/callback"},
		// A keyword-pair URL stays readable, but the credential nested
		// inside it is still elided (#2463 review): keeping the URL for
		// its context must never keep a secret with it.
		{"MCPX_SCRIPT_ENV", "AUTH_URL=https://example.com/callback?token=abc123", "AUTH_URL=https://example.com/callback?token=<elided>"},
		{"MCPX_SCRIPT_ENV", "AUTH_URL=https://example.com/path?q=1&api_key=abcdef", "AUTH_URL=https://example.com/path?q=1&api_key=<elided>"},
		// A token literal in the path is a credential wherever it sits.
		{"MCPX_SCRIPT_ENV", "AUTH_URL=https://example.com/ghp_abcdefghijklmnopqrst", "AUTH_URL=https://example.com/<elided>"},
	}
	for _, c := range cases {
		if got := logging.Redact(c.name, c.value); got != c.want {
			t.Errorf("%s=%q: got %q, want %q", c.name, c.value, got, c.want)
		}
	}
}
