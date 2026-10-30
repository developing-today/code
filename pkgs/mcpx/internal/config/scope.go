package config

import (
	"fmt"
	"os"
	"path/filepath"
	"strconv"
	"strings"
	"sync"
)

// CallContext is what a caller knows about itself. The daemon resolves a
// server's Scope against this to decide which process the call belongs to.
//
// Cwd, PID and CallID are always available. SessionID and ParentSessionID are
// not discoverable by mcpx -- a subagent and its parent share a working
// directory and differ only in an identifier their host assigns -- so the
// caller supplies them, through MCPX_SESSION_ID / MCPX_PARENT_SESSION_ID or
// an explicit --session.
type CallContext struct {
	Cwd             string `json:"cwd,omitempty"`
	SessionID       string `json:"sessionId,omitempty"`
	ParentSessionID string `json:"parentSessionId,omitempty"`
	PID             int    `json:"pid,omitempty"`
	// CallID is unique per invocation and is the fallback whenever a more
	// specific identifier is missing, so an absent session degrades to
	// per-call isolation rather than to accidental sharing.
	CallID string `json:"callId,omitempty"`
	// Ephemeral marks a SessionID the caller minted for itself rather than
	// one its host assigned. Such a key can only ever have one user, so it is
	// safe to stop the instance the moment that caller exits instead of
	// waiting out an idle timer.
	Ephemeral bool `json:"ephemeral,omitempty"`
	// PIDShared marks a PID that names a process shared by many callers
	// rather than the caller's own: the mcpx daemon's own PID, which
	// every /v1/exec run it serves reports (the script env carries the
	// daemon's os.Getpid()). A pid: key resolved from it is a shared
	// instance key, never caller-private, however ephemeral the run is.
	PIDShared bool `json:"pidShared,omitempty"`
}

// CallerOwned reports whether a resolved key belongs solely to this caller and
// may be torn down when the caller finishes: a per-call key, or one built from
// an identity the caller minted for itself.
//
// The pid: case has the same trap in daemon clothing: an ephemeral
// /v1/exec run reports the *daemon's* PID, which every overlapping run
// shares, so PIDShared keeps that key out of caller ownership -- releasing
// it at one run's end closed an instance another run still needed.
//
// Ephemeral says the *session id* is private, not that everything the caller
// touched is. Treating it as the latter stopped the shared global, repo and
// worktree instances at the end of every `mcpx exec` run without a session,
// so the next caller paid a cold start -- and a caller that had just been
// using one lost it.
func (c CallContext) CallerOwned(key string) bool {
	if strings.HasPrefix(key, "call:") {
		return true
	}
	if !c.Ephemeral {
		return false
	}
	return (c.SessionID != "" && key == "session:"+c.SessionID) ||
		(c.PID > 0 && !c.PIDShared && key == "pid:"+strconv.Itoa(c.PID))
}

// Key resolves the instance key for a scope. The returned key is opaque; only
// equality matters. Degraded reports whether a more specific scope fell back
// to something weaker, which the daemon logs once so the cause is visible.
func (s Scope) Key(ctx CallContext) (key string, degraded string) {
	switch s {
	case ScopeGlobal:
		return "global", ""

	case ScopeCwd:
		if ctx.Cwd == "" {
			return fallback(ctx), "cwd is unknown"
		}
		return "cwd:" + keyPath(ctx.Cwd), ""

	case ScopeRepo, ScopeWorktree:
		if ctx.Cwd == "" {
			return fallback(ctx), "cwd is unknown"
		}
		// A directory outside any repository -- or one whose repository
		// cannot be read -- keys by itself: narrower than the scope asked
		// for, never wider.
		w, _, err := locateGit(ctx.Cwd)
		switch {
		case err != nil:
			return "cwd:" + keyPath(ctx.Cwd), err.Error()
		case s == ScopeRepo:
			return "repo:" + keyPath(w.commonDir), ""
		case w.worktree != "":
			return "worktree:" + keyPath(w.worktree), ""
		default:
			return "cwd:" + keyPath(ctx.Cwd), "no worktree: " + w.noWorktree
		}

	case ScopeSession:
		if ctx.SessionID != "" {
			return "session:" + ctx.SessionID, ""
		}
		return fallback(ctx), "no session id; set MCPX_SESSION_ID or pass --session"

	case ScopeParentSession:
		if ctx.ParentSessionID != "" {
			return "psession:" + ctx.ParentSessionID, ""
		}
		if ctx.SessionID != "" {
			return "session:" + ctx.SessionID, "no parent session id; using the session id"
		}
		return fallback(ctx), "no parent or session id; set MCPX_PARENT_SESSION_ID"

	case ScopePid:
		if ctx.PID > 0 {
			return "pid:" + strconv.Itoa(ctx.PID), ""
		}
		return fallback(ctx), "caller pid is unknown"

	case ScopeCall:
		return fallback(ctx), ""
	}
	return fallback(ctx), fmt.Sprintf("unknown scope %q", s)
}

// fallback is per-call isolation: the safe direction to fail, since sharing a
// stateful process by accident corrupts results while over-isolating only
// costs a process.
func fallback(ctx CallContext) string {
	if ctx.CallID != "" {
		return "call:" + ctx.CallID
	}
	return "call:anonymous"
}

// WatchesPID reports whether this scope ties a process's lifetime to a caller.
func (s Scope) WatchesPID() bool { return s == ScopePid }

// PIDOf extracts the watched pid from a key produced by ScopePid.
func PIDOf(key string) (int, bool) {
	rest, ok := strings.CutPrefix(key, "pid:")
	if !ok {
		return 0, false
	}
	n, err := strconv.Atoi(rest)
	return n, err == nil
}

// canonical resolves a path for use as a key. Two callers naming one directory
// differently -- /tmp and /private/tmp on macOS, or any symlinked checkout --
// must land on the same process, so every path that becomes a key goes through
// here. The input is returned unchanged when it cannot be resolved, which is
// the right failure: an unresolvable path is still consistently itself.
func canonical(p string) string {
	if p == "" {
		return p
	}
	if !filepath.IsAbs(p) {
		if wd, err := os.Getwd(); err == nil {
			p = filepath.Join(wd, p)
		}
	}
	if resolved, err := filepath.EvalSymlinks(p); err == nil {
		return resolved
	}
	return filepath.Clean(p)
}

// keyPath is canonical plus directory identity, for use as a scope key
// (#1912). canonical keeps the caller's spelling, so on a case-insensitive
// volume (APFS by default, SMB/vfat mounts) /Users/x/Repo and /users/x/repo
// -- one directory -- keyed separately and spawned a second instance. The
// error was always over-isolation, never accidental sharing.
//
// The fix keys on identity, not spelling: stat the canonical path and look
// its (device, inode) up in a side table that remembers the first spelling
// seen for that identity; every spelling of the directory keys to the
// first. The key stays a plain readable path, which is what `mcpx daemons`
// and the degraded-scope log print. Two genuinely different directories
// that differ only by case have different inodes, so a case-sensitive
// volume can never merge them -- the unsafe direction is impossible.
//
// When the stat fails (a deleted or unreadable cwd) or the platform has
// no identity to give, the canonical path alone is the key: today's
// behaviour, and still the safe direction. Keys are computed by the
// daemon and held in memory for its lifetime, never persisted, so an
// identity that changes across a filesystem rebuild needs no migration;
// the table holds one entry per distinct directory actually scoped.
var identitySpellings = struct {
	sync.Mutex
	byID map[[2]uint64]string
}{byID: map[[2]uint64]string{}}

func keyPath(p string) string {
	c := canonical(p)
	dev, ino, ok := dirIdentity(c)
	if !ok {
		return c
	}
	id := [2]uint64{dev, ino}
	identitySpellings.Lock()
	defer identitySpellings.Unlock()
	if first, seen := identitySpellings.byID[id]; seen && first != c {
		// Reuse is only sound while the remembered spelling still names
		// this identity. An inode is recycled when its directory is
		// deleted, and the recycled number must not resurrect the dead
		// directory's spelling as a live directory's key.
		if fdev, fino, fok := dirIdentity(first); !fok || fdev != dev || fino != ino {
			identitySpellings.byID[id] = c
			return c
		}
		return first
	}
	identitySpellings.byID[id] = c
	return c
}
