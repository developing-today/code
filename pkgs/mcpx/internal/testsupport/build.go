// Package testsupport builds the fake MCP server used across mcpx tests.
package testsupport

import (
	"os"
	"os/exec"
	"os/user"
	"path/filepath"
	"strings"
	"sync"
	"testing"
)

var (
	once     sync.Once
	binary   string
	buildErr error

	mcpxOnce sync.Once
	mcpxBin  string
	mcpxErr  error
)

// buildEnv returns the environment for a test-support build. The build
// cache is pinned under the build's own temp dir: a caller (or a guard
// test planting hostile values, see the e2e env guard) must never be
// able to point the build at a path of its choosing — with an ambient
// XDG_CACHE_HOME under a nonexistent root, go build dies with
// "failed to initialize build cache" before any test logic runs.
// GOCACHE is set explicitly as well as XDG_CACHE_HOME so the pin holds
// even where a caller's GOCACHE was already set.
func buildEnv(dir string) []string {
	cache := filepath.Join(dir, ".cache")
	env := make([]string, 0, len(os.Environ())+2)
	for _, kv := range os.Environ() {
		name, _, _ := strings.Cut(kv, "=")
		if name == "GOCACHE" || name == "XDG_CACHE_HOME" || name == "HOME" || isProxyVar(name) {
			continue
		}
		env = append(env, kv)
	}
	env = append(env,
		"XDG_CACHE_HOME="+cache,
		"GOCACHE="+filepath.Join(cache, "go-build"),
	)
	// HOME is reset from the passwd entry, not inherited: a planted
	// HOME (the guard test sets one) would move GOPATH and the module
	// cache under the caller's chosen root, making the build download
	// every module into a temp dir it then cannot clean up. The build
	// uses the invoking user's real home and its warm caches.
	if u, err := user.Current(); err == nil && u.HomeDir != "" {
		env = append(env, "HOME="+u.HomeDir)
	} else if home, herr := os.UserHomeDir(); herr == nil {
		env = append(env, "HOME="+home)
	}
	return env
}

// isProxyVar reports whether name is a proxy variable in either case.
// A build inherits no proxy: the same reasoning as ChildEnv — a proxy
// planted by a caller (or a guard test) between the build and the
// module proxy turns a hermetic build into a network failure.
func isProxyVar(name string) bool {
	switch strings.ToUpper(name) {
	case "HTTP_PROXY", "HTTPS_PROXY", "ALL_PROXY", "NO_PROXY":
		return true
	}
	return false
}

// FakeMCPBinary compiles the fake MCP server once per test binary and returns
// its path.
func FakeMCPBinary(t *testing.T) string {
	t.Helper()
	once.Do(func() {
		dir, err := os.MkdirTemp("", "mcpx-fake-")
		if err != nil {
			buildErr = err
			return
		}
		registerBuildTempDir(dir)
		binary = filepath.Join(dir, "fakemcp")
		cmd := exec.Command("go", "build", "-o", binary, "./internal/testsupport/fakemcp")
		cmd.Dir = repoRoot()
		cmd.Env = buildEnv(dir)
		out, err := cmd.CombinedOutput()
		if err != nil {
			buildErr = &buildFailure{err: err, out: string(out)}
		}
	})
	if buildErr != nil {
		t.Fatalf("build fakemcp: %v", buildErr)
	}
	return binary
}

// MCPXBinary compiles the mcpx command once per test binary.
//
// Every end-to-end test needs the real binary, and building it per test meant
// roughly a hundred `go build` invocations in one package. Go runs package
// test binaries concurrently, so under a full `go test ./...` that load was
// enough to push start timeouts over the edge -- producing a different
// spurious failure on each run, which is the signature that sends someone
// hunting a logic bug that is not there.
func MCPXBinary(t *testing.T) string {
	t.Helper()
	mcpxOnce.Do(func() {
		dir, err := os.MkdirTemp("", "mcpx-bin-")
		if err != nil {
			mcpxErr = err
			return
		}
		registerBuildTempDir(dir)
		mcpxBin = filepath.Join(dir, "mcpx")
		cmd := exec.Command("go", "build", "-o", mcpxBin, "./cmd/mcpx")
		cmd.Dir = repoRoot()
		cmd.Env = buildEnv(dir)
		out, err := cmd.CombinedOutput()
		if err != nil {
			mcpxErr = &buildFailure{err: err, out: string(out)}
		}
	})
	if mcpxErr != nil {
		t.Fatalf("build mcpx: %v", mcpxErr)
	}
	return mcpxBin
}

type buildFailure struct {
	err error
	out string
}

func (b *buildFailure) Error() string { return b.err.Error() + "\n" + b.out }

// repoRoot walks up from the working directory to the module root.
func repoRoot() string {
	dir, err := os.Getwd()
	if err != nil {
		return "."
	}
	for {
		if _, err := os.Stat(filepath.Join(dir, "go.mod")); err == nil {
			return dir
		}
		parent := filepath.Dir(dir)
		if parent == dir {
			return "."
		}
		dir = parent
	}
}
