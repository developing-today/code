package cli

import (
	"context"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

// #1926: `mcpx config` surfaces Auth.Describe() per server — the variable
// reference, never the resolved secret — and doctor reports the same
// redacted posture.
const authTestConfig = `{"mcpServers":{
	"api": {"url": "https://example.test/mcp", "auth": {"type": "bearer", "token": "${MCPX_AUTH_TEST_TOKEN}"}},
	"local": {"command": "true"}
}}`

func writeAuthConfig(t *testing.T) string {
	t.Helper()
	t.Setenv("MCPX_AUTH_TEST_TOKEN", "the-resolved-secret-value")
	p := filepath.Join(t.TempDir(), "c.json")
	if err := os.WriteFile(p, []byte(authTestConfig), 0o600); err != nil {
		t.Fatal(err)
	}
	return p
}

func TestConfigPrintsRedactedAuth(t *testing.T) {
	cfg := writeAuthConfig(t)
	a := &App{ConfigPath: cfg, JSON: true}
	out, err := captureStdout(t, func() error {
		return a.CmdConfig(context.Background(), nil)
	})
	if err != nil {
		t.Fatal(err)
	}
	if !strings.Contains(out, `"auth": "bearer ${MCPX_AUTH_TEST_TOKEN}"`) {
		t.Fatalf("config output lacks the redacted bearer reference:\n%s", out)
	}
	if !strings.Contains(out, `"auth": "none"`) {
		t.Fatalf("config output lacks auth=none for the unauthenticated server:\n%s", out)
	}
	if strings.Contains(out, "the-resolved-secret-value") {
		t.Fatalf("config output leaked the resolved secret:\n%s", out)
	}
}

func TestDoctorReportsAuthPosture(t *testing.T) {
	cfg := writeAuthConfig(t)
	a := &App{ConfigPath: cfg, JSON: true}
	// Doctor's exit reflects every check (a daemon-less test box fails
	// the connection check); the auth posture is what this test is for,
	// so the error is deliberately not asserted.
	out, _ := captureStdout(t, func() error {
		return a.CmdDoctor(context.Background(), []string{"-v"})
	})
	if !strings.Contains(out, "api: bearer ${MCPX_AUTH_TEST_TOKEN}") {
		t.Fatalf("doctor output lacks the auth posture:\n%s", out)
	}
	if strings.Contains(out, "the-resolved-secret-value") {
		t.Fatalf("doctor output leaked the resolved secret:\n%s", out)
	}
}
