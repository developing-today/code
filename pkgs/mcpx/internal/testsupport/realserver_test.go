package testsupport

import (
	"os"
	"path/filepath"
	"testing"
)

func writeExecutable(t *testing.T, path string) {
	t.Helper()
	if err := os.MkdirAll(filepath.Dir(path), 0o755); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(path, []byte("#!/bin/sh\n"), 0o755); err != nil {
		t.Fatal(err)
	}
}

func TestRealServerPathBuildEnvLayout(t *testing.T) {
	dir := t.TempDir()
	writeExecutable(t, filepath.Join(dir, "bin", "mcp-server-time"))
	got, ok := realServerPath(dir, "mcp-server-time")
	if !ok || got != filepath.Join(dir, "bin", "mcp-server-time") {
		t.Fatalf("buildEnv layout: got %q, %v", got, ok)
	}
}

func TestRealServerPathFlatLayout(t *testing.T) {
	dir := t.TempDir()
	writeExecutable(t, filepath.Join(dir, "mcp-server-memory"))
	got, ok := realServerPath(dir, "mcp-server-memory")
	if !ok || got != filepath.Join(dir, "mcp-server-memory") {
		t.Fatalf("flat layout: got %q, %v", got, ok)
	}
}

func TestRealServerPathMissingAndNotExecutable(t *testing.T) {
	dir := t.TempDir()
	if _, ok := realServerPath(dir, "mcp-server-time"); ok {
		t.Fatal("missing server reported found")
	}
	plain := filepath.Join(dir, "bin", "mcp-server-time")
	writeExecutable(t, plain)
	if err := os.Chmod(plain, 0o644); err != nil {
		t.Fatal(err)
	}
	if _, ok := realServerPath(dir, "mcp-server-time"); ok {
		t.Fatal("non-executable file reported as a server")
	}
}

func TestRealRequired(t *testing.T) {
	for _, tc := range []struct {
		list, name string
		want       bool
	}{
		{"", "mcp-server-time", false},
		{"deno,bun", "mcp-server-time", false},
		{"mcp-server-time", "mcp-server-time", true},
		{"deno, mcp-server-time ,bun", "mcp-server-time", true},
		{"realsoftware", "mcp-server-memory", true},
		{"real", "mcp-server-git", true},
		{"*", "mcp-server-fetch", true},
	} {
		if got := realRequired(tc.list, tc.name); got != tc.want {
			t.Errorf("realRequired(%q, %q) = %v, want %v", tc.list, tc.name, got, tc.want)
		}
	}
}
