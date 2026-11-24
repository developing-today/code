package placement

import (
	"os"
	"path/filepath"
	"strings"
	"testing"
)

// TestReadHardStops is the reader half of issue #2732 proposal step 2:
// a steward hard-stop note lives in a machine-readable file the seating
// path checks. The oracle is the file contract in ReadHardStops: a
// stopped pool reads true, an absent pool or an explicit
// stopped:false entry reads false, and a missing, malformed, or
// wrong-version file is an error — never a silent "not stopped".
func TestReadHardStops(t *testing.T) {
	write := func(contents string) string {
		t.Helper()
		path := filepath.Join(t.TempDir(), "hard-stops.json")
		if err := os.WriteFile(path, []byte(contents), 0o600); err != nil {
			t.Fatalf("write hard-stops fixture: %v", err)
		}
		return path
	}

	valid := write(`{"version":1,"stops":{"chatgpt-plan":{"stopped":true,"note":"monthly 97% hard stop","issuedAt":"2026-10-29T13:22:00Z"},"opencode-go.popesdrew":{"stopped":false}}}`)

	t.Run("stopped pool reads true", func(t *testing.T) {
		stopped, err := ReadHardStops(valid, "chatgpt-plan")
		if err != nil {
			t.Fatalf("ReadHardStops: %v", err)
		}
		if !stopped {
			t.Fatal("stopped pool did not read stopped")
		}
	})

	t.Run("absent pool reads false", func(t *testing.T) {
		stopped, err := ReadHardStops(valid, "never-seen-pool")
		if err != nil {
			t.Fatalf("ReadHardStops: %v", err)
		}
		if stopped {
			t.Fatal("absent pool read stopped")
		}
	})

	t.Run("explicit stopped:false reads false", func(t *testing.T) {
		stopped, err := ReadHardStops(valid, "opencode-go.popesdrew")
		if err != nil {
			t.Fatalf("ReadHardStops: %v", err)
		}
		if stopped {
			t.Fatal("stopped:false entry read stopped")
		}
	})

	t.Run("missing file is an error", func(t *testing.T) {
		if _, err := ReadHardStops(filepath.Join(t.TempDir(), "absent.json"), "chatgpt-plan"); err == nil {
			t.Fatal("expected an error for a missing hard-stops file, got nil")
		}
	})

	t.Run("malformed file is an error", func(t *testing.T) {
		path := write(`{"version":1,"stops":`)
		if _, err := ReadHardStops(path, "chatgpt-plan"); err == nil {
			t.Fatal("expected an error for a malformed hard-stops file, got nil")
		}
	})

	t.Run("wrong version is an error", func(t *testing.T) {
		path := write(`{"version":2,"stops":{"chatgpt-plan":{"stopped":true}}}`)
		_, err := ReadHardStops(path, "chatgpt-plan")
		if err == nil || !strings.Contains(err.Error(), "unsupported version") {
			t.Fatalf("expected an unsupported-version error, got %v", err)
		}
	})
}
