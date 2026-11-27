package placement

import (
	"bytes"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"os"
	"path/filepath"
	"strings"
	"syscall"
	"time"
)

// ReadHardStops reads the machine-readable steward hard-stop registry for
// issue #2732 proposal step 2. The seating path checks this steward-written
// file instead of relying on dispatcher-log prose. It reports whether the
// named pool is under a steward hard-stop.
//
// File contract (version 1):
//
//	{"version":1,"stops":{"<serviceId>":{"stopped":true,"note":"...","issuedAt":"..."}}}
//
// A pool with no entry — or an entry with stopped false — is not
// stopped. A missing, unreadable, or malformed file is an error, never
// a silent "not stopped": a seating path that asked for hard-stop
// enforcement must fail loudly rather than seat unguarded.
func ReadHardStops(path, pool string) (bool, error) {
	raw, err := os.ReadFile(path)
	if err != nil {
		return false, fmt.Errorf("hard-stops: read %s: %w", path, err)
	}
	var doc hardStopsDoc
	if err := json.Unmarshal(raw, &doc); err != nil {
		return false, fmt.Errorf("hard-stops: parse %s: %w", path, err)
	}
	if doc.Version != 1 {
		return false, fmt.Errorf("hard-stops: %s: unsupported version %d, want 1", path, doc.Version)
	}
	entry, ok := doc.Stops[pool]
	if !ok {
		return false, nil
	}
	return entry.Stopped, nil
}

type hardStopsDoc struct {
	Version int                      `json:"version"`
	Stops   map[string]hardStopEntry `json:"stops"`
}

type hardStopEntry struct {
	Stopped  bool   `json:"stopped"`
	Note     string `json:"note,omitempty"`
	IssuedAt string `json:"issuedAt,omitempty"`
}

// WriteHardStop creates, updates, or clears one pool's steward hard-stop
// entry. A cleared entry remains in the registry with stopped:false and the
// clearing reason, preserving an auditable lifecycle. The replacement is
// written to a same-directory temporary file and atomically renamed so a
// reader never observes a partially-written registry.
func WriteHardStop(path, pool, note string, stopped bool, issuedAt time.Time) error {
	pool = strings.TrimSpace(pool)
	if pool == "" {
		return fmt.Errorf("hard-stops: pool identity is required")
	}
	if strings.TrimSpace(note) == "" {
		return fmt.Errorf("hard-stops: an auditable reason is required")
	}
	if path == "" {
		return fmt.Errorf("hard-stops: registry path is required")
	}
	if issuedAt.IsZero() {
		return fmt.Errorf("hard-stops: issued-at timestamp is required")
	}

	// A persistent sidecar lock serializes the whole read-modify-rename,
	// not the replaced inode. Never remove the lock file: doing so would
	// allow contenders to lock different inodes and lose another pool's stop.
	lock, err := os.OpenFile(path+".lock", os.O_CREATE|os.O_RDWR, 0o600)
	if err != nil {
		return fmt.Errorf("hard-stops: open writer lock: %w", err)
	}
	defer lock.Close()
	if err := syscall.Flock(int(lock.Fd()), syscall.LOCK_EX); err != nil {
		return fmt.Errorf("hard-stops: lock registry: %w", err)
	}
	defer syscall.Flock(int(lock.Fd()), syscall.LOCK_UN)

	doc := hardStopsDoc{Version: 1, Stops: make(map[string]hardStopEntry)}
	raw, err := os.ReadFile(path)
	if err == nil {
		doc = hardStopsDoc{}
		decoder := json.NewDecoder(bytes.NewReader(raw))
		decoder.DisallowUnknownFields()
		if err := decoder.Decode(&doc); err != nil {
			return fmt.Errorf("hard-stops: parse %s: %w", path, err)
		}
		if err := decoder.Decode(new(any)); err != io.EOF {
			return fmt.Errorf("hard-stops: %s: unexpected trailing JSON", path)
		}
		if doc.Version != 1 {
			return fmt.Errorf("hard-stops: %s: unsupported version %d, want 1", path, doc.Version)
		}
		if doc.Stops == nil {
			return fmt.Errorf("hard-stops: %s: stops must be an object", path)
		}
	} else if !errors.Is(err, os.ErrNotExist) {
		return fmt.Errorf("hard-stops: read %s: %w", path, err)
	}
	doc.Stops[pool] = hardStopEntry{
		Stopped:  stopped,
		Note:     strings.TrimSpace(note),
		IssuedAt: issuedAt.UTC().Format(time.RFC3339),
	}
	encoded, err := json.MarshalIndent(doc, "", "  ")
	if err != nil {
		return fmt.Errorf("hard-stops: encode %s: %w", path, err)
	}
	encoded = append(encoded, '\n')

	dir := filepath.Dir(path)
	tmp, err := os.CreateTemp(dir, ".hard-stops-*.tmp")
	if err != nil {
		return fmt.Errorf("hard-stops: create temporary registry in %s: %w", dir, err)
	}
	tmpName := tmp.Name()
	defer os.Remove(tmpName)
	if err := tmp.Chmod(0o600); err != nil {
		tmp.Close()
		return fmt.Errorf("hard-stops: set registry permissions: %w", err)
	}
	if _, err := tmp.Write(encoded); err != nil {
		tmp.Close()
		return fmt.Errorf("hard-stops: write registry: %w", err)
	}
	if err := tmp.Sync(); err != nil {
		tmp.Close()
		return fmt.Errorf("hard-stops: sync registry: %w", err)
	}
	if err := tmp.Close(); err != nil {
		return fmt.Errorf("hard-stops: close registry: %w", err)
	}
	if err := os.Rename(tmpName, path); err != nil {
		return fmt.Errorf("hard-stops: replace %s: %w", path, err)
	}
	return nil
}
