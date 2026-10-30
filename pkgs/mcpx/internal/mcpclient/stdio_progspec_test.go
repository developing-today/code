package mcpclient_test

import (
	"context"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"github.com/dezren39/mcpx/internal/mcpclient"
)

// stubNixForSpawn installs a fake nix binary on PATH whose build prints
// a fake store output holding one executable shell script.
func stubNixForSpawn(t *testing.T) {
	t.Helper()
	dir := t.TempDir()
	bin := filepath.Join(dir, "store", "aaaa-fakepkg-1.0", "bin")
	if err := os.MkdirAll(bin, 0o755); err != nil {
		t.Fatal(err)
	}
	tool := filepath.Join(bin, "faketool")
	if err := os.WriteFile(tool, []byte("#!/bin/sh\nexec cat\n"), 0o755); err != nil {
		t.Fatal(err)
	}
	out := filepath.Dir(bin)
	stub := "#!/bin/sh\n" +
		"for a in \"$@\"; do case \"$a\" in *slow*) sleep 30;; esac; done\n" +
		"echo \"" + out + "\"\n"
	if err := os.WriteFile(filepath.Join(dir, "nix"), []byte(stub), 0o755); err != nil {
		t.Fatal(err)
	}
	t.Setenv("PATH", dir+string(os.PathListSeparator)+os.Getenv("PATH"))
}

func TestNewStdioResolvesNixSpecAtSpawn(t *testing.T) {
	stubNixForSpawn(t)
	tr, err := mcpclient.NewStdio(mcpclient.StdioOptions{Command: "nix:nixpkgs#faketool", InheritEnv: true})
	if err != nil {
		t.Fatalf("NewStdio with a nix: spec: %v", err)
	}
	_ = tr.Close()
}

func TestNewStdioNixResolutionHonoursTheStartupContext(t *testing.T) {
	stubNixForSpawn(t)
	ctx, cancel := context.WithTimeout(context.Background(), 300*time.Millisecond)
	defer cancel()
	_, err := mcpclient.NewStdio(mcpclient.StdioOptions{
		Command: "nix:slowref#tool", InheritEnv: true, Context: ctx,
	})
	if err == nil || !strings.Contains(err.Error(), "nix:slowref#tool") {
		t.Fatalf("err = %v, want a context-bound resolution error naming the spec", err)
	}
}

func TestSupervisedResolvesNixSpecForItsChild(t *testing.T) {
	stubNixForSpawn(t)
	tr, err := mcpclient.NewSupervised(mcpclient.SupervisedOptions{
		Name: "nixecho", StateDir: t.TempDir(),
		Command: "nix:nixpkgs#echotool", InheritEnv: true,
	})
	if err != nil {
		t.Fatalf("NewSupervised with a nix: spec: %v", err)
	}
	defer killChild(t, tr)
	defer tr.Close()
	ctx := context.Background()
	frame := []byte(`{"jsonrpc":"2.0","id":1,"method":"ping"}`)
	if err := tr.Send(ctx, frame); err != nil {
		t.Fatalf("send: %v", err)
	}
	got, err := tr.Recv()
	if err != nil {
		t.Fatalf("recv: %v", err)
	}
	if string(got) != string(frame) {
		t.Fatalf("echo = %s, want %s", got, frame)
	}
}
