// The test child environment: an allowlist, not an inheritance.
//
// Every end-to-end test starts a real mcpx binary as a child process. That
// child used to inherit the developer's whole environment minus MCPX_*
// variables, so anything else the caller's shell exported -- XDG_* paths
// into the user's real config, OPENCODE_* session scoping from the opencode
// plugin, proxies, NODE_OPTIONS -- reached the test, and the suite's result
// depended on whose machine ran it (#1992). The concrete caller-daemon hole
// (MCPX_DAEMON_ENDPOINT/MCPX_SOCKET) is closed by dropping MCPX_*; this file
// closes the rest by failing open in the other direction: a child gets only
// what is listed here, plus what the harness sets explicitly for that test.
//
// A new caller variable class is isolated by default. Widening the allowlist
// is a deliberate edit here, held to the guard in internal/e2e.
package testsupport

import (
	"path/filepath"
	"strings"
)

// ChildEnv builds the environment for a test child process from an
// allowlist (#1992, Approach C).
//
// parent is the caller's environment, usually os.Environ. dir is the test's
// own directory, usually testsupport.TempDir: the XDG base directories are
// pinned underneath it so no caller XDG_* path, and no real $HOME reached
// through them, is visible to the child. extra is what the test controls --
// MCPX_STATE_DIR, HOME, the stub registry and the like -- and wins, because
// os/exec keeps the last value for a duplicated key.
//
// The parent contributes only:
//   - PATH, so the child finds its runtimes (deno, bun, node) and sh;
//   - TMPDIR (TEMP/TMP), LANG, LANGUAGE, LC_* and TERM, so locale-aware
//     output and the temp directory behave as on the caller's machine;
//   - SYSTEMROOT, so the same builder stays correct if the suite ever runs
//     on Windows (#136);
//   - the runtime-cache passthroughs DENO_DIR, BUN_INSTALL_CACHE_DIR and
//     NPM_CONFIG_CACHE, so a test reuses the caller's warm caches instead
//     of paying for a cold download per test.
//
// Everything else is dropped, in particular: every MCPX_* variable (a
// parent value would run the suite under the caller's settings, or point it
// at the caller's daemon); every XDG_* variable (replaced below); every
// OPENCODE_* variable (outer session scoping must not leak into the test's);
// proxy variables in either case (a proxy between the test and its own stub
// registry is a failure that only happens on some machines); NODE_OPTIONS
// (a caller's --require flag would load into every script the test runs);
// and TLS, credential and identity variables, which the isolated suite has
// no business reading.
func ChildEnv(parent []string, dir string, extra ...string) []string {
	out := make([]string, 0, len(parent)+len(extra)+5)
	for _, kv := range parent {
		name, _, ok := strings.Cut(kv, "=")
		if !ok {
			continue
		}
		if !inheritedFromParent(name) {
			continue
		}
		out = append(out, kv)
	}
	// XDG base directories under the test's own dir. The child resolves
	// state, cache, config and the runtime dir from these (or from the
	// MCPX_* overrides in extra), so nothing under the caller's real home
	// or real runtime dir is shared with the test.
	out = append(out,
		"XDG_CONFIG_HOME="+filepath.Join(dir, ".config"),
		"XDG_DATA_HOME="+filepath.Join(dir, ".local", "share"),
		"XDG_STATE_HOME="+filepath.Join(dir, ".local", "state"),
		"XDG_CACHE_HOME="+filepath.Join(dir, ".cache"),
		"XDG_RUNTIME_DIR="+filepath.Join(dir, "run"),
	)
	out = append(out, extra...)
	return out
}

// inheritedFromParent reports whether a parent variable passes the allowlist
// into a test child. Anything not named here is dropped, whatever its
// prefix; in particular no MCPX_* variable is ever inherited.
func inheritedFromParent(name string) bool {
	switch name {
	case "PATH", "TMPDIR", "TEMP", "TMP",
		"LANG", "LANGUAGE", "TERM",
		"DENO_DIR", "BUN_INSTALL_CACHE_DIR", "NPM_CONFIG_CACHE",
		"SYSTEMROOT":
		return true
	}
	return strings.HasPrefix(name, "LC_")
}
