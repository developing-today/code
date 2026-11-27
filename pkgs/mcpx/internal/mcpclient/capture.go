package mcpclient

// Capture is the opt-in live-client half of #8242's family 2
// (record-and-replay of tool calls). A dialed client records nothing
// by default; setting Options.TraceCapture wraps the transport in a
// RecordTransport before the handshake, so the session is captured
// exactly as the server saw it — handshake, notifications,
// server-initiated requests and all — with no cooperation from the
// code under test and no change to the normal transport behavior when
// the flag is off.
//
// A recorded trace carries tool arguments and results verbatim, which
// can include authorization material and other secrets. Redaction
// happens before persistence, not after: persist
// Client.RedactedTrace (or CallTrace.MarshalRedacted), never the raw
// recorder output.

// wrapCapture wraps t in a RecordTransport when capture is enabled,
// returning the transport to dial over and the recorder to keep. When
// capture is off both halves are t and nil: the transport passes
// through untouched.
func wrapCapture(t Transport, capture bool) (Transport, *RecordTransport) {
	if !capture {
		return t, nil
	}
	rec := NewRecordTransport(t)
	return rec, rec
}

// RedactedTrace returns the session recorded so far with
// credential-bearing content elided, or nil when this client was not
// dialed with Options.TraceCapture. The result is a redacted copy:
// recording continues afterwards, and the raw (unredacted) trace is
// never exposed here, so a caller holding only the client cannot
// persist secrets by accident.
func (c *Client) RedactedTrace() (*CallTrace, error) {
	rec := c.recorder
	if rec == nil {
		return nil, nil
	}
	return rec.Trace().Redacted()
}

// bareTransport unwraps the capture recorder, if any, so transport
// type assertions below keep meaning what they always meant: capture
// is a recording layer around the real transport, not a different
// kind of session.
func (c *Client) bareTransport() Transport {
	if c.recorder != nil {
		return c.recorder.Inner()
	}
	return c.t
}
