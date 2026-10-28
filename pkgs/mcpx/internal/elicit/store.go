package elicit

import (
	"fmt"
	"io"
	"os"
	"path/filepath"
	"syscall"

	"github.com/dezren39/mcpx/internal/defaults"
)

// storeFileMode is the mode the broker database and its SQLite sidecars
// are held at: owner read/write only. Pending questions can carry
// credentials and consent prompts, so the store is never group- or
// world-readable (design #1675, Phase 1).
const storeFileMode os.FileMode = 0o600

// StorePath is the one place that knows where the broker store lives:
// <state>/elicit/elicit.db, in its own owner-only directory rather than
// beside the log index. Every opener — daemon and CLI alike — resolves
// the store through here so the two can never drift apart again.
func StorePath(stateDir string) string {
	return filepath.Join(stateDir, "elicit", "elicit.db")
}

// OpenAt opens the broker store for a state directory, migrating a
// legacy store into place first when one exists.
//
// The legacy store lived at legacyPath (daemon: <state>/logs/elicit.db;
// CLI: <effective logging.dir>/elicit.db — the logging directory is
// honoured as a migration source only; the destination is always
// StorePath). Migration preserves pending questions: files are renamed
// into place, with a copy-then-delete fallback when a rename cannot
// cross filesystems. If migration fails outright, the legacy store is
// opened where it lies, tightened in place, rather than leaving the
// broker unusable or stranding its questions.
//
// The store directory is created and held at defaults.DirMode (0700)
// and the database files at 0600. Tightening is chmod-if-looser and
// idempotent, so a daemon and a CLI opening the store at the same
// moment cannot fail each other, and a store loosened by hand or by an
// older build is tightened on the next open. A tightening failure is a
// returned error: a store that cannot be made owner-only is not opened
// at all.
func OpenAt(stateDir, legacyPath string) (*Broker, error) {
	dest := StorePath(stateDir)
	dir := filepath.Dir(dest)
	if err := os.MkdirAll(dir, defaults.DirMode); err != nil {
		return nil, err
	}
	if err := tighten(dir, defaults.DirMode); err != nil {
		return nil, err
	}
	// Migration and the first open of the destination are one atomic
	// step across processes, not just within one: without a shared
	// lock, a second opener can see the destination database file
	// before its WAL sidecar has been moved into place, open it, and
	// permanently lose questions that only exist in the legacy WAL
	// (the state a daemon crash leaves behind). The lock is held only
	// until the store is complete and opened; SQLite itself arbitrates
	// access from then on.
	unlock, err := lockStoreDir(dir)
	if err != nil {
		return nil, err
	}
	defer unlock()
	return openAt(dest, legacyPath)
}

// lockStoreDir takes the store directory's cross-process lock and
// returns the release function. The lock file lives in the owner-only
// store directory and is itself owner-only.
func lockStoreDir(dir string) (func(), error) {
	f, err := os.OpenFile(filepath.Join(dir, "elicit.db.lock"), os.O_CREATE|os.O_RDWR, storeFileMode)
	if err != nil {
		return nil, err
	}
	if err := syscall.Flock(int(f.Fd()), syscall.LOCK_EX); err != nil {
		f.Close()
		return nil, fmt.Errorf("locking elicitation store directory: %w", err)
	}
	return func() {
		_ = syscall.Flock(int(f.Fd()), syscall.LOCK_UN)
		_ = f.Close()
	}, nil
}

// openAt is OpenAt under the store lock: migrate (if needed), open,
// tighten.
func openAt(dest, legacyPath string) (*Broker, error) {
	if legacyPath != "" && legacyPath != dest {
		_, destErr := os.Stat(dest)
		_, legacyErr := os.Stat(legacyPath)
		if os.IsNotExist(destErr) && legacyErr == nil {
			if err := migrateStore(legacyPath, dest); err != nil {
				if _, derr := os.Stat(dest); derr != nil {
					// Migration failed and no store appeared (a
					// concurrent opener that won the rename leaves
					// dest in place, handled by the stat above):
					// fall back to the legacy store, tightened in
					// place.
					b, oerr := Open(legacyPath)
					if oerr != nil {
						return nil, oerr
					}
					if terr := tightenFiles(legacyPath); terr != nil {
						b.Close()
						return nil, fmt.Errorf("tightening legacy elicitation store: %w", terr)
					}
					return b, nil
				}
			}
		}
	}
	dir := filepath.Dir(dest)
	if err := os.MkdirAll(dir, defaults.DirMode); err != nil {
		return nil, err
	}
	if err := tighten(dir, defaults.DirMode); err != nil {
		return nil, err
	}
	b, err := Open(dest)
	if err != nil {
		return nil, err
	}
	if err := tightenFiles(dest); err != nil {
		b.Close()
		return nil, err
	}
	return b, nil
}

// migrateStore moves the legacy database (and any WAL/SHM sidecars)
// to dest, preserving every stored question.
func migrateStore(legacyPath, dest string) error {
	dir := filepath.Dir(dest)
	if err := os.MkdirAll(dir, defaults.DirMode); err != nil {
		return err
	}
	if err := tighten(dir, defaults.DirMode); err != nil {
		return err
	}
	for _, suffix := range []string{"", "-wal", "-shm"} {
		src, dst := legacyPath+suffix, dest+suffix
		if _, err := os.Stat(src); err != nil {
			if os.IsNotExist(err) {
				continue
			}
			return err
		}
		if err := os.Rename(src, dst); err == nil {
			continue
		}
		// A rename can fail across filesystems, or because a
		// concurrent opener already moved the file. Copy, then
		// delete the original.
		if err := copyFile(src, dst); err != nil {
			if suffix == "" {
				if _, derr := os.Stat(dest); derr == nil {
					continue // a concurrent opener won; its copy stands
				}
			}
			return err
		}
		_ = os.Remove(src)
	}
	return nil
}

func copyFile(src, dst string) error {
	in, err := os.Open(src)
	if err != nil {
		return err
	}
	defer in.Close()
	out, err := os.OpenFile(dst, os.O_CREATE|os.O_TRUNC|os.O_WRONLY, storeFileMode)
	if err != nil {
		return err
	}
	if _, err := io.Copy(out, in); err != nil {
		out.Close()
		return err
	}
	if err := out.Sync(); err != nil {
		out.Close()
		return err
	}
	return out.Close()
}

// tighten chmods path to want when its current mode grants anything
// beyond want ("looser"). A mode that is already want, or stricter,
// is left alone. A missing path is not an error: SQLite sidecars
// exist only once the database has been written to.
func tighten(path string, want os.FileMode) error {
	info, err := os.Stat(path)
	if err != nil {
		if os.IsNotExist(err) {
			return nil
		}
		return err
	}
	if info.Mode().Perm()&^want != 0 {
		if err := os.Chmod(path, want); err != nil {
			return fmt.Errorf("tightening %s to %04o: %w", path, want, err)
		}
	}
	return nil
}

// tightenFiles holds the database and its sidecars at storeFileMode.
func tightenFiles(dbPath string) error {
	for _, suffix := range []string{"", "-wal", "-shm"} {
		if err := tighten(dbPath+suffix, storeFileMode); err != nil {
			return err
		}
	}
	return nil
}
