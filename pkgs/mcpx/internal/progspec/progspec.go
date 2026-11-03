// Package progspec resolves a program specification -- the string a
// configuration uses to name a program to run -- to the executable path
// that is actually spawned.
//
// Two shapes exist:
//
//   - A plain spec is a program name or path, resolved exactly as
//     exec.LookPath resolves it today.
//   - A nix spec, "nix:<ref>#<attr>" or "nix:<ref>", names a flake
//     package. It is resolved ONCE, at launch, with
//     `nix build --no-link --print-out-paths`, and the spawned argv uses
//     the resulting store path directly: no nix process sits in the hot
//     path, and a pinned configuration is visibly a store path rather
//     than whatever PATH happens to hold.
//
// Resolution failures are explicit and name the spec: a missing nix
// binary, an unresolvable ref, or a resolution timeout is an error,
// never a silent fallback to looking the name up on PATH (which would
// run a different program than the one configured).
//
// Resolved nix specs are memoised in-process: a daemon that restarts a
// server repeatedly pays the build once. The durable cache and the GC
// roots that keep a store path alive belong to the cache mechanism
// (#1908); this package deliberately does not persist anything.
package progspec

import (
	"context"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"sync"
	"time"
)

// DefaultTimeout bounds one nix resolution. A first build of a large
// flake can legitimately take minutes; a hanging one must not hang the
// launch forever.
const DefaultTimeout = 3 * time.Minute

// Spec is a parsed program specification.
type Spec struct {
	Raw string
	Nix bool
	// Ref and Attr are set for nix specs. Attr may be empty: the ref's
	// default package is meant.
	Ref, Attr string
}

// IsNix reports whether spec is a nix spec.
func IsNix(spec string) bool { return strings.HasPrefix(spec, "nix:") }

// Parse splits a program specification into its parts. A malformed nix
// spec is an error here, not at spawn time, so a configuration can be
// validated before anything launches.
func Parse(raw string) (Spec, error) {
	if !IsNix(raw) {
		if strings.TrimSpace(raw) == "" {
			return Spec{}, fmt.Errorf("progspec: empty program specification")
		}
		return Spec{Raw: raw}, nil
	}
	rest := raw[len("nix:"):]
	ref, attr, hasAttr := strings.Cut(rest, "#")
	if strings.TrimSpace(ref) == "" {
		return Spec{}, fmt.Errorf("progspec: %q names no flake ref", raw)
	}
	if hasAttr && strings.TrimSpace(attr) == "" {
		return Spec{}, fmt.Errorf("progspec: %q names an empty attribute after #", raw)
	}
	return Spec{Raw: raw, Nix: true, Ref: ref, Attr: attr}, nil
}

// Installable is the argument nix build takes for the spec: ref#attr, or
// the bare ref for its default package.
func (s Spec) Installable() string {
	if s.Attr == "" {
		return s.Ref
	}
	return s.Ref + "#" + s.Attr
}

// Resolved is the outcome of resolving a spec.
type Resolved struct {
	Spec string
	// Path is the executable that gets spawned.
	Path string
	// StorePath is the nix store output the executable came from, for
	// nix specs; empty for plain ones. It is the program's identity
	// where one is reported: a pinned config shows a store path, a
	// floating one shows a PATH lookup.
	StorePath string
	Nix       bool
}

var (
	memoMu sync.Mutex
	memo   = map[string]Resolved{}
)

// Resolve turns a program specification into the executable to spawn.
func Resolve(ctx context.Context, raw string) (Resolved, error) {
	return ResolveInDir(ctx, raw, "")
}

// ResolveInDir is Resolve for a caller that runs the program in a
// configured working directory: a nix spec naming a relative flake ref
// (nix:. , nix:./sub) must be built in that directory, not in the
// daemon's, and the memo keys relative refs by the directory they were
// resolved in -- the same relative ref in another directory is a
// different program.
func ResolveInDir(ctx context.Context, raw, dir string) (Resolved, error) {
	spec, err := Parse(raw)
	if err != nil {
		return Resolved{}, err
	}
	if !spec.Nix {
		if strings.Contains(spec.Raw, "/") {
			// A path is used as-is, exactly as exec.Command does.
			return Resolved{Spec: raw, Path: spec.Raw}, nil
		}
		path, err := exec.LookPath(spec.Raw)
		if err != nil {
			return Resolved{}, fmt.Errorf("progspec: %q: %w", raw, err)
		}
		return Resolved{Spec: raw, Path: path}, nil
	}

	key := raw
	if dir != "" {
		key = raw + "\x00dir:" + dir
	}
	memoMu.Lock()
	if r, ok := memo[key]; ok {
		if !isExecutable(r.Path) {
			// The memo outlived its store path: outputs get garbage-
			// collected, and a test stub's output dies with its temp
			// dir. A memoised path that no longer exists is not a
			// resolution -- evict it and resolve again rather than
			// hand the caller a dead executable.
			delete(memo, key)
		} else {
			memoMu.Unlock()
			if err := ctx.Err(); err != nil {
				return Resolved{}, fmt.Errorf("progspec: resolving %q: %w", spec.Raw, err)
			}
			return r, nil
		}
	}
	memoMu.Unlock()

	return resolveNixMemoized(ctx, spec, dir, key)
}

// resolveNixMemoized resolves a nix spec and records the outcome in
// the memo under key.
func resolveNixMemoized(ctx context.Context, spec Spec, dir, key string) (Resolved, error) {
	r, err := resolveNix(ctx, spec, dir)
	if err != nil {
		return Resolved{}, err
	}
	memoMu.Lock()
	memo[key] = r
	memoMu.Unlock()
	return r, nil
}

// ResolveFreshInDir is ResolveInDir without the memo READ: the
// resolution happens now, in this process, with this process's
// environment and the current state of the world. Callers that hand
// the result to ANOTHER process (the supervised spawn, #2585) must
// use this form: a memoized path names whatever an earlier context
// resolved — possibly a build output that has since been cleaned
// up, or one produced under a different PATH — and the receiving
// process cannot tell. The fresh result still updates the memo.
func ResolveFreshInDir(ctx context.Context, raw, dir string) (Resolved, error) {
	spec, err := Parse(raw)
	if err != nil {
		return Resolved{}, err
	}
	if !spec.Nix {
		return ResolveInDir(ctx, raw, dir)
	}
	key := raw
	if dir != "" {
		key = raw + "\x00dir:" + dir
	}
	return resolveNixMemoized(ctx, spec, dir, key)
}

// ResolvePath is Resolve for callers that only need the executable.
func ResolvePath(ctx context.Context, raw string) (string, error) {
	r, err := Resolve(ctx, raw)
	return r.Path, err
}

// ResolvePathInDir is ResolveInDir for callers that only need the
// executable.
func ResolvePathInDir(ctx context.Context, raw, dir string) (string, error) {
	r, err := ResolveInDir(ctx, raw, dir)
	return r.Path, err
}

// ResolvePathFreshInDir is ResolveFreshInDir for callers that only
// need the executable.
func ResolvePathFreshInDir(ctx context.Context, raw, dir string) (string, error) {
	r, err := ResolveFreshInDir(ctx, raw, dir)
	return r.Path, err
}

func resolveNix(ctx context.Context, spec Spec, dir string) (Resolved, error) {
	nix, err := exec.LookPath("nix")
	if err != nil {
		return Resolved{}, fmt.Errorf("progspec: %q requires the nix binary, which is not on PATH: %w", spec.Raw, err)
	}
	if _, ok := ctx.Deadline(); !ok {
		var cancel context.CancelFunc
		ctx, cancel = context.WithTimeout(ctx, DefaultTimeout)
		defer cancel()
	}
	cmd := exec.CommandContext(ctx, nix, "build", "--no-link", "--print-out-paths", spec.Installable())
	if dir != "" {
		// A relative flake ref means the project in the server's
		// configured directory, not the daemon's.
		cmd.Dir = dir
	}
	// A timed-out build's children can hold the output pipes open after
	// the kill; WaitDelay bounds how long Run waits for them to close,
	// so the timeout the caller set is the timeout they get.
	cmd.WaitDelay = 2 * time.Second
	var stdout, stderr strings.Builder
	cmd.Stdout = &stdout
	cmd.Stderr = &stderr
	if err := cmd.Run(); err != nil {
		if ctx.Err() != nil {
			return Resolved{}, fmt.Errorf("progspec: resolving %q: %w", spec.Raw, ctx.Err())
		}
		return Resolved{}, fmt.Errorf("progspec: resolving %q: nix build failed: %v: %s",
			spec.Raw, err, strings.TrimSpace(tail(stderr.String(), 400)))
	}
	out := ""
	for _, line := range strings.Split(stdout.String(), "\n") {
		if line = strings.TrimSpace(line); line != "" {
			out = line
			break
		}
	}
	if out == "" {
		return Resolved{}, fmt.Errorf("progspec: resolving %q: nix build printed no store path", spec.Raw)
	}
	bin, err := programIn(spec, out)
	if err != nil {
		return Resolved{}, err
	}
	return Resolved{Spec: spec.Raw, Path: bin, StorePath: out, Nix: true}, nil
}

// programIn picks the executable inside a built store output. The
// attribute's last segment names it in the common case (hello -> bin/hello);
// when that name is not there (ripgrep -> bin/rg), a lone executable in
// bin/ is unambiguous and is used. Anything else is an explicit error:
// guessing among several programs would run the wrong one silently.
func programIn(spec Spec, out string) (string, error) {
	binDir := filepath.Join(out, "bin")
	if spec.Attr != "" {
		leaf := spec.Attr[strings.LastIndex(spec.Attr, ".")+1:]
		candidate := filepath.Join(binDir, leaf)
		if isExecutable(candidate) {
			return candidate, nil
		}
	}
	entries, err := os.ReadDir(binDir)
	if err != nil {
		return "", fmt.Errorf("progspec: resolving %q: built output %s has no bin directory: %w", spec.Raw, out, err)
	}
	var execs []string
	for _, e := range entries {
		if p := filepath.Join(binDir, e.Name()); isExecutable(p) {
			execs = append(execs, p)
		}
	}
	switch len(execs) {
	case 1:
		return execs[0], nil
	case 0:
		return "", fmt.Errorf("progspec: resolving %q: built output %s has no executable in bin/", spec.Raw, out)
	default:
		return "", fmt.Errorf("progspec: resolving %q: built output %s has %d executables in bin/ and none matches the attribute name; name the attribute that identifies the program", spec.Raw, out, len(execs))
	}
}

func isExecutable(path string) bool {
	fi, err := os.Stat(path)
	return err == nil && !fi.IsDir() && fi.Mode().Perm()&0o111 != 0
}

func tail(s string, n int) string {
	if len(s) <= n {
		return s
	}
	return "..." + s[len(s)-n:]
}
