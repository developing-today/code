package cli

import (
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/developing-today/code/pkgs/mcpx/internal/config"
	"github.com/developing-today/code/pkgs/mcpx/internal/daemon"
	"github.com/developing-today/code/pkgs/mcpx/internal/testsupport"
)

// The advertised form puts flags after the name: mcpx adapter add demo
// --tool status -- git. Hoisting must keep each --tool with its value, or
// the parser reads demo as the tool and status as the name (Codex review,
// PR #2565).
func TestHoistKeepsToolFlagsPairedWithTheirValues(t *testing.T) {
	in := []string{"demo", "--tool", "status", "--tool", "log", "--runtime", "bash"}
	got := hoistFlags(in, adapterAddValuedFlags)
	want := []string{"--tool", "status", "--tool", "log", "--runtime", "bash", "demo"}
	if strings.Join(got, " ") != strings.Join(want, " ") {
		t.Fatalf("hoisted %v, want %v", got, want)
	}
}

// A generated tool's fixed arguments select its subcommand: without them
// the tool runs the bare command (Codex review, PR #2565).
func TestAdapterToolEntriesCarryTheSubcommandAsArgs(t *testing.T) {
	tools := adapterToolEntries([]string{"status", "log"})
	if len(tools) != 2 {
		t.Fatalf("entries %v", tools)
	}
	for i, name := range []string{"status", "log"} {
		if tools[i]["name"] != name {
			t.Fatalf("tool %d name %v, want %s", i, tools[i]["name"], name)
		}
		args, ok := tools[i]["args"].([]string)
		if !ok || len(args) != 1 || args[0] != name {
			t.Fatalf("tool %d args %v, want [%s]", i, tools[i]["args"], name)
		}
	}
}

// adapterApp builds an App whose paths.adapters names file, in a scratch
// home so no real declaration can leak into the test.
func adapterApp(t *testing.T, adaptersFile string) *App {
	t.Helper()
	root := testsupport.TempDir(t)
	t.Setenv("HOME", root)
	t.Setenv("XDG_CONFIG_HOME", filepath.Join(root, "xdg"))
	t.Setenv("MCPX_PATHS_ADAPTERS", adaptersFile)
	cfg := filepath.Join(root, "mcpx.json")
	if err := os.WriteFile(cfg, []byte(`{"mcpServers":{}}`), 0o600); err != nil {
		t.Fatal(err)
	}
	return &App{ConfigPath: cfg, Paths: daemon.PathsAt(root, "")}
}

// Replacing a declaration must change the generated server's identity, or
// the reload keeps the child that captured the old declaration at startup
// (Codex review, PR #2565).
func TestAugmentAdaptersFingerprintMovesWithTheDeclaration(t *testing.T) {
	dir := t.TempDir()
	file := filepath.Join(dir, "adapters.json")
	write := func(tools string) {
		t.Helper()
		decl := `{"adapters":[{"name":"git","command":"git","tools":[` + tools + `]}]}`
		if err := os.WriteFile(file, []byte(decl), 0o644); err != nil {
			t.Fatal(err)
		}
	}
	fingerprint := func() string {
		t.Helper()
		a := adapterApp(t, file)
		cfg := &config.Config{MCPServers: map[string]*config.Server{}}
		if err := a.augmentAdapters(cfg); err != nil {
			t.Fatal(err)
		}
		s, ok := cfg.MCPServers["git"]
		if !ok {
			t.Fatalf("adapter not added as a server: %v", cfg.MCPServers)
		}
		fp := s.Env["MCPX_ADAPTER_DECLARATION_SHA256"]
		if fp == "" {
			t.Fatal("generated server carries no declaration fingerprint")
		}
		return fp
	}
	write(`{"name":"status"}`)
	before := fingerprint()
	write(`{"name":"status"},{"name":"log"}`)
	if after := fingerprint(); after == before {
		t.Fatalf("fingerprint did not move when the declaration changed: %s", before)
	}
}
