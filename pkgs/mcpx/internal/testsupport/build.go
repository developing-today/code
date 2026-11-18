// Package testsupport builds the fake MCP server used across mcpx tests.
package testsupport

import (
	"fmt"
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

	// mcpxCoverDir is the GOCOVERDIR the suite's mcpx children write
	// coverage into, established by MCPXBinary (#1980 stage 3). Empty
	// until the binary is resolved.
	mcpxCoverDir string
)

// buildEnv returns the environment for a test-support build. The build
// cache lives at a fixed location under the real user's cache dir,
// chosen here: a caller (or a guard test planting hostile values, see
// the e2e env guard) must never be able to point the build at a path of
// its choosing — with an ambient XDG_CACHE_HOME under a nonexistent
// root, go build dies with "failed to initialize build cache" before
// any test logic runs. GOCACHE is set explicitly as well as
// XDG_CACHE_HOME so the pin holds even where a caller's GOCACHE was
// already set.
//
// The cache used to be pinned inside each build's own temp dir. That
// made every test binary pay a full cold build of all dependencies
// (~730 MB of cache writes per binary) and made a leaked build dir
// cost the same again in disk. A fixed shared cache stays warm across
// test binaries — Go's build cache is built for concurrent use — and
// a build temp dir that does leak now holds only the ~57 MB binary.
// If the shared cache dir cannot be created (a sandboxed build user
// whose passwd home is not writable), the per-dir pin is the fallback,
// preserving the old behaviour exactly there.
func buildEnv(dir string) []string {
	home := ""
	if u, err := user.Current(); err == nil && u.HomeDir != "" {
		home = u.HomeDir
	} else if h, herr := os.UserHomeDir(); herr == nil {
		home = h
	}
	cache := filepath.Join(dir, ".cache")
	if shared, ok := sharedBuildCacheDir(home); ok {
		cache = shared
	}
	env := make([]string, 0, len(os.Environ())+3)
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
	if home != "" {
		env = append(env, "HOME="+home)
	}
	return env
}

// sharedBuildCacheDir returns the fixed testsupport build cache root
// under home, creating it (and its go-build subdir) if needed. ok is
// false when home is unknown or the dir cannot be created, in which
// case the caller falls back to pinning the cache inside the build's
// own temp dir.
func sharedBuildCacheDir(home string) (string, bool) {
	if home == "" {
		return "", false
	}
	cache := filepath.Join(home, ".cache", "mcpx-testsupport")
	if err := os.MkdirAll(filepath.Join(cache, "go-build"), 0o755); err != nil {
		return "", false
	}
	return cache, true
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

// mcpxBinaryOverrideEnv names the environment variable that points the
// binary suite at a prebuilt mcpx binary instead of a fresh `go build`
// (#1980, approach B): the nix `result/bin/mcpx`, or the binary inside
// the shipped container image. That is the only way the suite exercises
// the artifact users actually install -- the wrapper's PATH pinning,
// the ldflags version, the generated man page and completions -- rather
// than a plain build of the same source.
const mcpxBinaryOverrideEnv = "MCPX_TEST_BINARY"

// resolveMCPXOverride validates a mcpxBinaryOverrideEnv value. An empty
// value means no override: the caller builds as before. A set value must
// name an existing executable file; anything else is a hard error, so a
// typo'd path fails the suite loudly instead of silently falling back
// to a fresh build and testing the wrong artifact.
func resolveMCPXOverride(value string) (string, error) {
	if value == "" {
		return "", nil
	}
	abs, err := filepath.Abs(value)
	if err != nil {
		return "", fmt.Errorf("%s: %w", mcpxBinaryOverrideEnv, err)
	}
	info, err := os.Stat(abs)
	if err != nil {
		return "", fmt.Errorf("%s: %w", mcpxBinaryOverrideEnv, err)
	}
	if info.IsDir() {
		return "", fmt.Errorf("%s: %s is a directory, not a binary", mcpxBinaryOverrideEnv, abs)
	}
	if info.Mode()&0o111 == 0 {
		return "", fmt.Errorf("%s: %s is not executable", mcpxBinaryOverrideEnv, abs)
	}
	return abs, nil
}

// MCPXBinary compiles the mcpx command once per test binary.
//
// Every end-to-end test needs the real binary, and building it per test meant
// roughly a hundred `go build` invocations in one package. When MCPX_TEST_BINARY names a prebuilt binary, that binary is used as-is and nothing is built. Go runs package
// test binaries concurrently, so under a full `go test ./...` that load was
// enough to push start timeouts over the edge -- producing a different
// spurious failure on each run, which is the signature that sends someone
// hunting a logic bug that is not there.
//
// The fresh build is a coverage build (`go build -cover`, #1980 stage 3):
// children launched through ChildEnv carry GOCOVERDIR (see MCPXCoverDir),
// so the e2e suite's executions of the real binary produce coverage that
// `go tool covdata` merges with the unit-test coverage. An override binary
// is used exactly as shipped; the cover dir is still established, so an
// override that was itself built with -cover also emits coverage, and one
// that was not simply ignores GOCOVERDIR.
func MCPXBinary(t *testing.T) string {
	t.Helper()
	mcpxOnce.Do(func() {
		override, err := resolveMCPXOverride(os.Getenv(mcpxBinaryOverrideEnv))
		if err != nil {
			mcpxErr = err
			return
		}
		dir, err := os.MkdirTemp("", "mcpx-bin-")
		if err != nil {
			mcpxErr = err
			return
		}
		registerBuildTempDir(dir)
		mcpxCoverDir = filepath.Join(dir, "cover")
		if err := os.MkdirAll(mcpxCoverDir, 0o755); err != nil {
			mcpxErr = err
			return
		}
		if override != "" {
			mcpxBin = override
			return
		}
		mcpxBin = filepath.Join(dir, "mcpx")
		cmd := exec.Command("go", "build", "-cover", "-o", mcpxBin, "./cmd/mcpx")
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

// MCPXCoverDir returns the directory the suite's mcpx children write
// coverage data into (GOCOVERDIR), or "" before MCPXBinary has resolved
// the binary. ChildEnv puts it into every test child's environment once
// it exists; the daemon a child starts inherits it in turn, so daemon
// executions of the cover-built binary land in the same dir.
func MCPXCoverDir() string {
	return mcpxCoverDir
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
