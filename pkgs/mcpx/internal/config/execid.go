package config

import (
	"crypto/sha256"
	"encoding/hex"
	"fmt"
	"io"
	"os"
	"path/filepath"
	"strings"

	"github.com/dezren39/mcpx/internal/defaults"
)

// ExecIdentity says which file a stdio server's command runs, as of now.
//
// PoolID describes the configuration; it cannot see a binary that changed
// underneath an unchanged command. `npx`, `uvx` and a nix profile that
// re-points `~/.nix-profile/bin/foo` all keep the same command, args and env
// while the program behind them moves, so a pool keyed by PoolID alone kept
// running the old program until somebody restarted the daemon.
//
// The identity is the resolved real path together with what that file holds:
// its content hash when it is small, and its device, inode, size and mtime
// when it is not. The path matters on its own, because a re-pointed symlink
// can reach a byte-identical file. The stat fallback keeps a large program,
// such as a node binary, from being read whole on every reload. An in-place
// rewrite changes size or mtime in practice; a replacement changes the inode.
//
// The identity is "" for a server that is not a child process, and begins
// with "missing:" when the command does not resolve. A missing command is
// its own identity, so the pool still starts and fails the way it always did,
// and it is replaced when the command appears.
func (r *Resolved) ExecIdentity() string {
	if !r.Stdio() {
		return ""
	}
	path := r.resolveExec()
	if path == "" {
		return "missing:" + r.Command
	}
	real, err := filepath.EvalSymlinks(path)
	if err != nil {
		return "missing:" + path
	}
	fi, err := os.Stat(real)
	if err != nil {
		return "missing:" + real
	}
	h := sha256.New()
	fmt.Fprintf(h, "%s\x00", real)
	if fi.Mode().IsRegular() && fi.Size() <= defaults.ExecHashMax {
		f, err := os.Open(real)
		if err != nil {
			return "missing:" + real
		}
		_, err = io.Copy(h, f)
		f.Close()
		if err != nil {
			return "missing:" + real
		}
	} else {
		// Not Sys(): its atime moves every time the program runs, which
		// would restart the pool on each reload.
		fmt.Fprintf(h, "%s\x00%d\x00%d\x00%d", fileInode(fi), fi.Size(), fi.ModTime().UnixNano(), fi.Mode())
	}
	return real + "|" + hex.EncodeToString(h.Sum(nil))[:16]
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
	if strings.Contains(r.Command, string(filepath.Separator)) {
		p := r.Command
		if !filepath.IsAbs(p) && r.Cwd != "" {
			p = filepath.Join(r.Cwd, p)
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
