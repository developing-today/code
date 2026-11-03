package daemon

import (
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"net/http"
	"os"
	"path/filepath"
	"sync"
	"time"
)

const (
	writeLedgerFilename = "unknown-writes.json"
	writeLedgerVersion  = 1
)

type unknownWriteRecord struct {
	Server     string    `json:"server"`
	Tool       string    `json:"tool"`
	ArgsSHA256 string    `json:"argsSha256"`
	At         time.Time `json:"at"`
	Surface    string    `json:"surface"`
}

// ErrUnknownOutcome refuses another write after an earlier dispatched call
// failed without a conclusive result.
type ErrUnknownOutcome struct {
	Record           unknownWriteRecord
	Reason           string
	PersistenceError error
}

func (e ErrUnknownOutcome) Error() string {
	message := fmt.Sprintf("%s.%s may have completed, but its outcome is unknown; further non-read-only calls to %q are blocked until cleared with `mcpx writes clear %s` (POST /v1/writes/%s/clear)",
		e.Record.Server, e.Record.Tool, e.Record.Server, e.Record.Server, e.Record.Server)
	if e.Reason != "" {
		message += "; " + e.Reason
	}
	if e.PersistenceError != nil {
		message += "; the block is active only for this daemon lifetime because persisting it failed: " + e.PersistenceError.Error()
	}
	return message
}

func (e ErrUnknownOutcome) Unwrap() error { return e.PersistenceError }

type writeLedgerFile struct {
	Version int                           `json:"version"`
	Writes  map[string]unknownWriteRecord `json:"writes"`
}

// WriteLedger keeps the first unknown write outcome for each server. A server
// stays blocked until an operator clears that entry explicitly.
type WriteLedger struct {
	mu     sync.RWMutex
	path   string
	writes map[string]unknownWriteRecord
}

// OpenWriteLedger loads the versioned ledger from the daemon state directory.
// An empty directory is supported by in-memory registries used in tests.
func OpenWriteLedger(stateDir string) (*WriteLedger, error) {
	l := &WriteLedger{writes: map[string]unknownWriteRecord{}}
	if stateDir == "" {
		return l, nil
	}
	l.path = filepath.Join(stateDir, writeLedgerFilename)
	data, err := os.ReadFile(l.path)
	if errors.Is(err, os.ErrNotExist) {
		return l, nil
	}
	if err != nil {
		return nil, fmt.Errorf("read unknown-write ledger: %w", err)
	}
	var file writeLedgerFile
	if err := json.Unmarshal(data, &file); err != nil {
		return nil, fmt.Errorf("decode unknown-write ledger: %w", err)
	}
	if file.Version != writeLedgerVersion {
		return nil, fmt.Errorf("unsupported unknown-write ledger version %d", file.Version)
	}
	for server, record := range file.Writes {
		if server == "" || record.Server != server || record.Tool == "" || len(record.ArgsSHA256) != 64 || record.At.IsZero() || record.Surface == "" {
			return nil, fmt.Errorf("invalid unknown-write ledger entry for %q", server)
		}
		if _, err := hex.DecodeString(record.ArgsSHA256); err != nil {
			return nil, fmt.Errorf("invalid unknown-write ledger digest for %q", server)
		}
		l.writes[server] = record
	}
	return l, nil
}

// Lookup reports whether server has an uncleared unknown write outcome.
func (l *WriteLedger) Lookup(server string) (unknownWriteRecord, bool) {
	l.mu.RLock()
	defer l.mu.RUnlock()
	record, ok := l.writes[server]
	return record, ok
}

// Record stores an unknown write outcome without retaining its arguments.
// The first record wins because the first one already blocks further writes.
func (l *WriteLedger) Record(record unknownWriteRecord) error {
	if record.Server == "" || record.Tool == "" || len(record.ArgsSHA256) != 64 || record.At.IsZero() || record.Surface == "" {
		return errors.New("unknown-write ledger record is incomplete")
	}
	if _, err := hex.DecodeString(record.ArgsSHA256); err != nil {
		return errors.New("unknown-write ledger digest must be SHA-256 hex")
	}
	l.mu.Lock()
	defer l.mu.Unlock()
	if _, exists := l.writes[record.Server]; exists {
		return nil
	}
	next := cloneWriteRecords(l.writes)
	next[record.Server] = record
	// Block immediately in this daemon even if the disk write fails. Return
	// the persistence error so the caller can surface the loss of restart
	// protection rather than silently claiming it was recorded durably.
	l.writes = next
	return l.saveLocked(next)
}

// Clear removes one server's block. Clearing a server with no record is a
// successful no-op, so the CLI command is safe to repeat.
func (l *WriteLedger) Clear(server string) (bool, error) {
	l.mu.Lock()
	defer l.mu.Unlock()
	if _, exists := l.writes[server]; !exists {
		return false, nil
	}
	next := cloneWriteRecords(l.writes)
	delete(next, server)
	if err := l.saveLocked(next); err != nil {
		return false, err
	}
	l.writes = next
	return true, nil
}

func cloneWriteRecords(in map[string]unknownWriteRecord) map[string]unknownWriteRecord {
	out := make(map[string]unknownWriteRecord, len(in))
	for server, record := range in {
		out[server] = record
	}
	return out
}

func (l *WriteLedger) saveLocked(writes map[string]unknownWriteRecord) error {
	if l.path == "" {
		return nil
	}
	if err := os.MkdirAll(filepath.Dir(l.path), 0o700); err != nil {
		return fmt.Errorf("create unknown-write ledger directory: %w", err)
	}
	data, err := json.MarshalIndent(writeLedgerFile{Version: writeLedgerVersion, Writes: writes}, "", "  ")
	if err != nil {
		return fmt.Errorf("encode unknown-write ledger: %w", err)
	}
	dir := filepath.Dir(l.path)
	tmp, err := os.CreateTemp(dir, ".unknown-writes-*.tmp")
	if err != nil {
		return fmt.Errorf("create unknown-write ledger temp file: %w", err)
	}
	tmpName := tmp.Name()
	defer os.Remove(tmpName)
	if err := tmp.Chmod(0o600); err != nil {
		_ = tmp.Close()
		return fmt.Errorf("set unknown-write ledger permissions: %w", err)
	}
	if _, err := tmp.Write(data); err != nil {
		_ = tmp.Close()
		return fmt.Errorf("write unknown-write ledger: %w", err)
	}
	if err := tmp.Sync(); err != nil {
		_ = tmp.Close()
		return fmt.Errorf("sync unknown-write ledger: %w", err)
	}
	if err := tmp.Close(); err != nil {
		return fmt.Errorf("close unknown-write ledger: %w", err)
	}
	if err := os.Rename(tmpName, l.path); err != nil {
		return fmt.Errorf("publish unknown-write ledger: %w", err)
	}
	if dirFile, err := os.Open(dir); err == nil {
		_ = dirFile.Sync()
		_ = dirFile.Close()
	}
	return nil
}

func (s *Server) handleWriteClear(w http.ResponseWriter, r *http.Request) {
	server := s.reg.CanonicalName(r.PathValue("server"))
	if server == "" || s.reg.writeLedger == nil {
		http.Error(w, "unknown-write ledger is unavailable", http.StatusInternalServerError)
		return
	}
	cleared, err := s.reg.writeLedger.Clear(server)
	if err != nil {
		writeJSON(w, http.StatusInternalServerError, map[string]any{"error": err.Error()})
		return
	}
	writeJSON(w, http.StatusOK, map[string]any{"server": server, "cleared": cleared})
}
