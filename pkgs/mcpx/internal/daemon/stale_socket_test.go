package daemon

import (
	"io"
	"log"
	"net"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

// Tests for clearStaleSocket (#5742). Oracle: the filesystem state
// after the call -- a non-socket entry must be byte-for-byte
// preserved with a startup error, a stale socket must be gone with
// no error, and a live socket must be untouched with the
// already-listening error.

func staleSocketTestServer(t *testing.T, socket string) *Server {
	t.Helper()
	return &Server{
		set:    gapTestSettings(t),
		paths:  Paths{Socket: socket},
		logger: log.New(io.Discard, "", 0),
	}
}

func TestClearStaleSocketPreservesRegularFile(t *testing.T) {
	sock := filepath.Join(t.TempDir(), "daemon.sock")
	contents := []byte("not a socket -- user data")
	if err := os.WriteFile(sock, contents, 0o600); err != nil {
		t.Fatal(err)
	}
	s := staleSocketTestServer(t, sock)
	if err := s.clearStaleSocket(); err == nil {
		t.Fatal("expected a startup error for a regular file at the socket path")
	} else if !strings.Contains(err.Error(), sock) {
		t.Fatalf("error should name the colliding path, got: %v", err)
	}
	got, err := os.ReadFile(sock)
	if err != nil {
		t.Fatalf("regular file was removed: %v", err)
	}
	if string(got) != string(contents) {
		t.Fatalf("regular file contents changed: %q", got)
	}
}

func TestClearStaleSocketPreservesSymlink(t *testing.T) {
	dir := t.TempDir()
	target := filepath.Join(dir, "target.txt")
	if err := os.WriteFile(target, []byte("target data"), 0o600); err != nil {
		t.Fatal(err)
	}
	sock := filepath.Join(dir, "daemon.sock")
	if err := os.Symlink(target, sock); err != nil {
		t.Fatal(err)
	}
	s := staleSocketTestServer(t, sock)
	if err := s.clearStaleSocket(); err == nil {
		t.Fatal("expected a startup error for a symlink at the socket path")
	}
	if _, err := os.Lstat(sock); err != nil {
		t.Fatalf("symlink was removed: %v", err)
	}
	got, err := os.ReadFile(target)
	if err != nil || string(got) != "target data" {
		t.Fatalf("symlink target changed: %q, %v", got, err)
	}
}

func TestClearStaleSocketPreservesSymlinkToStaleSocket(t *testing.T) {
	// The permutation that pins Lstat over Stat (#5742, Codex review):
	// with Stat, a symlink to a stale socket reports ModeSocket, the
	// dial fails, and cleanup removes the symlink. The symlink must
	// be preserved as a symlink, and its target left alone.
	dir := t.TempDir()
	target := filepath.Join(dir, "stale.sock")
	ln, err := net.Listen("unix", target)
	if err != nil {
		t.Fatal(err)
	}
	ln.(*net.UnixListener).SetUnlinkOnClose(false)
	if err := ln.Close(); err != nil {
		t.Fatal(err)
	}
	sock := filepath.Join(dir, "daemon.sock")
	if err := os.Symlink(target, sock); err != nil {
		t.Fatal(err)
	}
	s := staleSocketTestServer(t, sock)
	if err := s.clearStaleSocket(); err == nil {
		t.Fatal("expected a startup error for a symlink at the socket path")
	}
	fi, err := os.Lstat(sock)
	if err != nil {
		t.Fatalf("symlink was removed: %v", err)
	}
	if fi.Mode()&os.ModeSymlink == 0 {
		t.Fatalf("entry at the socket path is no longer a symlink: %v", fi.Mode())
	}
	if _, err := os.Lstat(target); err != nil {
		t.Fatalf("symlink target (stale socket) was removed: %v", err)
	}
}

func TestClearStaleSocketRemovesStaleSocket(t *testing.T) {
	sock := filepath.Join(t.TempDir(), "daemon.sock")
	ln, err := net.Listen("unix", sock)
	if err != nil {
		t.Fatal(err)
	}
	// Leave the socket file behind, as a crashed daemon does.
	ln.(*net.UnixListener).SetUnlinkOnClose(false)
	if err := ln.Close(); err != nil {
		t.Fatal(err)
	}
	s := staleSocketTestServer(t, sock)
	if err := s.clearStaleSocket(); err != nil {
		t.Fatalf("stale socket should be cleaned up, got: %v", err)
	}
	if _, err := os.Lstat(sock); !os.IsNotExist(err) {
		t.Fatalf("stale socket file still present (err=%v)", err)
	}
}

func TestClearStaleSocketProtectsActiveSocket(t *testing.T) {
	sock := filepath.Join(t.TempDir(), "daemon.sock")
	ln, err := net.Listen("unix", sock)
	if err != nil {
		t.Fatal(err)
	}
	defer func() { _ = ln.Close() }()
	go func() {
		for {
			c, err := ln.Accept()
			if err != nil {
				return
			}
			_ = c.Close()
		}
	}()
	s := staleSocketTestServer(t, sock)
	if err := s.clearStaleSocket(); err == nil {
		t.Fatal("expected the already-listening error for an active socket")
	} else if !strings.Contains(err.Error(), "already listening") {
		t.Fatalf("expected already-listening error, got: %v", err)
	}
	if _, err := os.Lstat(sock); err != nil {
		t.Fatalf("active socket was removed: %v", err)
	}
}

func TestClearStaleSocketAbsentPath(t *testing.T) {
	s := staleSocketTestServer(t, filepath.Join(t.TempDir(), "daemon.sock"))
	if err := s.clearStaleSocket(); err != nil {
		t.Fatalf("absent socket path should be a no-op, got: %v", err)
	}
}
