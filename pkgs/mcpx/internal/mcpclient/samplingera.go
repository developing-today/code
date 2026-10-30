package mcpclient

import (
	"context"
	"errors"
)

// SamplingEra is the protocol generation a server's sampling request arrived
// under.
//
// Both sampling paths converge on Client.answer -- a legacy server's request
// through handleServerRequest, a 2026-07-28 one through resolveInput -- so
// that is where it is stamped, and the relay that answers the question reads
// it back to know which message shapes the upstream is allowed to have sent
// and to receive.
type SamplingEra struct {
	// Negotiated is the version actually in use; "" when the connection has
	// not settled one.
	Negotiated string
	// Era is which protocol generation the connection settled on.
	Era Era
}

type samplingEraKey struct{}

// WithSamplingEra carries the era a sampling request arrived under to the
// handler that relays it, the WithClientCapabilities precedent.
func WithSamplingEra(ctx context.Context, se SamplingEra) context.Context {
	return context.WithValue(ctx, samplingEraKey{}, se)
}

// SamplingEraFrom is what WithSamplingEra stored.
//
// ok is false for a handler called outside a connection's answer path, such
// as a test calling the relay directly; the validator then accepts the
// permissive union rather than guessing an era.
func SamplingEraFrom(ctx context.Context) (SamplingEra, bool) {
	if ctx == nil {
		return SamplingEra{}, false
	}
	se, ok := ctx.Value(samplingEraKey{}).(SamplingEra)
	return se, ok
}

// modernEra is the era as the atomic mirror sees it, safe to read while a
// concurrent recovery may be writing Era.
func (c *Client) modernEra() Era {
	if c.modern.Load() {
		return EraModern
	}
	return EraLegacy
}

// negotiatedVersion is the negotiated version as the atomic mirror sees
// it, safe to read while a concurrent recovery may be writing Negotiated.
// Every path that settles a version publishes the mirror (finishLegacy,
// probe, Resume), so a client with no mirror has settled none.
func (c *Client) negotiatedVersion() string {
	v, _ := c.negotiated.Load().(string)
	return v
}

// rpcErrorFor is a handler error as the JSON-RPC error the server hears.
//
// A handler that knows the code its failure stands for says so with an
// RPCCode method -- sampling validation is -32602, InvalidParamsError the
// precedent -- and anything else stays the internal error it was. Flattening
// every handler failure to -32603 told a server its request was fine when it
// was not.
func rpcErrorFor(err error) *rpcError {
	var coded interface{ RPCCode() int }
	if errors.As(err, &coded) {
		return &rpcError{Code: coded.RPCCode(), Message: err.Error()}
	}
	return &rpcError{Code: -32603, Message: err.Error()}
}
