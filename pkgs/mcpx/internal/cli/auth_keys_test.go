package cli

import (
	"bytes"
	"context"
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/developing-today/code/pkgs/mcpx/internal/daemon"
)

func runAuthKeyCLI(t *testing.T, args ...string) (int, string, string) {
	t.Helper()
	var stderr bytes.Buffer
	code := 0
	stdout, err := captureStdout(t, func() error {
		code = Run(context.Background(), args, Env(os.Environ()), Stdio{Err: &stderr})
		return nil
	})
	if err != nil {
		t.Fatal(err)
	}
	return code, stdout, stderr.String()
}

func TestAuthKeyLifecycleCLI(t *testing.T) {
	// Oracle: the key-store reader must resolve the one-time create output,
	// while the persisted JSON and list output contain only the hash/public
	// metadata. Revoke is correct when the same reader rejects that secret on
	// its next lookup and the listed record is marked revoked.
	home := t.TempDir()
	stateDir := filepath.Join(t.TempDir(), "state")
	t.Setenv("HOME", home)
	t.Setenv("MCPX_STATE_DIR", stateDir)

	code, created, stderr := runAuthKeyCLI(t,
		"auth", "key", "create",
		"--user", "owner",
		"--label", "laptop-cli",
		"--ceiling", daemon.CeilingFull,
	)
	if code != 0 {
		t.Fatalf("create exit code = %d, stderr = %q, output = %q", code, stderr, created)
	}
	var secret string
	for _, field := range strings.Fields(created) {
		if strings.HasPrefix(field, daemon.KeyPrefix) {
			secret = field
			break
		}
	}
	if secret == "" || strings.Count(created, secret) != 1 {
		t.Fatalf("create must print one generated secret exactly once, got %q", created)
	}

	store := daemon.OpenKeyStore(stateDir)
	record, ok := store.Lookup(secret)
	if !ok {
		t.Fatal("the key returned by create does not resolve in the state store")
	}
	if record.User != "owner" || record.Label != "laptop-cli" || record.Ceiling != daemon.CeilingFull {
		t.Fatalf("created key metadata = %+v", record)
	}
	if len(record.ID) != 12 {
		t.Fatalf("public key ID length = %d, want 12", len(record.ID))
	}

	storePath := filepath.Join(stateDir, daemon.KeyStoreFile)
	stored, err := os.ReadFile(storePath)
	if err != nil {
		t.Fatal(err)
	}
	if bytes.Contains(stored, []byte(secret)) || bytes.Contains(stored, []byte(strings.TrimPrefix(secret, daemon.KeyPrefix))) {
		t.Fatal("auth.json contains the secret instead of only its hash")
	}
	if !bytes.Contains(stored, []byte(record.Hash)) {
		t.Fatal("auth.json does not contain the record's SHA-256 hash")
	}
	fileInfo, err := os.Stat(storePath)
	if err != nil {
		t.Fatal(err)
	}
	if got := fileInfo.Mode().Perm(); got != 0o600 {
		t.Fatalf("auth.json permissions = %04o, want 0600", got)
	}
	dirInfo, err := os.Stat(stateDir)
	if err != nil {
		t.Fatal(err)
	}
	if got := dirInfo.Mode().Perm(); got != 0o700 {
		t.Fatalf("state directory permissions = %04o, want 0700", got)
	}

	code, listed, stderr := runAuthKeyCLI(t, "--json", "auth", "key", "list")
	if code != 0 {
		t.Fatalf("list exit code = %d, stderr = %q, output = %q", code, stderr, listed)
	}
	if !strings.Contains(listed, record.ID) || !strings.Contains(listed, "laptop-cli") {
		t.Fatalf("list omitted public key metadata: %q", listed)
	}
	if strings.Contains(listed, secret) || strings.Contains(listed, record.Hash) || strings.Contains(listed, `"hash"`) {
		t.Fatalf("list exposed key material: %q", listed)
	}

	code, revoked, stderr := runAuthKeyCLI(t, "auth", "key", "revoke", record.ID)
	if code != 0 {
		t.Fatalf("revoke exit code = %d, stderr = %q, output = %q", code, stderr, revoked)
	}
	if _, ok := store.Lookup(secret); ok {
		t.Fatal("revoke did not take effect on the next key-store lookup")
	}
	keys, err := store.List()
	if err != nil {
		t.Fatal(err)
	}
	if len(keys) != 1 || !keys[0].Revoked || keys[0].RevokedAt == nil {
		t.Fatalf("list after revoke = %+v, want the key marked revoked", keys)
	}
}

func TestAuthKeyCLIRejectsUnexpectedArguments(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	t.Setenv("MCPX_STATE_DIR", filepath.Join(t.TempDir(), "state"))

	for _, tc := range []struct {
		name string
		args []string
	}{
		{name: "create positional", args: []string{"auth", "key", "create", "--user", "owner", "--label", "x", "stray"}},
		{name: "revoke extra", args: []string{"auth", "key", "revoke", "one", "two"}},
		{name: "unknown option", args: []string{"auth", "key", "create", "--user", "owner", "--label", "x", "--unknown"}},
	} {
		t.Run(tc.name, func(t *testing.T) {
			code, _, stderr := runAuthKeyCLI(t, tc.args...)
			if code == 0 {
				t.Fatalf("unexpected arguments succeeded: %v", tc.args)
			}
			if strings.TrimSpace(stderr) == "" {
				t.Fatalf("unexpected arguments failed without a usage/error message: %v", tc.args)
			}
		})
	}
}

func TestAuthKeyHelpDocumentsRecovery(t *testing.T) {
	out, err := captureStdout(t, func() error {
		return (&App{}).helpFor("auth")
	})
	if err != nil {
		t.Fatal(err)
	}
	for _, want := range []string{"create", "list", "revoke", "shown once", "create a replacement"} {
		if !strings.Contains(strings.ToLower(out), strings.ToLower(want)) {
			t.Errorf("auth help lacks %q: %q", want, out)
		}
	}
}
