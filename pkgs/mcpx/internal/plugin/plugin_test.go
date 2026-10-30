package plugin

import (
	"os"
	"path/filepath"
	"strings"
	"testing"
)

func writeFile(t *testing.T, path, content string, perm os.FileMode) {
	t.Helper()
	if err := os.MkdirAll(filepath.Dir(path), 0o755); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(path, []byte(content), perm); err != nil {
		t.Fatal(err)
	}
}

func TestDiscoverManifestPlugin(t *testing.T) {
	root := t.TempDir()
	writeFile(t, filepath.Join(root, "weather", ManifestFile), `{
		// a comment, because manifests are mcpx documents too
		"name": "weather",
		"description": "forecasts as tools",
		"namespace": "wx",
		"command": "./weather-server",
		"args": ["--units", "metric"],
		"requires": {"api": "v1", "capabilities": ["tools"]},
		"hooks": ["catalog.changed"]
	}`, 0o644)
	writeFile(t, filepath.Join(root, "weather", "weather-server"), "#!/bin/sh\n", 0o755)

	got, err := Discover([]string{root}, "")
	if err != nil {
		t.Fatal(err)
	}
	if len(got) != 1 {
		t.Fatalf("want 1 plugin, got %d: %+v", len(got), got)
	}
	p := got[0]
	if p.Name != "weather" || p.ZeroManifest {
		t.Fatalf("unexpected plugin: %+v", p)
	}
	wantCmd := filepath.Join(root, "weather", "weather-server")
	if p.Command != wantCmd {
		t.Fatalf("relative command not resolved against manifest dir: got %q want %q", p.Command, wantCmd)
	}
	if p.Requires == nil || p.Requires.API != "v1" || len(p.Hooks) != 1 {
		t.Fatalf("requires/hooks not parsed: %+v", p)
	}

	s := p.Server("/tmp/daemon.sock", "sess-1")
	if s.Name != "weather" || s.Command != wantCmd {
		t.Fatalf("server conversion lost identity: %+v", s)
	}
	if s.Mcpx == nil || s.Mcpx.Namespace != "wx" || s.Mcpx.Description != "forecasts as tools" {
		t.Fatalf("server extras lost namespace/description: %+v", s.Mcpx)
	}
	if s.Env["MCPX_SOCKET"] != "/tmp/daemon.sock" || s.Env["MCPX_SESSION_ID"] != "sess-1" {
		t.Fatalf("back-channel env missing: %v", s.Env)
	}
}

func TestDiscoverManifestAtDirectoryRoot(t *testing.T) {
	root := t.TempDir()
	writeFile(t, filepath.Join(root, ManifestFile), `{"name":"solo","command":"/bin/echo"}`, 0o644)
	// The plugin's own helper executable is not a second plugin.
	writeFile(t, filepath.Join(root, "helper"), "#!/bin/sh\n", 0o755)

	got, err := Discover([]string{root}, "")
	if err != nil {
		t.Fatal(err)
	}
	if len(got) != 1 || got[0].Name != "solo" {
		t.Fatalf("want only the manifest plugin, got %+v", got)
	}
}

func TestDiscoverZeroManifestInDirectory(t *testing.T) {
	root := t.TempDir()
	writeFile(t, filepath.Join(root, "echoer"), "#!/bin/sh\ncat\n", 0o755)
	writeFile(t, filepath.Join(root, "notes.txt"), "not a plugin\n", 0o644)

	got, err := Discover([]string{root}, "")
	if err != nil {
		t.Fatal(err)
	}
	if len(got) != 1 {
		t.Fatalf("want 1 zero-manifest plugin, got %+v", got)
	}
	p := got[0]
	if !p.ZeroManifest || p.Name != "echoer" || p.Command != filepath.Join(root, "echoer") {
		t.Fatalf("unexpected zero-manifest plugin: %+v", p)
	}
}

func TestDiscoverZeroManifestOnPATH(t *testing.T) {
	bin := t.TempDir()
	writeFile(t, filepath.Join(bin, "mcpx-plugin-jokes"), "#!/bin/sh\n", 0o755)
	writeFile(t, filepath.Join(bin, "mcpx-plugin-notexec"), "#!/bin/sh\n", 0o644)
	writeFile(t, filepath.Join(bin, "unrelated"), "#!/bin/sh\n", 0o755)

	got, err := Discover(nil, bin)
	if err != nil {
		t.Fatal(err)
	}
	if len(got) != 1 || got[0].Name != "jokes" || !got[0].ZeroManifest {
		t.Fatalf("want only mcpx-plugin-jokes, got %+v", got)
	}
}

func TestDiscoverPATHShadowedByDirectoryPlugin(t *testing.T) {
	root := t.TempDir()
	writeFile(t, filepath.Join(root, "dup", ManifestFile), `{"name":"dup","command":"/bin/echo"}`, 0o644)
	bin := t.TempDir()
	writeFile(t, filepath.Join(bin, "mcpx-plugin-dup"), "#!/bin/sh\n", 0o755)

	got, err := Discover([]string{root}, bin)
	if err != nil {
		t.Fatal(err)
	}
	if len(got) != 1 || got[0].ZeroManifest {
		t.Fatalf("directory plugin should win over PATH namesake, got %+v", got)
	}
}

func TestDiscoverDuplicateNamesAreAnError(t *testing.T) {
	a, b := t.TempDir(), t.TempDir()
	for _, d := range []string{a, b} {
		writeFile(t, filepath.Join(d, "same", ManifestFile), `{"name":"same","command":"/bin/echo"}`, 0o644)
	}
	_, err := Discover([]string{a, b}, "")
	if err == nil || !strings.Contains(err.Error(), "two plugins named") {
		t.Fatalf("want duplicate-name error, got %v", err)
	}
}

func TestDiscoverMissingNamedPathIsAnError(t *testing.T) {
	_, err := Discover([]string{filepath.Join(t.TempDir(), "nope")}, "")
	if err == nil {
		t.Fatal("want error for a named paths.plugins entry that does not exist")
	}
}

func TestDiscoverRemoteManifest(t *testing.T) {
	root := t.TempDir()
	writeFile(t, filepath.Join(root, "remote", ManifestFile),
		`{"name":"remote","url":"https://example.invalid/mcp","transport":"http"}`, 0o644)
	got, err := Discover([]string{root}, "")
	if err != nil {
		t.Fatal(err)
	}
	if len(got) != 1 {
		t.Fatalf("want 1 plugin, got %+v", got)
	}
	s := got[0].Server("", "")
	if s.URL != "https://example.invalid/mcp" || s.Transport != "http" || s.Command != "" {
		t.Fatalf("remote plugin not converted: %+v", s)
	}
	if s.Cwd != "" {
		t.Fatalf("remote plugin must not get a local cwd, got %q", s.Cwd)
	}
}

func TestManifestValidation(t *testing.T) {
	for _, m := range []Manifest{
		{Name: "", Command: "/bin/echo"},
		{Name: "x"},
		{Name: "x", Command: "/bin/echo", URL: "https://example.invalid"},
	} {
		if err := m.Validate(); err == nil {
			t.Fatalf("want validation error for %+v", m)
		}
	}
	if err := (Manifest{Name: "x", Command: "/bin/echo"}).Validate(); err != nil {
		t.Fatalf("valid manifest rejected: %v", err)
	}
}

func TestServerEnvManifestValuesKeptBackChannelWins(t *testing.T) {
	p := Plugin{Manifest: Manifest{
		Name:    "envy",
		Command: "/bin/echo",
		Env:     map[string]string{"MCPX_SOCKET": "/wrong.sock", "OWN": "1"},
	}}
	s := p.Server("/right.sock", "sess")
	if s.Env["MCPX_SOCKET"] != "/right.sock" || s.Env["OWN"] != "1" {
		t.Fatalf("env merge wrong: %v", s.Env)
	}
}
