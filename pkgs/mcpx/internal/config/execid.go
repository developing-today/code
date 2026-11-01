package config

import (
	"crypto/sha256"
	"encoding/hex"
	"fmt"
	"io"
	"io/fs"
	"os"
	"path/filepath"
	"strings"

	"github.com/developing-today/code/pkgs/mcpx/internal/defaults"
)

// ExecIdentity says which program a stdio server's command runs, as of now.
//
// PoolID describes the configuration; it cannot see a binary that changed
// underneath an unchanged command. `npx`, `uvx` and a nix profile that
// re-points `~/.nix-profile/bin/foo` all keep the same command, args and env
// while the program behind them moves, so a pool keyed by PoolID alone kept
// running the old program until somebody restarted the daemon.
//
// The identity is what the file holds, not where it is: its content hash when
// it is small, and its device, inode, size, mtime and mode when it is not. A
// symlink re-pointed at a byte-identical file is the same program and keeps
// its child. A re-pointed link to a different file is a different program,
// whatever its path, because its bytes or its inode differ. The stat fallback
// keeps a large program, such as a node binary, from being read whole on every
// check. An in-place rewrite changes size or mtime in practice; a replacement
// changes the inode.
//
// One exception: a script — a hashed file whose first two bytes are #! — is
// identified by its content plus the resolved directory of the real file,
// because a script finds its own files relative to its real location. A node
// script beside its node_modules, re-pointed at an identical copy in another
// directory, is a different program and gets a new child, which runs with the
// new location's dependencies; a re-point within the same directory keeps
// its child. The directory is folded into the hash under a NUL-led marker
// framing: a filesystem path cannot contain NUL, so the trailing directory
// component is unambiguous and no two distinct (content, directory) pairs
// share a hash input. Non-script files stay bytes-only, so their identities
// do not move. Script identities changed once when this rule landed: a
// recorded script identity mismatches at the first check after the upgrade
// and the pool restarts once, through the usual binary-changed path. Above
// the hash limit the identity is the inode, so such a copy is restarted
// instead, script or not.
//
// The identity is "" for a server that is not a child process, and begins
// with "missing:" when the command does not resolve. A missing command is
// its own identity, so the pool still starts and fails the way it always did,
// and it is replaced when the command appears.
func (r *Resolved) ExecIdentity() string {
	_, id, _ := r.ExecState()
	return id
}

// ExecState is the stamp of the executable a command resolves to, and its
// identity. ok is false when the command does not resolve or the file cannot
// be read, in which case identity is the "missing:" form ExecIdentity gives.
//
// The stamp is taken before the identity is worked out, so a rewrite that
// lands in between leaves a stamp that differs from the file and the next
// check sees it.
func (r *Resolved) ExecState() (st ExecStamp, identity string, ok bool) {
	return r.ExecStateAt(r.Cwd)
}

// ExecStateAt is ExecState for a child whose effective spawn cwd is cwd.
// Scope-rooted instances use this instead of resolving a relative command
// against the daemon's cwd (decision 0004, review of #958).
func (r *Resolved) ExecStateAt(cwd string) (st ExecStamp, identity string, ok bool) {
	if !r.Stdio() {
		return ExecStamp{}, "", false
	}
	st, found := r.ExecStampAt(cwd)
	if found {
		if id, err := st.Identity(); err == nil {
			return st, id, true
		}
	}
	return ExecStamp{}, "missing:" + r.Command, false
}

// ExecStamp is the cheap part of an executable's identity: what a stat says
// about the file the command resolves to. A periodic check compares stamps and
// only works out an identity when one moved.
type ExecStamp struct {
	// Path is the resolved real path, after symlinks.
	Path  string
	Inode string
	Size  int64
	MTime int64
	Mode  fs.FileMode
}

// ExecStamp stats the file the command resolves to. It reports false for a
// server that is not a child process, and for a command that does not
// resolve to a file it can stat.
func (r *Resolved) ExecStamp() (ExecStamp, bool) {
	return r.ExecStampAt(r.Cwd)
}

// ExecStampAt is ExecStamp for a child with effective spawn cwd cwd.
func (r *Resolved) ExecStampAt(cwd string) (ExecStamp, bool) {
	if !r.Stdio() {
		return ExecStamp{}, false
	}
	path := r.resolveExecAt(cwd)
	if path == "" {
		return ExecStamp{}, false
	}
	real, err := filepath.EvalSymlinks(path)
	if err != nil {
		return ExecStamp{}, false
	}
	fi, err := os.Stat(real)
	if err != nil {
		return ExecStamp{}, false
	}
	// Not Sys(): its atime moves every time the program runs, which would
	// count as a change on every check.
	return ExecStamp{
		Path:  real,
		Inode: fileInode(fi),
		Size:  fi.Size(),
		MTime: fi.ModTime().UnixNano(),
		Mode:  fi.Mode(),
	}, true
}

// Identity works out what the stamped file holds. It reads the whole file only
// when the file is small enough to hash, and an error means it could not be
// read, so the caller treats it as missing.
func (s ExecStamp) Identity() (string, error) {
	h := sha256.New()
	if s.Mode.IsRegular() && s.Size <= defaults.ExecHashMax {
		f, err := os.Open(s.Path)
		if err != nil {
			return "", err
		}
		// Note the first two bytes while streaming the file: a file that
		// begins with #! is a script, and a script's identity also folds
		// in the resolved directory below.
		var head [2]byte
		n, err := io.ReadFull(f, head[:])
		if err != nil && err != io.EOF && err != io.ErrUnexpectedEOF {
			f.Close()
			return "", err
		}
		h.Write(head[:n])
		_, err = io.Copy(h, f)
		f.Close()
		if err != nil {
			return "", err
		}
		if n == 2 && head == [2]byte{'#', '!'} {
			// Framing: content || NUL || "mcpx:exec-dir" || NUL ||
			// directory. A path cannot contain NUL, so the trailing
			// directory component is uniquely recoverable and no two
			// distinct (content, directory) pairs share a hash input.
			h.Write([]byte{0})
			h.Write([]byte("mcpx:exec-dir"))
			h.Write([]byte{0})
			h.Write([]byte(filepath.Dir(s.Path)))
		}
		return "sha256:" + hex.EncodeToString(h.Sum(nil))[:16], nil
	}
	fmt.Fprintf(h, "%s\x00%d\x00%d\x00%d", s.Inode, s.Size, s.MTime, s.Mode)
	return "stat:" + hex.EncodeToString(h.Sum(nil))[:16], nil
}

// ProcessID is the identity two servers must share to share a pool: the
// configured process definition and the executable it currently resolves to.
func (r *Resolved) ProcessID() string {
	return r.PoolID() + "/" + r.ExecIdentity()
}

// resolveExec finds the file the child will run. It searches the PATH the
// child will see -- the server's own env if it sets one, else the daemon's --
// rather than the daemon's PATH alone, since a server can set its own.
func (r *Resolved) resolveExec() string {
	return r.resolveExecAt(r.Cwd)
}

// ExecUsesScopeRoot reports whether resolving the configured executable can
// depend on a scope root. A relative command containing a path separator is
// resolved by the child relative to its effective spawn cwd when no config
// cwd is set.
func (r *Resolved) ExecUsesScopeRoot() bool {
	return r.Cwd == "" && strings.Contains(r.Command, string(filepath.Separator)) && !filepath.IsAbs(r.Command)
}

func (r *Resolved) resolveExecAt(cwd string) string {
	if strings.Contains(r.Command, string(filepath.Separator)) {
		p := r.Command
		if !filepath.IsAbs(p) && cwd != "" {
			p = filepath.Join(cwd, p)
		}
		if isExecutable(p) {
			return p
		}
		return ""
	}
	pathEnv, ok := r.Env["PATH"]
	if !ok {
		pathEnv = os.Getenv("PATH")
	}
	for _, dir := range filepath.SplitList(pathEnv) {
		if dir == "" {
			dir = "."
		}
		p := filepath.Join(dir, r.Command)
		if isExecutable(p) {
			return p
		}
	}
	return ""
}

func isExecutable(p string) bool {
	fi, err := os.Stat(p)
	return err == nil && !fi.IsDir() && fi.Mode().Perm()&0o111 != 0
}
