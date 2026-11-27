package mcpclient

import (
	"context"
	"encoding/json"
	"fmt"
	"sync"

	"github.com/developing-today/code/pkgs/mcpx/internal/logging"
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
// include anything a caller sent. Handle a raw trace with the same
// care as a log file: persist the redacted form (Redacted /
// MarshalRedacted), which elides credential-bearing content with the
// same shapes the log redactor uses, and keep the raw trace out of
// artifacts.
type CallTrace struct {
	Format string      `json:"format"`
	Frames []CallFrame `json:"frames"`
}

// Marshal renders the trace as JSON, stamped with the current format
// version. This method preserves frame content verbatim and is meant
// for replay/test fixtures, not persistence of a live trace: use
// MarshalRedacted at persistence boundaries.
func (t *CallTrace) Marshal() ([]byte, error) {
	return t.marshalFrames(t.Frames)
}

// MarshalRedacted renders a replayable copy of the trace after
// redacting sensitive content in every JSON string value. It is the
// persistence-safe writer for live capture. The oracle is that the
// output remains valid mcpx-call-trace/v1 and credentials recognized
// by logging.Redact are absent from serialized bytes.
func (t *CallTrace) MarshalRedacted() ([]byte, error) {
	redacted, err := t.Redacted()
	if err != nil {
		return nil, err
	}
	return redacted.Marshal()
}

// Redacted returns a deep, replayable copy with credential-bearing
// string values elided using the same matcher as structured logs.
func (t *CallTrace) Redacted() (*CallTrace, error) {
	frames := make([]CallFrame, len(t.Frames))
	for i, frame := range t.Frames {
		var value any
		if err := json.Unmarshal(frame.Msg, &value); err != nil {
			return nil, fmt.Errorf("call trace: frame %d: %w", i, err)
		}
		frames[i] = CallFrame{Dir: frame.Dir, Msg: redactJSON(value)}
	}
	return &CallTrace{Format: CallTraceFormat, Frames: frames}, nil
}

func redactJSON(value any) json.RawMessage {
	switch v := value.(type) {
	case map[string]any:
		out := make(map[string]any, len(v))
		for k, child := range v {
			out[k] = redactJSONValue(k, child)
		}
		b, _ := json.Marshal(out)
		return b
	default:
		b, _ := json.Marshal(redactJSONValue("", v))
		return b
	}
}

func redactJSONValue(key string, value any) any {
	switch v := value.(type) {
	case string:
		return logging.Redact(key, v)
	case map[string]any:
		out := make(map[string]any, len(v))
		for k, child := range v {
			out[k] = redactJSONValue(k, child)
		}
		return out
	case []any:
		out := make([]any, len(v))
		for i, child := range v {
			out[i] = redactJSONValue(key, child)
		}
		return out
	default:
		return value
	}
}

func (t *CallTrace) marshalFrames(frames []CallFrame) ([]byte, error) {
	out := &CallTrace{Format: CallTraceFormat, Frames: frames}
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

// Inner returns the transport wrapped by the recorder. The transport
// itself is unchanged and is exposed so capture remains transparent to
// existing transport-specific lifecycle paths (for example stdio
// handoff and HTTP session inspection).
func (r *RecordTransport) Inner() Transport { return r.inner }

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
