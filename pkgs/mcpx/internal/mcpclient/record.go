package mcpclient

import (
	"context"
	"encoding/json"
	"fmt"
	"sync"
)

// CallTraceFormat is the version string stamped on every recorded call
// trace. A trace is an artifact meant to outlive the run that produced
// it — a failure captured in the field today must still load next
// release — so the format is versioned from the first one, and
// UnmarshalCallTrace refuses a version it does not recognise rather
// than guessing at its meaning.
const CallTraceFormat = "mcpx-call-trace/v1"

// TraceDir is the direction one recorded frame travelled.
type TraceDir string

const (
	// TraceSend is a frame the client wrote to the server.
	TraceSend TraceDir = "send"
	// TraceRecv is a frame the client read from the server.
	TraceRecv TraceDir = "recv"
)

// CallFrame is one JSON-RPC message in a CallTrace, kept raw. Raw is
// the point: replay compares and re-serves the frame's exact meaning,
// and any normalisation at capture time would throw away evidence the
// trace exists to keep.
type CallFrame struct {
	Dir TraceDir        `json:"dir"`
	Msg json.RawMessage `json:"msg"`
}

// CallTrace is one recorded client session: every frame, in the order
// the transport carried it. It is the family-2 artifact of #8242 —
// record-and-replay of tool calls. A captured scenario replays
// against a fresh client (see testsupport.ReplayTransport), so a
// failure seen once becomes a deterministic regression test and a
// forensic artifact.
//
// A trace carries tool arguments and results verbatim, which can
// include anything a caller sent. Handle a trace file with the same
// care as a log file until a redacting writer lands.
type CallTrace struct {
	Format string      `json:"format"`
	Frames []CallFrame `json:"frames"`
}

// Marshal renders the trace as JSON, stamped with the current format
// version.
func (t *CallTrace) Marshal() ([]byte, error) {
	out := &CallTrace{Format: CallTraceFormat, Frames: t.Frames}
	return json.Marshal(out)
}

// UnmarshalCallTrace parses a trace and validates it strictly: the
// format version must be one this build knows, every frame must name
// a direction, and every frame body must be valid JSON. A trace that
// fails any of these is not partially usable — replaying half a
// scenario proves nothing — so the whole load fails.
func UnmarshalCallTrace(data []byte) (*CallTrace, error) {
	var t CallTrace
	if err := json.Unmarshal(data, &t); err != nil {
		return nil, fmt.Errorf("call trace: %w", err)
	}
	if t.Format != CallTraceFormat {
		return nil, fmt.Errorf("call trace: format %q, this build reads %q", t.Format, CallTraceFormat)
	}
	if len(t.Frames) == 0 {
		return nil, fmt.Errorf("call trace: no frames")
	}
	for i, f := range t.Frames {
		if f.Dir != TraceSend && f.Dir != TraceRecv {
			return nil, fmt.Errorf("call trace: frame %d has direction %q", i, f.Dir)
		}
		if !json.Valid(f.Msg) {
			return nil, fmt.Errorf("call trace: frame %d is not valid JSON", i)
		}
	}
	return &t, nil
}

// RecordTransport wraps a Transport and records every frame it
// carries, in order, as a CallTrace. It is the recorder at the
// transport seam: because it sits below the Client, it captures the
// session exactly as the server saw it — handshake, notifications,
// server-initiated requests and all — with no cooperation from the
// code under test.
//
// Only frames the inner transport accepted are recorded: a Send that
// fails never reached the server, and a Recv that fails delivered
// nothing, so neither belongs in the scenario a replay must
// reproduce.
type RecordTransport struct {
	inner Transport

	mu     sync.Mutex
	frames []CallFrame
}

// NewRecordTransport wraps inner with recording.
func NewRecordTransport(inner Transport) *RecordTransport {
	return &RecordTransport{inner: inner}
}

// Send forwards msg to the inner transport and records it on success.
func (r *RecordTransport) Send(ctx context.Context, msg []byte) error {
	if err := r.inner.Send(ctx, msg); err != nil {
		return err
	}
	r.append(TraceSend, msg)
	return nil
}

// Recv returns the next frame from the inner transport and records
// it on success.
func (r *RecordTransport) Recv() ([]byte, error) {
	msg, err := r.inner.Recv()
	if err != nil {
		return nil, err
	}
	r.append(TraceRecv, msg)
	return msg, nil
}

// Close shuts the inner transport down.
func (r *RecordTransport) Close() error { return r.inner.Close() }

// Info describes the transport, naming both the recorder and what it
// wraps.
func (r *RecordTransport) Info() string { return "record(" + r.inner.Info() + ")" }

func (r *RecordTransport) append(dir TraceDir, msg []byte) {
	cp := append(json.RawMessage(nil), msg...)
	r.mu.Lock()
	r.frames = append(r.frames, CallFrame{Dir: dir, Msg: cp})
	r.mu.Unlock()
}

// Trace returns the frames recorded so far, as a CallTrace stamped
// with the current format version. The result is a copy; recording
// may continue afterwards.
func (r *RecordTransport) Trace() *CallTrace {
	r.mu.Lock()
	defer r.mu.Unlock()
	frames := make([]CallFrame, len(r.frames))
	copy(frames, r.frames)
	return &CallTrace{Format: CallTraceFormat, Frames: frames}
}
