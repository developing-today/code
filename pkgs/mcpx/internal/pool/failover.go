package pool

// Daemon call-path wiring for identity failover (#8017, follow-on to
// #7948).
//
// Until now the mcpauth Rotator was consulted only at connection
// start (resolveAuth): a GitHub soft block that arrived mid-session,
// as an error tools/call result, surfaced straight to the calling
// session. Call now runs failover-configured servers through
// Rotator.Do: the result is translated to an *mcpauth.APIError
// (mcpauth.APIErrorFromToolResult / APIErrorFromText), a soft block
// rotates the identity, the instance serving this key is respawned
// so the next attempt starts on the rotated identity's credential,
// and the call is retried -- the session sees only the eventual
// success. A hard block (or any non-block outcome) settles the call
// exactly as it would have without failover, byte for byte.

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"net/http"
	"time"

	"github.com/developing-today/code/pkgs/mcpx/internal/config"
	"github.com/developing-today/code/pkgs/mcpx/internal/mcpauth"
)

// errCallSettled is the sentinel Rotator.Do's fn returns when the
// call has reached its final outcome -- success, a hard block, or an
// unrelated failure -- and no rotation should happen. Classify reads
// it as a hard block, so Do returns it at once; Call unwraps it back
// into the captured outcome.
var errCallSettled = errors.New("pool: failover call settled without rotation")

// failoverRotator builds the rotator, secret lookup and minter for
// this pool's failover list, or reports false when the server has
// none. It is the one construction site: resolveAuth (connection
// start) and Call (call path) share it, so both read and persist the
// same rotation state at the list's StatePath. Rotations are logged
// through the pool lifecycle log with both identity names, per
// Rotator.Do's log contract.
func (p *Pool) failoverRotator() (*mcpauth.Rotator, mcpauth.SecretFunc, *mcpauth.Minter, bool) {
	if p.cfg.Auth == nil || p.cfg.Auth.Failover == nil || len(p.cfg.Auth.Failover.Identities) == 0 {
		return nil, nil, nil, false
	}
	fo := p.cfg.Auth.Failover
	secrets := func(name string) (string, error) {
		if v := p.variable(name); v != "" {
			return v, nil
		}
		return "", fmt.Errorf("secret %s not set in the daemon's environment", name)
	}
	minter := &mcpauth.Minter{HTTP: &http.Client{Timeout: 15 * time.Second}}
	rot := mcpauth.NewRotator(fo, func(format string, args ...any) {
		lifecycle("mcpauth.failover", map[string]any{
			"server":  p.cfg.Name,
			"message": fmt.Sprintf(format, args...),
		})
	})
	return rot, secrets, minter, true
}

// failoverRespawn makes the next acquisition for key spawn a fresh
// instance when the serving identity has changed since this pool
// last respawned for failover. Rotator.Do persists the rotated
// Current before it invokes the retry, and resolveAuth resolves that
// same persisted Current at spawn, so dropping the idle instance is
// the whole respawn: the replacement starts on the rotated
// identity's env. The first identity a pool serves is recorded
// without a drop -- instances spawned before any rotation already
// carry it, having resolved it at their own connection start. When
// the instance is busy serving another caller ReleaseKey leaves it
// alone and the retry reuses it; the rotation still lands for the
// next fresh spawn.
func (p *Pool) failoverRespawn(key, identity string) {
	p.foMu.Lock()
	if p.foIdentity == "" || p.foIdentity == identity {
		p.foIdentity = identity
		p.foMu.Unlock()
		return
	}
	p.foIdentity = identity
	p.foMu.Unlock()
	p.ReleaseKey(key)
}

// Call runs a tool on a leased instance. For a server with an auth
// failover list it wraps callOnce in Rotator.Do as described at the
// top of this file; every other server takes callOnce directly,
// unchanged.
func (p *Pool) Call(ctx context.Context, rs config.ResolvedScope, tool string, args any) (json.RawMessage, error) {
	rot, secrets, minter, ok := p.failoverRotator()
	if !ok {
		return p.callOnce(ctx, rs, tool, args)
	}
	var settled json.RawMessage
	var settledErr error
	err := rot.Do(ctx, secrets, minter, func(id mcpauth.Identity, _ string) error {
		// The token Do resolved for id is the one resolveAuth hands
		// a fresh spawn for the same persisted Current, so the fn
		// does not thread it anywhere: it respawns if rotation moved
		// the identity since the serving instance started, then
		// makes the call on the instance that results.
		p.failoverRespawn(rs.Key, id.Name)
		res, cerr := p.callOnce(ctx, rs, tool, args)
		if cerr != nil {
			if ae, ok := mcpauth.APIErrorFromText(cerr.Error()); ok && mcpauth.Classify(ae) == mcpauth.BlockSoft {
				return ae
			}
			settled, settledErr = res, cerr
			return errCallSettled
		}
		if ae, ok := mcpauth.APIErrorFromToolResult(res); ok && mcpauth.Classify(ae) == mcpauth.BlockSoft {
			return ae
		}
		settled, settledErr = res, nil
		return errCallSettled
	})
	if errors.Is(err, errCallSettled) {
		return settled, settledErr
	}
	// Do's own error: every identity soft-blocked or in cooldown, or
	// a credential that could not be produced. That is the honest
	// outcome to surface -- the last soft block wrapped with the
	// failover context -- rather than replaying one identity's raw
	// block as if no failover existed.
	return nil, err
}
