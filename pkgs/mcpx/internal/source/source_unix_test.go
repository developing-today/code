//go:build !windows

package source_test

import (
	"context"
	"errors"
	"net"
	"os"
	"os/exec"
	"path/filepath"
	"syscall"
	"testing"
	"time"

	"github.com/developing-today/code/pkgs/mcpx/internal/source"
)

func TestDirectorySkipsFIFO(t *testing.T) {
	if dir := os.Getenv("MCPX_SOURCE_FIFO_TEST_DIR"); dir != "" {
		got, err := source.Resolve("@dir:"+dir, opts(dir))
		if err != nil {
			t.Fatal(err)
		}
		if len(got.Files) != 1 || filepath.Base(got.Files[0]) != "regular.ts" {
			t.Fatalf("FIFO should be skipped while regular files are retained, got %v", got.Files)
		}
		return
	}

	dir := t.TempDir()
	if err := os.WriteFile(filepath.Join(dir, "regular.ts"), []byte("regular"), 0o644); err != nil {
		t.Fatal(err)
	}
	if err := syscall.Mkfifo(filepath.Join(dir, "fifo.ts"), 0o600); err != nil {
		t.Skipf("cannot create FIFO: %v", err)
	}

	ctx, cancel := context.WithTimeout(context.Background(), 3*time.Second)
	defer cancel()
	cmd := exec.CommandContext(ctx, os.Args[0], "-test.run=^TestDirectorySkipsFIFO$")
	cmd.Env = append(os.Environ(), "MCPX_SOURCE_FIFO_TEST_DIR="+dir)
	output, err := cmd.CombinedOutput()
	if ctx.Err() == context.DeadlineExceeded {
		t.Fatalf("resolving a directory containing a FIFO did not finish within 3s")
	}
	if err != nil {
		t.Fatalf("FIFO helper failed: %v\n%s", err, output)
	}
}

func TestDirectorySkipsUnixSocket(t *testing.T) {
	dir := t.TempDir()
	regular := filepath.Join(dir, "regular.ts")
	if err := os.WriteFile(regular, []byte("regular"), 0o644); err != nil {
		t.Fatal(err)
	}
	listener, err := net.Listen("unix", filepath.Join(dir, "socket.ts"))
	if err != nil {
		t.Skipf("cannot create Unix socket: %v", err)
	}
	defer listener.Close()

	got, err := source.Resolve("@dir:"+dir, opts(dir))
	if err != nil {
		t.Fatal(err)
	}
	if len(got.Files) != 1 || got.Files[0] != regular {
		t.Fatalf("socket should be skipped while regular files are retained, got %v", got.Files)
	}
}

func TestDirectoryReportsInaccessibleSymlinkTarget(t *testing.T) {
	if os.Geteuid() == 0 {
		t.Skip("root can traverse directories without execute permission")
	}

	root := t.TempDir()
	private := t.TempDir()
	if err := os.WriteFile(filepath.Join(private, "secret.ts"), []byte("secret"), 0o644); err != nil {
		t.Fatal(err)
	}
	if err := os.Symlink(filepath.Join(private, "secret.ts"), filepath.Join(root, "inaccessible.ts")); err != nil {
		t.Skipf("symlinks are unavailable: %v", err)
	}
	if err := os.Chmod(private, 0); err != nil {
		t.Skipf("cannot make target directory inaccessible: %v", err)
	}
	defer os.Chmod(private, 0o700)

	_, err := source.Resolve("@dir:"+root, opts(root))
	if !errors.Is(err, os.ErrPermission) {
		t.Fatalf("inaccessible symlink target should return a permission error, got %v", err)
	}
}
