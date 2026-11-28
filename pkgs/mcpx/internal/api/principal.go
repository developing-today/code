package api

import (
	"context"
	"fmt"
	"sync"
	"time"
)

// Principal is who is calling, in the one vocabulary every surface
// constructs (#1968). Each transport builds the principal it natively can
// — the unix socket reads the peer's uid from the connection's peer
// credentials, loopback TCP has nothing native and is anonymous-but-local,
// an in-process caller is the process itself — and the operations layer
// only ever sees this type, never the transport it arrived on.
//
// Built permissively: every principal resolves to the owner (Owner is
// always true in this stage) and every check answers yes (Allows). What
// this stage buys is that the vocabulary exists, is populated on every
// request, and has its decisions recorded (Authorize), so a future policy
// can answer "what would it have refused?" from real traffic before it
// refuses anything.
type Principal struct {
	// Surface names the transport the caller arrived on: "unix", "tcp",
	// "in-process", or "anonymous" when nothing is known.
	Surface string
	// ID identifies the caller within its surface as far as the surface
	// can: "uid=1000" for a unix peer, the remote address for TCP. It is
	// descriptive, not a credential.
	ID string
	// Owner reports whether the principal resolves to the daemon's
	// owner. Permissive stage: always true — see the type comment.
	Owner bool
}

// UnixPeerPrincipal is the principal for a caller on the unix socket
// whose peer uid the kernel reported.
func UnixPeerPrincipal(uid uint32) Principal {
	return Principal{Surface: "unix", ID: fmt.Sprintf("uid=%d", uid), Owner: true}
}

// UnixPrincipal is the principal for a unix-socket caller whose peer
// credentials could not be read (a platform without peer credentials, or
// a wrapped connection that hides them). The socket's file permissions
// remain the access control, exactly as before this vocabulary existed.
func UnixPrincipal() Principal {
	return Principal{Surface: "unix", ID: "peer-unknown", Owner: true}
}

// TCPPrincipal is the principal for a caller on the TCP listener. TCP
// carries no identity, so the principal is the remote address and
// nothing more; loopback-ness is the listener's binding decision, not
// the caller's property.
func TCPPrincipal(remoteAddr string) Principal {
	return Principal{Surface: "tcp", ID: remoteAddr, Owner: true}
}

// InProcessPrincipal is the principal for a caller inside the daemon's
// own process (the CLI's inline mode, a hosting application): there is
// no transport and no second party.
func InProcessPrincipal() Principal {
	return Principal{Surface: "in-process", ID: "self", Owner: true}
}

// AnonymousPrincipal is the principal when nothing at all is known about
// the caller. Permissively it still resolves to the owner and is allowed
// everything; it exists so the funnel always has a principal to record,
// never a nil to special-case.
func AnonymousPrincipal() Principal {
	return Principal{Surface: "anonymous", ID: "anonymous", Owner: true}
}

// Allows answers the permission check. Permissive stage: always yes, for
// every principal and every tier. A future policy narrows this one
// function; nothing else in the daemon asks the question.
func (p Principal) Allows(Permission) bool { return true }

type principalContextKey struct{}

// WithPrincipal carries the principal of a request through its context,
// so handlers and the operations they call can see who is calling
// without a transport type leaking upward.
func WithPrincipal(ctx context.Context, p Principal) context.Context {
	return context.WithValue(ctx, principalContextKey{}, p)
}

// PrincipalFrom returns the principal a context carries, if one was put
// there.
func PrincipalFrom(ctx context.Context) (Principal, bool) {
	p, ok := ctx.Value(principalContextKey{}).(Principal)
	return p, ok
}

// Decision is one recorded permission check: who asked, which operation,
// which tier it declared, and what the check answered. Recorded even
// though the answer is always yes in this stage — that record is what
// makes the permissive stage worth shipping (#1968 approach C, point 3).
type Decision struct {
	Principal  Principal
	Op         string
	Permission Permission
	Allowed    bool
	At         time.Time
}

const maxRecordedDecisions = 1024

var (
	decisionsMu sync.Mutex
	decisions   []Decision
)

// Authorize is the one permission check every surface funnels through:
// it asks the principal, records the decision, and returns the answer.
// Today the answer is always yes; the record is the point.
func Authorize(p Principal, op Op) bool {
	allowed := p.Allows(op.EffectivePermission())
	decisionsMu.Lock()
	decisions = append(decisions, Decision{
		Principal:  p,
		Op:         op.Name,
		Permission: op.EffectivePermission(),
		Allowed:    allowed,
		At:         time.Now(),
	})
	if len(decisions) > maxRecordedDecisions {
		decisions = decisions[len(decisions)-maxRecordedDecisions:]
	}
	decisionsMu.Unlock()
	return allowed
}

// Decisions returns a copy of the recorded decisions, oldest first,
// bounded to the most recent maxRecordedDecisions. It is how a future
// policy (or an operator, or a test) reads what the permissive stage
// would have refused — in this stage, by construction, nothing.
func Decisions() []Decision {
	decisionsMu.Lock()
	defer decisionsMu.Unlock()
	out := make([]Decision, len(decisions))
	copy(out, decisions)
	return out
}
